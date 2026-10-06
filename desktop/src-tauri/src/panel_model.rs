//! What the native menu panel shows, derived from a worker snapshot.
//! Kept free of Tauri and AppKit types so it is unit tested on every platform.
//! The step summary and project rules mirror `desktop/src/model.mjs`.
use crate::localization::{interpolate as tr_format, text as tr};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const HELPERS: &[&str] = &[
    "cd", "ls", "cp", "mv", "rm", "mkdir", "touch", "chmod", "ln", "echo", "printf", "grep",
    "egrep", "rg", "tail", "head", "cut", "sed", "awk", "cat", "tr", "sort", "uniq", "wc", "tee",
    "true", "false", "test", "[", "export", "set", "pwd", "sleep", "xargs", "find", "basename",
    "dirname", "date", "which", "stat", "file", "less", "more", "jq",
];
const PREFIXES: &[&str] = &["sudo", "env", "time", "nohup", "exec", "command", "(", "{"];
const GENERIC_PARENTS: &[&str] = &[
    "",
    "tmp",
    "private",
    "var",
    "folders",
    "users",
    "home",
    "work",
    "workspace",
    "workspaces",
    "src",
    "code",
    "codes",
    "projects",
    "repos",
    "dev",
    "documents",
    "desktop",
    "build",
    "mirror",
    "mirrors",
];
const APPROVAL_MS: u64 = 60_000;

/// Splits a shell line on `;`, `&&`, `||`, `|`, `&` and newlines outside quotes.
pub fn shell_segments(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        if let Some(q) = quote {
            current.push(c);
            if c == '\\' && q == '"' && next.is_some() {
                current.push(chars[i + 1]);
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '\\' && next.is_some() {
            current.push(c);
            current.push(chars[i + 1]);
            i += 1;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            current.push(c);
        } else if c == ';'
            || c == '\n'
            || c == '|'
            || (c == '&' && prev != Some('>') && next != Some('>'))
        {
            if (c == '|' || c == '&') && next == Some(c) {
                i += 1;
            }
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
        i += 1;
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

fn word(w: Option<&&str>) -> bool {
    w.is_some_and(|w| {
        let mut chars = w.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ":_-".contains(c))
    })
}

fn segment_step(segment: &str) -> Option<(String, bool)> {
    let words: Vec<&str> = segment.split_whitespace().collect();
    let assignment = |w: &str| {
        w.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    let i = words
        .iter()
        .position(|w| !assignment(w) && !PREFIXES.contains(w))?;
    let name = words[i]
        .rsplit('/')
        .next()
        .unwrap_or(words[i])
        .trim_start_matches(['(', '{']);
    if name.is_empty() {
        return None;
    }
    let helper = HELPERS.contains(&name);
    let mut text = name.to_owned();
    if !helper && word(words.get(i + 1)) {
        text.push(' ');
        text.push_str(words[i + 1]);
        if words[i + 1] == "run" && word(words.get(i + 2)) {
            text.push(' ');
            text.push_str(words[i + 2]);
        }
    }
    Some((text, helper))
}

/// Short headline for a command: main programs in order, helpers folded.
/// Returns the headline and how many helper commands were folded away.
pub fn step_summary(command: &str) -> (String, usize) {
    let text = command.trim();
    let segments = shell_segments(text);
    if segments.len() <= 1 && text.chars().count() <= 60 {
        return (text.to_owned(), 0);
    }
    let steps: Vec<(String, bool)> = segments.iter().filter_map(|s| segment_step(s)).collect();
    let helpers = steps.iter().filter(|s| s.1).count();
    let mut main: Vec<&str> = Vec::new();
    for (step, helper) in &steps {
        if !helper && main.last() != Some(&step.as_str()) {
            main.push(step);
        }
    }
    if main.is_empty() {
        if text.chars().count() <= 60 {
            return (text.to_owned(), 0);
        }
        let first = steps
            .first()
            .map(|s| s.0.clone())
            .unwrap_or_else(|| text.split_whitespace().next().unwrap_or("").to_owned());
        return (first, steps.len().saturating_sub(1));
    }
    let mut headline = main.iter().take(3).copied().collect::<Vec<_>>().join(" → ");
    if main.len() > 3 {
        headline.push_str(" → …");
    }
    (headline, helpers)
}

/// Project name, source tag and grouping key for a task.
pub struct Project {
    pub key: String,
    pub name: String,
    pub tag: String,
    pub desktop: bool,
}

pub fn project_of(task: &Value, workspaces: &[Value]) -> Project {
    let connection = task["connection_id"].as_str().unwrap_or("");
    if task["kind"] == "mcp.call" {
        return Project {
            key: format!("{connection}|desktop"),
            name: tr("桌面").into(),
            tag: String::new(),
            desktop: true,
        };
    }
    let args = &task["arguments"];
    let dir = args["cwd"]
        .as_str()
        .or_else(|| args["remote_root"].as_str())
        .map(str::to_owned)
        .or_else(|| {
            args["path"]
                .as_str()
                .map(|p| p.rsplit_once('/').map_or("/", |(d, _)| d).to_owned())
        })
        .unwrap_or_default();
    let dir = match dir.trim_end_matches('/') {
        "" => "/".to_owned(),
        d => d.to_owned(),
    };
    let mut root = dir.clone();
    let mut best = 0;
    for w in workspaces {
        let owner = w["connection_id"].as_str().unwrap_or("");
        if !owner.is_empty() && !connection.is_empty() && owner != connection {
            continue;
        }
        let r = w["root"].as_str().unwrap_or("").trim_end_matches('/');
        if r.is_empty() || r.len() <= best {
            continue;
        }
        if dir == r || dir.starts_with(&format!("{r}/")) {
            root = r.to_owned();
            best = r.len();
        }
    }
    let parts: Vec<&str> = root.split('/').filter(|p| !p.is_empty()).collect();
    let name = parts.last().copied().unwrap_or(tr("未知目录")).to_owned();
    let mut tag = if parts.len() >= 2 {
        parts[parts.len() - 2].to_owned()
    } else {
        String::new()
    };
    if GENERIC_PARENTS.contains(&tag.to_lowercase().as_str()) {
        tag.clear();
    }
    // "typeflux-gul206/typeflux-api" → "gul206": drop the shared word.
    let base = name.split('-').next().unwrap_or("");
    if let Some(rest) = tag.strip_prefix(&format!("{name}-")) {
        tag = rest.to_owned();
    } else if let Some(rest) = tag
        .strip_prefix(&format!("{base}-"))
        .filter(|_| !base.is_empty())
    {
        tag = rest.to_owned();
    }
    // mktemp suffixes such as ".jVAPDk" carry no meaning for a person.
    if let Some((base, suffix)) = tag.rsplit_once('.')
        && suffix.len() == 6
        && suffix.chars().all(|c| c.is_ascii_alphanumeric())
    {
        tag = base.to_owned();
    }
    if tag == name {
        tag.clear();
    }
    Project {
        key: format!("{connection}|{root}"),
        name,
        tag,
        desktop: false,
    }
}

/// Paired servers are saved under their address. Screens are often shared,
/// so the panel names them by position instead (mirrors `format.ts`).
fn looks_like_address(name: &str) -> bool {
    let host_port = name.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty()
            && !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-[]:".contains(c))
    });
    let ip = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_hexdigit() || ".:[]".contains(c))
        && name.contains(['.', ':']);
    host_port || ip
}

fn server_label(name: &str, index: usize) -> String {
    if !looks_like_address(name) {
        name.to_owned()
    } else if index == 0 {
        tr("主服务器").into()
    } else {
        tr_format("服务器 {0}", &[format!("{}", index + 1)])
    }
}

fn active(t: &Value) -> bool {
    matches!(
        t["status"].as_str(),
        Some("accepted" | "running" | "awaiting_approval")
    )
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn ago(ms: u64) -> String {
    match ms / 60_000 {
        0 => tr("刚刚").into(),
        m if m < 60 => tr_format("{0} 分钟前", &[format!("{m}")]),
        m if m < 24 * 60 => tr_format("{0} 小时前", &[format!("{}", m / 60)]),
        m => tr_format("{0} 天前", &[format!("{}", m / (24 * 60))]),
    }
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

fn desktop_step(task: &Value) -> String {
    let tool = task["arguments"]["tool"].as_str().unwrap_or(tr("桌面操作"));
    let args = &task["arguments"]["arguments"];
    for key in ["text", "label", "title", "name", "app", "window", "key"] {
        if let Some(v) = args[key].as_str().filter(|v| !v.is_empty()) {
            return format!("{tool} · “{}”", v.chars().take(40).collect::<String>());
        }
    }
    if let (Some(x), Some(y)) = (args["x"].as_f64(), args["y"].as_f64()) {
        return format!("{tool} · ({x}, {y})");
    }
    tool.to_owned()
}

fn step_of(task: &Value) -> String {
    if task["kind"] == "sync" {
        let p = &task["progress"];
        return match (p["received"].as_u64(), p["total"].as_u64()) {
            (Some(r), Some(t)) => {
                tr_format("同步文件 · {0}/{1}", &[format!("{r}"), format!("{t}")])
            }
            _ => tr("同步文件").into(),
        };
    }
    if task["kind"] == "mcp.call" {
        return desktop_step(task);
    }
    step_summary(task["arguments"]["command"].as_str().unwrap_or(tr("命令"))).0
}

/// Problems the person should know about: Macrun-level failures only.
/// A command that exited non-zero is the agent's result, not a problem.
fn problem_count(summary: &Value) -> u64 {
    let n = |k: &str| summary[k].as_u64().unwrap_or(0);
    (n("failed") + n("timed_out") + n("unknown") + n("denied")).saturating_sub(n("exited"))
}

fn risk_reasons(command: &str) -> Vec<String> {
    const READ_ONLY: &[&str] = &[
        "pwd", "ls", "cat", "head", "tail", "wc", "stat", "file", "which", "whoami", "uname",
        "date", "echo", "printf",
    ];
    let mut reasons = Vec::new();
    if command.contains('|') {
        reasons.push(tr("管道").to_owned());
    }
    if command.contains(['>', '<']) {
        reasons.push(tr("重定向").to_owned());
    }
    if command.contains([';', '&', '\n']) {
        reasons.push(tr("组合命令").to_owned());
    }
    if command.contains(['`', '$', '(', ')']) {
        reasons.push(tr("命令替换").to_owned());
    }
    let first = command.split_whitespace().next().unwrap_or("");
    if !first.is_empty() && !READ_ONLY.contains(&first) {
        reasons.push(tr_format(
            "未知程序 {0}",
            &[first.rsplit('/').next().unwrap_or(first).to_string()],
        ));
    }
    reasons
}

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "observe" => tr("看屏幕"),
        "high" => tr("不可撤回的操作"),
        _ => tr("点击和输入"),
    }
}

/// Builds the panel model. `now` is milliseconds since the Unix epoch.
pub fn presentation(v: &Value, pending: Vec<String>, error: String, now: u64) -> Value {
    let available = !v.is_null();
    let tasks: Vec<&Value> = v["tasks"]
        .as_array()
        .map(|t| t.iter().collect())
        .unwrap_or_default();
    let workspaces: Vec<Value> = v["workspaces"].as_array().cloned().unwrap_or_default();
    let servers: Vec<(String, bool, Option<String>)> = match v["connections"].as_array() {
        Some(c) if !c.is_empty() => c
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    server_label(c["name"].as_str().unwrap_or(tr("服务器")), i),
                    c["connection"]["state"] == "connected",
                    c["connection"]["error"].as_str().map(str::to_owned),
                )
            })
            .collect(),
        _ => vec![(
            tr("服务器").into(),
            v["connection"]["state"] == "connected",
            v["connection"]["error"].as_str().map(str::to_owned),
        )],
    };
    let online = servers.iter().filter(|s| s.1).count();
    let label_of = |t: &Value| -> String {
        let id = t["connection_id"].as_str().unwrap_or("");
        v["connections"]
            .as_array()
            .and_then(|c| c.iter().position(|c| c["id"] == id))
            .map(|i| servers[i].0.clone())
            .unwrap_or_else(|| {
                t["connection_name"]
                    .as_str()
                    .map(|n| server_label(n, 1))
                    .unwrap_or_default()
            })
    };
    let paused = v["policy"]["paused"] == true;
    let approvals: Vec<&Value> = tasks
        .iter()
        .copied()
        .filter(|t| t["status"] == "awaiting_approval")
        .collect();
    let working: Vec<&Value> = tasks
        .iter()
        .copied()
        .filter(|t| matches!(t["status"].as_str(), Some("accepted" | "running")))
        .collect();

    // One row per project: the newest running task is the step shown.
    let mut groups: BTreeMap<String, (Project, &Value, usize)> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for t in &working {
        let p = project_of(t, &workspaces);
        match groups.get_mut(&p.key) {
            Some(entry) => {
                entry.2 += 1;
                if t["started_at"].as_u64() > entry.1["started_at"].as_u64() {
                    entry.1 = t;
                }
            }
            None => {
                order.push(p.key.clone());
                groups.insert(p.key.clone(), (p, t, 1));
            }
        }
    }
    let projects = order.len();
    let desktop_rows = groups.values().filter(|g| g.0.desktop).count();
    let rows: Vec<Value> = order
        .iter()
        .take(3)
        .map(|key| {
            let (p, t, _) = &groups[key];
            let started = t["started_at"].as_u64().unwrap_or(now);
            json!({
                "id": t["task_id"],
                "name": p.name,
                "tag": if servers.len() > 1 && t["connection_name"].is_string() {
                    let server = label_of(t);
                    if p.tag.is_empty() { server } else { format!("{} · {server}", p.tag) }
                } else {
                    p.tag.clone()
                },
                "elapsed": clock(now.saturating_sub(started)),
                "step": step_of(t),
                "tail": if p.desktop { String::new() } else { last_line(t["output_tail"].as_str().unwrap_or("")) },
                "desktop": p.desktop,
            })
        })
        .collect();

    // Idle panels list the last two finished projects so a glance confirms
    // that the earlier work really ended.
    let mut recent: Vec<Value> = Vec::new();
    if working.is_empty() && approvals.is_empty() {
        let mut seen: Vec<String> = Vec::new();
        for t in tasks.iter().filter(|t| !active(t)) {
            let p = project_of(t, &workspaces);
            if seen.contains(&p.key) {
                continue;
            }
            let count = tasks
                .iter()
                .filter(|o| project_of(o, &workspaces).key == p.key)
                .count();
            seen.push(p.key.clone());
            let ended = t["ended_at"]
                .as_u64()
                .or(t["started_at"].as_u64())
                .unwrap_or(now);
            recent.push(json!({"id":t["task_id"],"name":p.name,"tag":p.tag,"detail":tr_format("{0} 个任务", &[format!("{count}")]),"when":ago(now.saturating_sub(ended))}));
            if recent.len() == 2 {
                break;
            }
        }
    }

    let approval = approvals.first().map(|t| {
        let desktop = t["kind"] == "mcp.call";
        let project = project_of(t, &workspaces);
        let deadline = t["approval_deadline"]
            .as_u64()
            .unwrap_or_else(|| t["started_at"].as_u64().unwrap_or(now) + APPROVAL_MS);
        let left = deadline.saturating_sub(now).div_ceil(1000);
        let tier = t["desktop_tier"].as_str().unwrap_or("control");
        let command = if desktop {
            format!(
                "{} · {}",
                t["arguments"]["server"].as_str().unwrap_or(""),
                desktop_step(t)
            )
        } else {
            t["arguments"]["command"].as_str().unwrap_or("").to_owned()
        };
        let countdown = tr_format("{0} 秒后自动拒绝", &[format!("{left}")]);
        let server = if t["connection_name"].is_string() {
            label_of(t)
        } else {
            String::new()
        };
        let meta = [server.as_str(), countdown.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let reasons = if desktop {
            vec![tr_format("{0}类桌面操作", &[tier_label(tier).to_string()])]
        } else {
            risk_reasons(t["arguments"]["command"].as_str().unwrap_or(""))
        };
        json!({
            "id": t["task_id"],
            "title": if desktop { tr("Agent 想操作桌面").to_owned() } else { tr_format("{0} 想运行一条命令", std::slice::from_ref(&project.name)) },
            "meta": meta,
            "command": command,
            "reasons": reasons,
            "cwd": t["arguments"]["cwd"].as_str().unwrap_or(""),
            "similar": if desktop { tr("15 分钟内允许这个工具") } else { tr("15 分钟内允许同类命令") },
            "session": if desktop { tr_format("本次运行允许{0}", &[tier_label(tier).to_string()]) } else { tr("本次运行允许此目录").to_owned() },
            "count": approvals.len(),
        })
    });

    let mut problems: Vec<Value> = Vec::new();
    if available && online < servers.len() && online > 0 {
        for (name, ok, err) in &servers {
            if !ok {
                problems.push(json!({"text":tr_format("连不上 {0}", std::slice::from_ref(name)),"detail":err.clone().unwrap_or_else(|| tr("正在自动重连").into()),"button":tr("查看"),"action":{"action":"open","route":"desktop"}}));
            }
        }
    }
    if available && paused && (online > 0) && !(working.is_empty() && approvals.is_empty()) {
        problems.push(json!({"text":tr("已暂停接收新任务"),"detail":tr("正在进行的任务会继续完成"),"button":tr("恢复"),"action":{"action":"pause","args":{"paused":false}}}));
    }

    let connection_text = if servers.len() > 1 {
        tr_format(
            "{0}/{1} 台服务器在线",
            &[format!("{online}"), format!("{}", servers.len())],
        )
    } else {
        match v["connection"]["rtt_ms"].as_u64() {
            Some(ms) => tr_format("已连接 · {0} ms", &[format!("{ms}")]),
            None => tr("已连接").into(),
        }
    };
    let (tone, title, subtitle, primary) = if !available {
        (
            "off",
            tr("执行器未运行").to_owned(),
            tr("打开 Macrun 查看连接").to_owned(),
            json!({"label":tr("打开"),"action":{"action":"open","route":"desktop"}}),
        )
    } else if online == 0 {
        let (name, _, err) = &servers[0];
        (
            "error",
            if servers.len() > 1 {
                tr("所有服务器都未连接").to_owned()
            } else if name == tr("服务器") {
                tr("未连接服务器").to_owned()
            } else {
                tr_format("连不上 {0}", std::slice::from_ref(name))
            },
            err.clone().unwrap_or_else(|| tr("正在自动重连").into()),
            json!({"label":tr("检查连接"),"action":{"action":"open","route":"desktop"}}),
        )
    } else if let Some(a) = &approval {
        (
            "approval",
            a["title"].as_str().unwrap_or("").to_owned(),
            a["meta"].as_str().unwrap_or("").to_owned(),
            Value::Null,
        )
    } else if paused && working.is_empty() {
        (
            "paused",
            tr("已暂停接收新任务").to_owned(),
            connection_text.clone(),
            json!({"label":tr("恢复"),"action":{"action":"pause","args":{"paused":false}},"primary":true}),
        )
    } else if !working.is_empty() {
        let code_projects = projects - desktop_rows;
        let title = match (code_projects, desktop_rows) {
            (0, _) => tr("Agent 正在操作桌面").to_owned(),
            (1, _) => tr_format(
                "正在处理 {0}",
                &[groups[&order
                    .iter()
                    .find(|k| !groups[*k].0.desktop)
                    .cloned()
                    .unwrap_or_default()]
                    .0
                    .name
                    .to_string()],
            ),
            (n, _) => tr_format("正在 {0} 个项目上工作", &[format!("{n}")]),
        };
        let mut sub = Vec::new();
        if code_projects > 0 && desktop_rows > 0 {
            sub.push(tr("另有桌面操作").to_owned());
        }
        sub.push(connection_text.clone());
        (
            if code_projects == 0 { "desktop" } else { "run" },
            title,
            sub.join(" · "),
            json!({"label":tr("停止"),"action":{"action":"stop_all"},"danger":true}),
        )
    } else {
        (
            "ok",
            tr("就绪，等待 Agent").to_owned(),
            connection_text.clone(),
            Value::Null,
        )
    };
    let today_total = v["today_summary"]["total"].as_u64().unwrap_or(0);
    let today_problems = problem_count(&v["today_summary"]);
    let today = if available {
        tr_format(
            "今天 {0} 个任务 · {1}",
            &[
                format!("{today_total}"),
                (if today_problems == 0 {
                    tr("没有问题").to_owned()
                } else {
                    tr_format("{0} 个问题", &[format!("{today_problems}")])
                })
                .to_string(),
            ],
        )
    } else {
        String::new()
    };
    // Structure changes rebuild the AppKit views; text-only changes update
    // labels in place so latency ticks never flicker the panel.
    let structure = json!([
        crate::localization::locale(),
        tone,
        primary["label"],
        approval.as_ref().map(|a| a["id"].clone()),
        rows.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
        recent.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
        problems
            .iter()
            .map(|p| p["text"].clone())
            .collect::<Vec<_>>(),
        projects > 3,
    ])
    .to_string();
    json!({
        "labels": json!({"取消所有任务、暂停接收并关闭桌面控制（⌃⌥⌘.）":tr("取消所有任务、暂停接收并关闭桌面控制（⌃⌥⌘.）"),
    "拒绝":tr("拒绝"),
    "允许一次":tr("允许一次"),
    "更多允许方式":tr("更多允许方式"),
    "更多":tr("更多"),
    "桌面操作":tr("桌面操作"),
    "%@ %@，查看任务详情":tr("%@ %@，查看任务详情"),
    "还有 %ld 个项目":tr("还有 %ld 个项目"),
    "最近":tr("最近"),
    "恢复":tr("恢复"),
    "查看今天的活动":tr("查看今天的活动"),
    "接收新任务":tr("接收新任务"),
    "允许 Agent 操作桌面":tr("允许 Agent 操作桌面"),
    "打开 Macrun":tr("打开 Macrun"),
    "设置…":tr("设置…"),
    "退出 Macrun":tr("退出 Macrun"),
    "正在请求执行器…":tr("正在请求执行器…"),
    "Macrun · 快捷面板":tr("Macrun · 快捷面板")}),
        "tone": tone,
        "title": title,
        "subtitle": subtitle,
        "primary": primary,
        "approval": approval,
        "rows": rows,
        "more": projects.saturating_sub(3),
        "recent": recent,
        "problems": problems,
        "today": today,
        "today_problems": today_problems,
        "available": available,
        "pause": !paused,
        "desktop": v["policy"]["desktop_enabled"] == true,
        "pending": pending,
        "error": error,
        "structure": structure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_respect_quotes_escapes_and_redirections() {
        assert_eq!(
            shell_segments("a 2>&1 | b && c || d; e & f\ng"),
            ["a 2>&1", "b", "c", "d", "e", "f", "g"]
        );
        assert_eq!(
            shell_segments(r#"echo "a;b|c" 'd&&e' && f"#),
            [r#"echo "a;b|c" 'd&&e'"#, "f"]
        );
        assert_eq!(
            shell_segments(r"echo a\;b; c &> log"),
            [r"echo a\;b", "c &> log"]
        );
        assert!(shell_segments("").is_empty());
    }

    #[test]
    fn summaries_match_the_web_model() {
        assert_eq!(
            step_summary("swift test --enable-code-coverage"),
            ("swift test --enable-code-coverage".into(), 0)
        );
        assert_eq!(
            step_summary("ls .build | head"),
            ("ls .build | head".into(), 0)
        );
        let long = r#"swift build 2>&1 | grep -E "error:|Build complete"; swift build --build-tests 2>&1 | grep error; swift test --skip-build > /tmp/final.log 2>&1; echo TEST_EXIT $?; tail -1 /tmp/final.log"#;
        assert_eq!(step_summary(long), ("swift build → swift test".into(), 4));
        assert_eq!(
            step_summary(r#"P=$PWD; TB="$P/.build/out"; sudo env X=1 /usr/bin/codesign --force -s - "$TB/App.app""#).0,
            "codesign"
        );
        assert_eq!(
            step_summary("cd /work && npm run verify -- --fast").0,
            "npm run verify"
        );
        assert_eq!(
            step_summary(&format!("a; b; c; d; {}", "x".repeat(60))).0,
            "a → b → c → …"
        );
        assert_eq!(
            step_summary(
                "ls -la /very/long/path/that/goes/on/and/on | grep something | tail -5 | head -2"
            ),
            ("ls".into(), 3)
        );
    }

    #[test]
    fn projects_use_synced_roots_and_readable_tags() {
        let workspaces = vec![
            json!({"root":"/tmp/typeflux-gul207"}),
            json!({"root":"/tmp/typeflux-gul207/typeflux/"}),
            json!({"root":"/srv/other","connection_id":"b"}),
        ];
        let p = project_of(
            &json!({"kind":"exec.start","arguments":{"cwd":"/tmp/typeflux-gul207/typeflux/Sources"}}),
            &workspaces,
        );
        assert_eq!(p.key, "|/tmp/typeflux-gul207/typeflux");
        assert_eq!((p.name.as_str(), p.tag.as_str()), ("typeflux", "gul207"));
        let p = project_of(
            &json!({"kind":"exec.start","arguments":{"cwd":"/tmp/typeflux-gul203.jVAPDk/typeflux"}}),
            &[],
        );
        assert_eq!(p.tag, "gul203");
        let p = project_of(
            &json!({"kind":"exec.start","arguments":{"cwd":"/tmp/typeflux-gul206/typeflux-api"}}),
            &[],
        );
        assert_eq!(
            (p.name.as_str(), p.tag.as_str()),
            ("typeflux-api", "gul206")
        );
        let p = project_of(
            &json!({"kind":"exec.start","connection_id":"a","arguments":{"cwd":"/srv/other/x"}}),
            &workspaces,
        );
        assert_eq!(p.key, "a|/srv/other/x");
        let p = project_of(
            &json!({"kind":"file.read","arguments":{"path":"/m/proj/a.txt"}}),
            &[],
        );
        assert_eq!(p.name, "proj");
        assert!(project_of(&json!({"kind":"mcp.call","arguments":{}}), &[]).desktop);
        assert_eq!(
            project_of(&json!({"kind":"x","arguments":{}}), &[]).name,
            "未知目录"
        );
    }

    fn snapshot(tasks: Value) -> Value {
        json!({"connection":{"state":"connected","rtt_ms":38},"policy":{"paused":false,"desktop_enabled":true},"today_summary":{"total":46,"failed":3,"exited":3},"tasks":tasks,"workspaces":[]})
    }

    #[test]
    fn working_panel_groups_by_project_and_offers_stop() {
        let now = 1_000_000;
        let v = snapshot(json!([
            {"task_id":"a","kind":"exec.start","status":"running","started_at":now-356_000,"arguments":{"command":"swift build 2>&1 | tail -3; swift test 2>&1 | tail -1","cwd":"/tmp/typeflux-gul207/typeflux"},"output_tail":"Compiling\nBuild complete! (35.54 secs)\n\n"},
            {"task_id":"b","kind":"exec.start","status":"running","started_at":now-10_000,"arguments":{"command":"ls","cwd":"/tmp/typeflux-gul207/typeflux"}},
            {"task_id":"c","kind":"exec.start","status":"running","started_at":now-147_000,"arguments":{"command":"make test","cwd":"/tmp/typeflux-gul210/typeflux"}},
            {"task_id":"d","kind":"mcp.call","status":"running","started_at":now-2_000,"arguments":{"server":"computer","tool":"click","arguments":{"label":"Run"}}},
            {"task_id":"old","kind":"exec.start","status":"failed","started_at":now-900_000,"ended_at":now-899_000,"result":{"exit_code":1},"arguments":{"command":"grep x","cwd":"/w/app"}}
        ]));
        let p = presentation(&v, vec![], String::new(), now);
        assert_eq!(p["tone"], "run");
        assert_eq!(p["title"], "正在 2 个项目上工作");
        assert_eq!(p["subtitle"], "另有桌面操作 · 已连接 · 38 ms");
        assert_eq!(p["primary"]["action"]["action"], "stop_all");
        let rows = p["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        // The newest task in a project is the step shown for it.
        assert_eq!(rows[0]["id"], "b");
        assert_eq!(rows[0]["name"], "typeflux");
        assert_eq!(rows[0]["tag"], "gul207");
        assert_eq!(rows[0]["elapsed"], "0:10");
        assert_eq!(rows[1]["step"], "make test");
        assert_eq!(rows[2]["desktop"], true);
        assert_eq!(rows[2]["step"], "click · “Run”");
        assert_eq!(p["more"], 0);
        // Non-zero exits are not counted as problems.
        assert_eq!(p["today"], "今天 46 个任务 · 没有问题");
        assert!(p["recent"].as_array().unwrap().is_empty());
    }

    #[test]
    fn single_project_names_it_and_shows_the_last_output_line() {
        let now = 1_000_000;
        let v = snapshot(json!([
            {"task_id":"a","kind":"exec.start","status":"running","started_at":now-356_000,"arguments":{"command":"swift build 2>&1 | tail -3; swift test 2>&1 | tail -1","cwd":"/tmp/typeflux-gul207/typeflux"},"output_tail":"Compiling\nBuild complete! (35.54 secs)\n\n"}
        ]));
        let p = presentation(&v, vec![], String::new(), now);
        assert_eq!(p["title"], "正在处理 typeflux");
        assert_eq!(p["rows"][0]["step"], "swift build → swift test");
        assert_eq!(p["rows"][0]["tail"], "Build complete! (35.54 secs)");
        assert_eq!(p["rows"][0]["elapsed"], "5:56");
    }

    #[test]
    fn approvals_lead_with_the_request_and_count_down() {
        let now = 1_000_000;
        let v = snapshot(json!([
            {"task_id":"w","kind":"exec.start","status":"awaiting_approval","started_at":now-19_000,"approval_deadline":now+40_500,"connection_id":"g","connection_name":"gulu","arguments":{"command":"ps aux | grep macrun","cwd":"/tmp/typeflux-gul207/typeflux"}},
            {"task_id":"w2","kind":"mcp.call","status":"awaiting_approval","started_at":now,"desktop_tier":"high","arguments":{"server":"computer","tool":"kill_app","arguments":{"app":"Xcode"}}}
        ]));
        let p = presentation(&v, vec![], String::new(), now);
        assert_eq!(p["tone"], "approval");
        assert_eq!(p["title"], "typeflux 想运行一条命令");
        assert_eq!(p["subtitle"], "gulu · 41 秒后自动拒绝");
        assert_eq!(p["approval"]["count"], 2);
        assert_eq!(p["approval"]["reasons"], json!(["管道", "未知程序 ps"]));
        assert!(p["primary"].is_null());
        let desktop = presentation(
            &snapshot(json!([v["tasks"][1].clone()])),
            vec![],
            String::new(),
            now,
        );
        assert_eq!(desktop["approval"]["title"], "Agent 想操作桌面");
        assert_eq!(
            desktop["approval"]["command"],
            "computer · kill_app · “Xcode”"
        );
        assert_eq!(
            desktop["approval"]["reasons"][0],
            "不可撤回的操作类桌面操作"
        );
        assert_eq!(desktop["approval"]["session"], "本次运行允许不可撤回的操作");
        assert_eq!(desktop["approval"]["meta"], "60 秒后自动拒绝");
    }

    #[test]
    fn idle_panel_lists_recent_projects_and_problems() {
        let now = 10_000_000;
        let mut v = snapshot(json!([
            {"task_id":"a2","kind":"exec.start","status":"succeeded","started_at":now-600_000,"ended_at":now-300_000,"arguments":{"command":"make","cwd":"/w/app"}},
            {"task_id":"a1","kind":"exec.start","status":"succeeded","started_at":now-700_000,"ended_at":now-650_000,"arguments":{"command":"make","cwd":"/w/app"}},
            {"task_id":"b","kind":"exec.start","status":"timed_out","started_at":now-800_000,"ended_at":now-799_000,"arguments":{"command":"make","cwd":"/w/lib"}},
            {"task_id":"c","kind":"exec.start","status":"succeeded","started_at":now-900_000,"ended_at":now-899_000,"arguments":{"command":"make","cwd":"/w/three"}}
        ]));
        v["today_summary"] = json!({"total":4,"timed_out":1});
        let p = presentation(&v, vec![], String::new(), now);
        assert_eq!(p["tone"], "ok");
        assert_eq!(p["title"], "就绪，等待 Agent");
        assert!(p["primary"].is_null());
        assert_eq!(p["today"], "今天 4 个任务 · 1 个问题");
        let recent = p["recent"].as_array().unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0]["name"], "app");
        assert_eq!(recent[0]["detail"], "2 个任务");
        assert_eq!(recent[0]["when"], "5 分钟前");
        assert_eq!(recent[1]["name"], "lib");
    }

    #[test]
    fn offline_paused_and_partial_outages_offer_one_action_each() {
        let p = presentation(&Value::Null, vec![], "x".into(), 0);
        assert_eq!(p["tone"], "off");
        assert_eq!(p["available"], false);
        assert_eq!(p["today"], "");

        let mut v = snapshot(json!([]));
        v["connection"] = json!({"state":"reconnecting","error":"网络不可达"});
        let p = presentation(&v, vec![], String::new(), 0);
        assert_eq!(p["tone"], "error");
        assert_eq!(p["title"], "未连接服务器");
        assert_eq!(p["subtitle"], "网络不可达");
        assert_eq!(p["primary"]["action"]["route"], "desktop");

        v["connections"] = json!([
            {"name":"gulu","connection":{"state":"connected"}},
            {"name":"ci","connection":{"state":"reconnecting","error":"超时"}}
        ]);
        v["connection"] = json!({"state":"connected"});
        let p = presentation(&v, vec![], String::new(), 0);
        assert_eq!(p["tone"], "ok");
        assert_eq!(p["subtitle"], "1/2 台服务器在线");
        assert_eq!(p["problems"][0]["text"], "连不上 ci");
        assert_eq!(p["problems"][0]["detail"], "超时");

        v["connections"] = json!([{"name":"gulu","connection":{"state":"reconnecting"}}]);
        v["connection"] = json!({"state":"reconnecting"});
        assert_eq!(
            presentation(&v, vec![], String::new(), 0)["title"],
            "连不上 gulu"
        );

        let mut v = snapshot(json!([]));
        v["policy"]["paused"] = json!(true);
        let p = presentation(&v, vec!["pause".into()], String::new(), 0);
        assert_eq!(p["tone"], "paused");
        assert_eq!(p["pause"], false);
        assert_eq!(
            p["primary"]["action"],
            json!({"action":"pause","args":{"paused":false}})
        );
        assert_eq!(p["pending"][0], "pause");

        v["tasks"] = json!([{"task_id":"r","kind":"exec.start","status":"running","started_at":0,"arguments":{"command":"make","cwd":"/w/app"}}]);
        let p = presentation(&v, vec![], String::new(), 0);
        assert_eq!(p["tone"], "run");
        assert_eq!(p["problems"][0]["button"], "恢复");
    }

    #[test]
    fn server_addresses_are_replaced_by_stable_labels() {
        assert!(looks_like_address("100.86.200.16:7443"));
        assert!(looks_like_address("server.example:7443"));
        assert!(looks_like_address("[fd00::1]:7443"));
        assert!(looks_like_address("10.0.0.1"));
        assert!(!looks_like_address("gulu-server"));
        assert!(!looks_like_address("Build server"));
        let mut v = snapshot(json!([
            {"task_id":"w","kind":"exec.start","status":"awaiting_approval","started_at":0,"connection_id":"b","connection_name":"10.1.2.3:7443","arguments":{"command":"ls","cwd":"/w/app"}}
        ]));
        v["connections"] = json!([
            {"id":"a","name":"100.86.200.16:7443","connection":{"state":"connected"}},
            {"id":"b","name":"10.1.2.3:7443","connection":{"state":"reconnecting"}}
        ]);
        let p = presentation(&v, vec![], String::new(), 0);
        let text = p.to_string();
        assert!(!text.contains("100.86") && !text.contains("10.1.2.3"));
        assert_eq!(p["approval"]["meta"], "服务器 2 · 60 秒后自动拒绝");
        assert_eq!(p["problems"][0]["text"], "连不上 服务器 2");
    }

    #[test]
    fn structure_ignores_ticks_but_tracks_rows() {
        let now = 1_000_000;
        let v = snapshot(json!([
            {"task_id":"a","kind":"exec.start","status":"running","started_at":now-1000,"arguments":{"command":"make","cwd":"/w/app"},"output_tail":"1"}
        ]));
        let a = presentation(&v, vec![], String::new(), now);
        let mut later = v.clone();
        later["tasks"][0]["output_tail"] = json!("2");
        later["connection"]["rtt_ms"] = json!(99);
        let b = presentation(&later, vec![], String::new(), now + 5000);
        assert_eq!(a["structure"], b["structure"]);
        assert_ne!(a["rows"][0]["elapsed"], b["rows"][0]["elapsed"]);
        let empty = presentation(&snapshot(json!([])), vec![], String::new(), now);
        assert_ne!(a["structure"], empty["structure"]);
    }
}
