import { tr } from "./i18n.mjs";
export const statuses = {
  get awaiting_approval() {
    return tr("待确认");
  },
  get denied() {
    return tr("已拒绝");
  },
  get accepted() {
    return tr("已接收");
  },
  get running() {
    return tr("运行中");
  },
  get succeeded() {
    return tr("成功");
  },
  get failed() {
    return tr("失败");
  },
  get cancelled() {
    return tr("已取消");
  },
  get timed_out() {
    return tr("超时");
  },
  get unknown() {
    return tr("未知");
  },
};
export const active = (t) =>
  ["accepted", "running", "awaiting_approval"].includes(t.status);
// Status groups shared with the worker's task_list filter.
export const statusGroups = {
  active: ["accepted", "running", "awaiting_approval"],
  attention: ["failed", "timed_out", "unknown", "denied"],
};
export const statusMatches = (filter, status) =>
  filter === "all" ||
  (statusGroups[filter]
    ? statusGroups[filter].includes(status)
    : status === filter);
export const tierLabels = {
  get observe() {
    return tr("看屏幕");
  },
  get control() {
    return tr("点击和输入");
  },
  get high() {
    return tr("不可撤回的操作");
  },
};
export const tierPolicyLabels = {
  get allow() {
    return tr("允许");
  },
  get confirm() {
    return tr("先问");
  },
  get deny() {
    return tr("禁止");
  },
};
const readOnlyPrograms = [
  "pwd",
  "ls",
  "cat",
  "head",
  "tail",
  "wc",
  "stat",
  "file",
  "which",
  "whoami",
  "uname",
  "date",
  "echo",
  "printf",
];
export const program = (command = "") => {
  const first = command.trim().split(/\s+/)[0] || "";
  return first.split("/").pop() || first;
};
// Mirrors the worker's risk filter so the prompt can say why it asked.
export function riskReasons(command = "") {
  const reasons = [];
  if (/[|]/.test(command)) reasons.push(tr("管道"));
  if (/[><]/.test(command)) reasons.push(tr("重定向"));
  if (/[;&\n]/.test(command)) reasons.push(tr("组合命令"));
  if (/[`$()]/.test(command)) reasons.push(tr("命令替换"));
  const first = command.trim().split(/\s+/)[0] || "";
  if (first && !readOnlyPrograms.includes(first))
    reasons.push(tr("未知程序 {0}", program(command)));
  return reasons;
}
export const title = (t) =>
  t.arguments?.command ||
  (t.kind === "sync"
    ? tr("同步 → {0}", t.arguments?.remote_root || tr("工作区"))
    : t.arguments?.tool || t.arguments?.path || t.kind);
export function selectTasks(tasks, filter, query) {
  const q = query.toLowerCase();
  return tasks.filter(
    (t) =>
      statusMatches(filter, t.status) &&
      `${title(t)} ${t.arguments?.cwd || ""} ${t.arguments?.remote_root || ""} ${t.arguments?.path || ""} ${t.task_id}`
        .toLowerCase()
        .includes(q),
  );
}
export function todaySummary(tasks, now = new Date()) {
  return tasks
    .filter((t) => new Date(t.started_at).toDateString() === now.toDateString())
    .reduce(
      (r, t) => {
        r.total++;
        r[t.status] = (r[t.status] || 0) + 1;
        if (agentExit(t)) r.exited = (r.exited || 0) + 1;
        return r;
      },
      { total: 0 },
    );
}

// v4: tell the agent's own results apart from Macrun problems, and describe
// work by project instead of by raw shell command.

/** A command that ran and exited non-zero; the agent handles its exit code. */
export const agentExit = (t) =>
  t.status === "failed" &&
  !t.error &&
  typeof t.result?.exit_code === "number" &&
  t.result.exit_code !== 0;
/** Mirrors the worker's "attention" group: Macrun-level outcomes only. */
export const needsAttention = (t) =>
  statusGroups.attention.includes(t.status) && !agentExit(t);
/** Count of Macrun problems in a status→count map that may carry "exited". */
export const attentionCount = (counts = {}) =>
  Math.max(
    0,
    statusGroups.attention.reduce((sum, s) => sum + (counts[s] || 0), 0) -
      (counts.exited || 0),
  );

// Programs that only inspect, move or filter; folded out of a step headline.
const helperPrograms = new Set([
  "cd",
  "ls",
  "cp",
  "mv",
  "rm",
  "mkdir",
  "touch",
  "chmod",
  "ln",
  "echo",
  "printf",
  "grep",
  "egrep",
  "rg",
  "tail",
  "head",
  "cut",
  "sed",
  "awk",
  "cat",
  "tr",
  "sort",
  "uniq",
  "wc",
  "tee",
  "true",
  "false",
  "test",
  "[",
  "export",
  "set",
  "pwd",
  "sleep",
  "xargs",
  "find",
  "basename",
  "dirname",
  "date",
  "which",
  "stat",
  "file",
  "less",
  "more",
  "jq",
]);
const prefixWords = new Set([
  "sudo",
  "env",
  "time",
  "nohup",
  "exec",
  "command",
  "(",
  "{",
]);

/** Splits a shell line on ; && || | & and newlines, outside quotes. */
export function shellSegments(command = "") {
  const parts = [];
  let current = "",
    quote = "";
  for (let i = 0; i < command.length; i++) {
    const c = command[i],
      next = command[i + 1],
      prev = command[i - 1];
    if (quote) {
      current += c;
      if (c === "\\" && quote === '"' && next !== undefined)
        current += command[++i];
      else if (c === quote) quote = "";
      continue;
    }
    if (c === "\\" && next !== undefined) {
      current += c + command[++i];
      continue;
    }
    if (c === "'" || c === '"') {
      quote = c;
      current += c;
      continue;
    }
    // ">&", "&>" and "2>&1" are redirections, not separators.
    const redirect = c === "&" && (prev === ">" || next === ">");
    if (c === ";" || c === "\n" || c === "|" || (c === "&" && !redirect)) {
      if ((c === "|" || c === "&") && next === c) i++;
      parts.push(current);
      current = "";
      continue;
    }
    current += c;
  }
  parts.push(current);
  return parts.map((p) => p.trim()).filter(Boolean);
}

/** The program of one segment, skipping env assignments and wrappers. */
function segmentStep(segment) {
  const words = segment.split(/\s+/).filter(Boolean);
  let i = 0;
  while (
    i < words.length &&
    (/^[A-Za-z_][A-Za-z0-9_]*=/.test(words[i]) || prefixWords.has(words[i]))
  )
    i++;
  if (i >= words.length) return null;
  const name = (words[i].split("/").pop() || words[i]).replace(/^[({]+/, "");
  if (!name) return null;
  const helper = helperPrograms.has(name);
  const word = (w) => !!w && /^[a-z][a-z0-9:_-]*$/.test(w);
  let sub = "";
  if (!helper && word(words[i + 1])) {
    sub = ` ${words[i + 1]}`;
    // "npm run verify" names the script, which is the useful part.
    if (words[i + 1] === "run" && word(words[i + 2])) sub += ` ${words[i + 2]}`;
  }
  return { text: name + sub, helper };
}

/**
 * Short headline for a command: the main programs in order, helpers folded.
 * Short single commands stay verbatim because they are already readable.
 */
export function stepSummary(command = "") {
  const text = command.trim();
  if (!text) return { text: "", helpers: 0, summarized: false };
  const segments = shellSegments(text);
  if (segments.length <= 1 && text.length <= 60)
    return { text, helpers: 0, summarized: false };
  const steps = segments.map(segmentStep).filter(Boolean);
  const main = steps
    .filter((s) => !s.helper)
    .map((s) => s.text)
    .filter((s, i, all) => s !== all[i - 1]);
  if (!main.length) {
    if (text.length <= 60) return { text, helpers: 0, summarized: false };
    const first = steps[0]?.text || text.split(/\s+/)[0];
    return {
      text: first,
      helpers: Math.max(0, steps.length - 1),
      summarized: true,
    };
  }
  const shown = main.slice(0, 3);
  return {
    text: shown.join(" → ") + (main.length > 3 ? " → …" : ""),
    helpers: steps.filter((s) => s.helper).length,
    summarized: true,
  };
}

/** One-line name for any task kind, used in lists and the menu bar. */
export function taskHeadline(t) {
  if (t.kind === "sync") return tr("同步文件");
  if (t.kind === "mcp.call")
    return (
      [t.arguments?.tool, describeDesktopArgs(t.arguments?.arguments)]
        .filter(Boolean)
        .join(" · ") || tr("桌面操作")
    );
  return stepSummary(t.arguments?.command || title(t)).text;
}
function describeDesktopArgs(args) {
  if (!args || typeof args !== "object") return "";
  for (const key of ["text", "label", "title", "name", "app", "window", "key"])
    if (typeof args[key] === "string" && args[key])
      return `“${args[key].slice(0, 40)}”`;
  if (typeof args.x === "number" && typeof args.y === "number")
    return `(${args.x}, ${args.y})`;
  return "";
}

const genericParents = new Set([
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
]);
const taskDir = (t) => {
  const a = t.arguments || {};
  if (a.cwd || a.remote_root) return String(a.cwd || a.remote_root);
  if (a.path) return String(a.path).replace(/\/[^/]*$/, "") || "/";
  return "";
};
/**
 * Which project a task belongs to: the longest synced workspace root that
 * contains its directory, else the directory itself. Desktop calls form one
 * project per server.
 */
export function projectOf(t, workspaces = []) {
  const connection = t.connection_id || "";
  if (t.kind === "mcp.call")
    return {
      key: `${connection}|desktop`,
      name: tr("桌面操作"),
      tag: "",
      root: "",
      desktop: true,
    };
  const dir = taskDir(t).replace(/\/+$/, "") || "/";
  let root = dir;
  let best = -1;
  for (const w of workspaces) {
    if (w.connection_id && connection && w.connection_id !== connection)
      continue;
    const r = String(w.root || "").replace(/\/+$/, "");
    if (!r || r.length <= best) continue;
    if (dir === r || dir.startsWith(`${r}/`)) {
      root = r;
      best = r.length;
    }
  }
  const parts = root.split("/").filter(Boolean);
  const name = parts.at(-1) || root || tr("未知目录");
  let tag = parts.at(-2) || "";
  if (genericParents.has(tag.toLowerCase())) tag = "";
  // "typeflux-gul206/typeflux-api" → "gul206": drop the shared word.
  const base = name.split("-")[0];
  if (tag.startsWith(`${name}-`)) tag = tag.slice(name.length + 1);
  else if (base && tag.startsWith(`${base}-`)) tag = tag.slice(base.length + 1);
  tag = tag.replace(/\.[A-Za-z0-9]{6}$/, "");
  if (tag === name) tag = "";
  return { key: `${connection}|${root}`, name, tag, root, desktop: false };
}

export const SESSION_GAP_MS = 30 * 60 * 1000;
/**
 * Groups tasks into project sessions: same project and server, with no gap
 * longer than `gap` between one task ending and the next starting.
 * Sessions are returned newest first; their tasks are newest first.
 */
export function groupSessions(
  tasks,
  workspaces = [],
  gap = SESSION_GAP_MS,
  now = Date.now(),
) {
  const open = new Map();
  const sessions = [];
  for (const t of [...tasks].sort((a, b) => a.started_at - b.started_at)) {
    const project = projectOf(t, workspaces);
    const end = active(t) ? now : t.ended_at || t.started_at;
    let s = open.get(project.key);
    if (!s || t.started_at - s.end > gap) {
      s = {
        id: `${project.key}@${t.started_at}`,
        project,
        connection_id: t.connection_id,
        connection_name: t.connection_name,
        start: t.started_at,
        end,
        tasks: [],
      };
      open.set(project.key, s);
      sessions.push(s);
    }
    s.tasks.push(t);
    s.end = Math.max(s.end, end);
  }
  for (const s of sessions) {
    s.tasks.reverse();
    s.active = s.tasks.some(active);
    s.problems = s.tasks.filter(needsAttention).length;
    s.exited = s.tasks.filter(agentExit).length;
  }
  return sessions.sort(
    (a, b) => Number(b.active) - Number(a.active) || b.end - a.end,
  );
}

/** Plain-language explanation for a finished task, or null when it ran fine. */
export function explainTask(t) {
  if (agentExit(t))
    return {
      agentExit: true,
      what: tr("命令以退出码 {0} 结束。", t.result.exit_code),
      agent: tr("Agent 会读取退出码和输出，自行决定下一步。"),
      you: tr("这是命令自己的结果，不是 Macrun 的问题，通常无需处理。"),
    };
  const code = t.error?.code;
  const message = t.error?.message || "";
  if (code === "approval_expired")
    return {
      what: tr("60 秒内没有人处理这个确认请求，命令没有执行。"),
      agent: tr("收到 approval_expired，可以稍后重试或换个做法。"),
      you: tr("需要时及时在菜单栏或通知中确认；也可以在权限页放宽规则。"),
    };
  if (code === "approval_rejected")
    return {
      what: tr("你拒绝了这个请求，没有执行。"),
      agent: tr("收到 approval_rejected，知道是你拒绝的。"),
      you: tr("无需处理。"),
    };
  if (code === "approval_interrupted")
    return {
      what: tr("执行器重启时请求仍在等待确认，已自动拒绝。"),
      agent: tr("收到 approval_interrupted，可以重新发起。"),
      you: tr("无需处理。"),
    };
  switch (t.status) {
    case "denied":
      return {
        what: message || tr("请求被安全规则拒绝，没有执行。"),
        agent: tr("收到 denied，知道这项操作当前不被允许。"),
        you: tr("如需允许，在权限页调整对应规则。"),
      };
    case "timed_out":
      return {
        what:
          message && message !== "timed_out"
            ? tr("超过时限后被停止：{0}", message)
            : tr("超过时限后被停止。"),
        agent: tr(
          "收到 timed_out，可以缩小范围或延长时限后重试；Macrun 不会自动重放。",
        ),
        you: tr("通常无需处理。频繁出现时检查网络或任务规模。"),
      };
    case "unknown":
      return {
        what: tr("执行中断，操作可能已经生效，也可能没有。"),
        agent: tr("收到 unknown；Macrun 不会自动重放。"),
        you: tr("先核对本机状态再决定下一步；桌面操作可在本机页实拍核对。"),
      };
    case "failed":
      return {
        what: message || tr("任务没有正常完成。"),
        agent: tr("收到 failed 和上面的错误信息。"),
        you: tr("查看下方输出；反复出现时在设置中导出诊断包。"),
      };
    case "cancelled":
      return {
        what: tr("任务被取消。"),
        agent: tr("收到 cancelled。"),
        you: tr("无需处理。"),
      };
  }
  return null;
}
/** Short label for a problem, e.g. in a one-line list. */
export const problemLabel = (t) =>
  t.error?.code === "approval_expired"
    ? tr("确认已过期")
    : t.error?.code === "approval_rejected"
      ? tr("已拒绝")
      : t.kind === "sync" && t.status !== "succeeded"
        ? tr("同步{0}", statuses[t.status] || t.status)
        : statuses[t.status] || t.status;

/** Permission presets on the access page. Directory limits are separate. */
export const presets = {
  open: {
    get label() {
      return tr("放手");
    },
    get detail() {
      return tr(
        "命令和桌面操作都直接执行，操作桌面时显示屏幕提示。适合专门给 Agent 用的 Mac。",
      );
    },
    approval: "direct",
    desktop: { observe: "allow", control: "allow", high: "allow" },
  },
  balanced: {
    get label() {
      return tr("平衡");
    },
    get detail() {
      return tr("风险命令和不可撤回的桌面操作先问你，其余直接执行。");
    },
    approval: "risk",
    desktop: { observe: "allow", control: "allow", high: "confirm" },
  },
  careful: {
    get label() {
      return tr("谨慎");
    },
    get detail() {
      return tr("每条命令、每次桌面操作都先问你。适合日常使用的主力机。");
    },
    approval: "all",
    desktop: { observe: "confirm", control: "confirm", high: "confirm" },
  },
};
/** Which preset a safety policy matches, or "custom". */
export function presetOf(safety) {
  if (!safety?.desktop) return "custom";
  for (const [key, p] of Object.entries(presets))
    if (
      safety.approval === p.approval &&
      ["observe", "control", "high"].every(
        (k) => safety.desktop[k] === p.desktop[k],
      )
    )
      return key;
  return "custom";
}
