//! Menu-bar icon drawing, kept free of Tauri types so it can be unit tested anywhere.
//! Calm states are monochrome template images that macOS tints for light and
//! dark menu bars. Only states that need the person (an approval, or the agent
//! using the desktop) are drawn in colour, with a coloured link.
use crate::localization::text as tr;
use serde_json::Value;

pub const WIDTH: u32 = 44;
pub const HEIGHT: u32 = 44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Working,
    Approval,
    Desktop,
    Offline,
    Paused,
}

impl TrayState {
    pub fn from_snapshot(v: &Value) -> Self {
        let tasks = v["tasks"].as_array();
        let active = |t: &&Value| matches!(t["status"].as_str(), Some("accepted" | "running"));
        if v["connection"]["state"] != "connected" {
            Self::Offline
        } else if tasks.is_some_and(|t| t.iter().any(|t| t["status"] == "awaiting_approval")) {
            Self::Approval
        } else if tasks.is_some_and(|t| t.iter().filter(active).any(|t| t["kind"] == "mcp.call")) {
            Self::Desktop
        } else if v["policy"]["paused"] == true {
            Self::Paused
        } else if v["active_count"].as_u64().unwrap_or(0) > 0 {
            Self::Working
        } else {
            Self::Idle
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => tr("空闲"),
            Self::Working => tr("工作中"),
            Self::Approval => tr("待确认"),
            Self::Desktop => tr("正在操作桌面"),
            Self::Offline => tr("断线"),
            Self::Paused => tr("暂停"),
        }
    }
    /// Template images must be pure alpha masks; coloured states are not templates.
    pub fn template(self) -> bool {
        !matches!(self, Self::Approval | Self::Desktop)
    }
}

/// Only these values change the native menu-bar presentation. Heartbeats,
/// latency and task output must not cause image replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub state: TrayState,
    pub approvals: usize,
}
impl Presentation {
    pub fn from_snapshot(v: &Value) -> Self {
        Self {
            state: TrayState::from_snapshot(v),
            approvals: approval_count(v),
        }
    }
}

/// Number of approvals waiting, shown as menu-bar text beside the icon.
pub fn approval_count(v: &Value) -> usize {
    v["tasks"].as_array().map_or(0, |t| {
        t.iter()
            .filter(|t| t["status"] == "awaiting_approval")
            .count()
    })
}

// The menu bar uses the same linked shape as the application icon. Drawing
// a monochrome mask keeps it crisp and legible under macOS menu-bar tinting.
fn segment_distance(x: f64, y: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let t = (((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
    (x - x0 - t * dx).hypot(y - y0 - t * dy)
}

fn link_distance(x: f64, y: f64) -> f64 {
    // Rotate a pair of open links by -45 degrees around the centre.
    let u = 12. + (x - y) * std::f64::consts::FRAC_1_SQRT_2;
    let v = 12. + (x + y - 24.) * std::f64::consts::FRAC_1_SQRT_2;
    let mut d = segment_distance(u, v, 8., 12., 16., 12.);
    for (x0, x1, y) in [(7., 9., 7.), (7., 9., 17.), (15., 17., 7.), (15., 17., 17.)] {
        d = d.min(segment_distance(u, v, x0, y, x1, y));
    }
    if u <= 7. {
        d = d.min(((u - 7.).hypot(v - 12.) - 5.).abs());
    }
    if u >= 17. {
        d = d.min(((u - 17.).hypot(v - 12.) - 5.).abs());
    }
    d
}

/// Antialiased 22×22 mask. macOS scales it to its native menu-bar height.
pub fn pixels(state: TrayState) -> Vec<u8> {
    let mut rgba = vec![0; (WIDTH * HEIGHT * 4) as usize];
    let colour = match state {
        TrayState::Approval => [208, 138, 11],
        TrayState::Desktop => [240, 90, 26],
        _ => [0, 0, 0],
    };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let mut coverage = 0.;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f64 + (sx as f64 + 0.5) / 4.) * 24. / WIDTH as f64;
                    let py = (y as f64 + (sy as f64 + 0.5) / 4.) * 24. / HEIGHT as f64;
                    let mut painted = link_distance(px, py) <= 1.25;
                    if state == TrayState::Working {
                        let badge = (px - 20.5).hypot(py - 3.5);
                        if badge < 3. {
                            painted = badge <= 1.6;
                        }
                    }
                    if state == TrayState::Paused && (px - 19.).hypot(py - 19.) < 4. {
                        painted = ((17.0..=18.3).contains(&px) || (20.0..=21.3).contains(&px))
                            && (16.0..=22.0).contains(&py);
                    }
                    if state == TrayState::Offline {
                        painted |= segment_distance(px, py, 4., 4., 20., 20.) <= 0.7;
                    }
                    if painted {
                        coverage += 1.;
                    }
                }
            }
            let i = ((y * WIDTH + x) * 4) as usize;
            rgba[i..i + 3].copy_from_slice(&colour);
            rgba[i + 3] = (coverage / 16.
                * if state == TrayState::Offline {
                    115.
                } else {
                    255.
                }) as u8;
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn alpha_at(p: &[u8], x: u32, y: u32) -> u8 {
        p[((y * WIDTH + x) * 4 + 3) as usize]
    }
    fn colours(p: &[u8]) -> Vec<[u8; 3]> {
        let mut seen: Vec<[u8; 3]> = p
            .chunks(4)
            .filter(|c| c[3] > 0)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        seen.sort();
        seen.dedup();
        seen
    }

    #[test]
    fn heartbeat_changes_do_not_redraw_the_tray() {
        let mut snapshot =
            json!({"connection":{"state":"connected","rtt_ms":10},"tasks":[],"active_count":0});
        let idle = Presentation::from_snapshot(&snapshot);
        snapshot["connection"]["rtt_ms"] = json!(900);
        snapshot["timestamp"] = json!(999999);
        snapshot["total_tasks"] = json!(1500);
        assert_eq!(Presentation::from_snapshot(&snapshot), idle);
        snapshot["tasks"] = json!([{"status":"awaiting_approval"}]);
        let one = Presentation::from_snapshot(&snapshot);
        assert_ne!(one.state, idle.state);
        snapshot["tasks"] = json!([{"status":"awaiting_approval"},{"status":"awaiting_approval"}]);
        let two = Presentation::from_snapshot(&snapshot);
        assert_eq!(one.state, two.state);
        assert_ne!(one.approvals, two.approvals);
    }

    #[test]
    fn idle_symbol_has_square_visible_bounds_and_clear_corners() {
        let p = pixels(TrayState::Idle);
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if alpha_at(&p, x, y) > 0 {
                    xs.push(x);
                    ys.push(y);
                }
            }
        }
        assert_eq!(
            xs.iter().max().unwrap() - xs.iter().min().unwrap(),
            ys.iter().max().unwrap() - ys.iter().min().unwrap()
        );
        assert_eq!(alpha_at(&p, 4, 4), 0);
    }

    #[test]
    fn calm_states_are_monochrome_templates() {
        for state in [
            TrayState::Idle,
            TrayState::Working,
            TrayState::Offline,
            TrayState::Paused,
        ] {
            let p = pixels(state);
            assert_eq!(p.len(), (WIDTH * HEIGHT * 4) as usize);
            assert!(state.template());
            assert_eq!(colours(&p), vec![[0, 0, 0]], "{state:?}");
        }
    }

    #[test]
    fn attention_states_use_colour_without_an_opaque_background() {
        for (state, colour) in [
            (TrayState::Approval, [208, 138, 11]),
            (TrayState::Desktop, [240, 90, 26]),
        ] {
            let p = pixels(state);
            assert!(!state.template());
            assert_eq!(colours(&p), vec![colour]);
            assert_eq!(alpha_at(&p, 0, 0), 0);
        }
    }

    #[test]
    fn shapes_distinguish_states_without_colour() {
        let idle = pixels(TrayState::Idle);
        assert_ne!(idle, pixels(TrayState::Working));
        assert_ne!(idle, pixels(TrayState::Paused));
        assert_ne!(idle, pixels(TrayState::Offline));
        assert!(pixels(TrayState::Offline).chunks(4).all(|c| c[3] <= 115));
        assert!(idle.chunks(4).any(|c| c[3] > 0 && c[3] < 255));
    }

    #[test]
    fn snapshot_maps_to_the_most_urgent_state() {
        let base = |extra: Value| {
            let mut v = json!({"connection":{"state":"connected"},"policy":{"paused":false},"active_count":0,"tasks":[]});
            for (k, val) in extra.as_object().unwrap() {
                v[k] = val.clone();
            }
            v
        };
        assert_eq!(TrayState::from_snapshot(&base(json!({}))), TrayState::Idle);
        assert_eq!(TrayState::from_snapshot(&json!({})), TrayState::Offline);
        assert_eq!(
            TrayState::from_snapshot(&base(
                json!({"active_count":1,"tasks":[{"status":"running","kind":"exec.start"}]})
            )),
            TrayState::Working
        );
        assert_eq!(
            TrayState::from_snapshot(&base(
                json!({"active_count":1,"tasks":[{"status":"running","kind":"mcp.call"}]})
            )),
            TrayState::Desktop
        );
        let waiting = base(
            json!({"policy":{"paused":true},"active_count":2,"tasks":[{"status":"running","kind":"mcp.call"},{"status":"awaiting_approval","kind":"exec.start"}]}),
        );
        assert_eq!(TrayState::from_snapshot(&waiting), TrayState::Approval);
        assert_eq!(approval_count(&waiting), 1);
        assert_eq!(
            TrayState::from_snapshot(&base(json!({"policy":{"paused":true}}))),
            TrayState::Paused
        );
        assert_eq!(TrayState::Desktop.label(), "正在操作桌面");
    }
}
