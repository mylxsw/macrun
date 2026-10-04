//! Menu-bar icon drawing, kept free of Tauri types so it can be unit tested anywhere.
//! Calm states are monochrome template images that macOS tints for light and
//! dark menu bars. Only states that need the person (an approval, or the agent
//! using the desktop) are drawn in colour, as a filled capsule.
use serde_json::Value;

pub const WIDTH: u32 = 52;
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
            Self::Idle => "空闲",
            Self::Working => "工作中",
            Self::Approval => "待确认",
            Self::Desktop => "正在操作桌面",
            Self::Offline => "断线",
            Self::Paused => "暂停",
        }
    }
    /// Template images must be pure alpha masks; coloured states are not templates.
    pub fn template(self) -> bool {
        !matches!(self, Self::Approval | Self::Desktop)
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

fn rounded_rect(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).pow(2) + (y - cy).pow(2) <= r * r
}

/// RGBA pixels for a state, `WIDTH`×`HEIGHT` (a 26×22 pt icon at 2×).
pub fn pixels(state: TrayState) -> Vec<u8> {
    let mut rgba = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let fill = match state {
        TrayState::Approval => Some([208, 138, 11]),
        TrayState::Desktop => Some([240, 90, 26]),
        _ => None,
    };
    let alpha = if state == TrayState::Offline {
        115
    } else {
        255
    };
    for y in 0..HEIGHT as i32 {
        for x in 0..WIDTH as i32 {
            // Screen frame: outline in template states, filled capsule when coloured.
            let outer = rounded_rect(x, y, 3, 7, 48, 36, 7);
            let inner = rounded_rect(x, y, 6, 10, 45, 33, 4);
            let frame = outer && !inner;
            let chevron =
                (13..=20).contains(&x) && ((y - x - 1).abs() <= 1 || (y + x - 41).abs() <= 1);
            let underscore = (24..=34).contains(&x) && (26..=28).contains(&y);
            let pause =
                ((17..=20).contains(&x) || (30..=33).contains(&x)) && (15..=28).contains(&y);
            let slash =
                state == TrayState::Offline && (x + y - 46).abs() <= 1 && (2..=50).contains(&x);
            let badge_ring = (x - 46).pow(2) + (y - 8).pow(2) <= 8 * 8;
            let badge = (x - 46).pow(2) + (y - 8).pow(2) <= 5 * 5;
            let glyph = if state == TrayState::Paused {
                pause
            } else {
                chevron || underscore
            };
            let i = ((y * WIDTH as i32 + x) * 4) as usize;
            let paint = |rgba: &mut [u8], c: [u8; 3], a: u8| {
                rgba[i..i + 3].copy_from_slice(&c);
                rgba[i + 3] = a;
            };
            match fill {
                Some(color) => {
                    if glyph {
                        paint(&mut rgba, [255, 255, 255], 255);
                    } else if outer {
                        paint(&mut rgba, color, 255);
                    }
                }
                None => {
                    let working = state == TrayState::Working;
                    if working && badge {
                        paint(&mut rgba, [0, 0, 0], 255);
                    } else if working && badge_ring {
                        // Keep a transparent gap between the badge and the frame.
                    } else if frame || glyph || slash {
                        paint(&mut rgba, [0, 0, 0], alpha);
                    }
                }
            }
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
    fn attention_states_are_coloured_capsules() {
        for (state, colour) in [
            (TrayState::Approval, [208, 138, 11]),
            (TrayState::Desktop, [240, 90, 26]),
        ] {
            let p = pixels(state);
            assert!(!state.template());
            assert_eq!(colours(&p), {
                let mut c = vec![colour, [255, 255, 255]];
                c.sort();
                c
            });
            // The capsule interior is filled, unlike the template outline.
            assert_eq!(alpha_at(&p, 40, 20), 255);
        }
        assert_eq!(alpha_at(&pixels(TrayState::Idle), 40, 20), 0);
    }

    #[test]
    fn shapes_distinguish_states_without_colour() {
        let idle = pixels(TrayState::Idle);
        let working = pixels(TrayState::Working);
        let paused = pixels(TrayState::Paused);
        let offline = pixels(TrayState::Offline);
        assert_eq!(alpha_at(&idle, 46, 8), 0);
        assert_eq!(alpha_at(&working, 46, 8), 255);
        assert_ne!(idle, paused);
        // Offline is dimmed and crossed out.
        assert_eq!(alpha_at(&offline, 23, 23), 115);
        assert!(offline.chunks(4).all(|c| c[3] == 0 || c[3] == 115));
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
