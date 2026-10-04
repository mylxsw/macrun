export const statuses = {
  awaiting_approval: "待确认",
  denied: "已拒绝",
  accepted: "已接收",
  running: "运行中",
  succeeded: "成功",
  failed: "失败",
  cancelled: "已取消",
  timed_out: "超时",
  unknown: "未知",
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
  (statusGroups[filter] ? statusGroups[filter].includes(status) : status === filter);
export const tierLabels = {
  observe: "观察",
  control: "操作",
  high: "高风险",
};
export const tierPolicyLabels = {
  allow: "允许并提示",
  confirm: "先确认",
  deny: "禁止",
};
const readOnlyPrograms = [
  "pwd", "ls", "cat", "head", "tail", "wc", "stat", "file", "which", "whoami",
  "uname", "date", "echo", "printf",
];
export const program = (command = "") => {
  const first = command.trim().split(/\s+/)[0] || "";
  return first.split("/").pop() || first;
};
// Mirrors the worker's risk filter so the prompt can say why it asked.
export function riskReasons(command = "") {
  const reasons = [];
  if (/[|]/.test(command)) reasons.push("管道");
  if (/[><]/.test(command)) reasons.push("重定向");
  if (/[;&\n]/.test(command)) reasons.push("组合命令");
  if (/[`$()]/.test(command)) reasons.push("命令替换");
  const first = command.trim().split(/\s+/)[0] || "";
  if (first && !readOnlyPrograms.includes(first))
    reasons.push(`未知程序 ${program(command)}`);
  return reasons;
}
export const title = (t) =>
  t.arguments?.command ||
  (t.kind === "sync"
    ? `同步 → ${t.arguments?.remote_root || "工作区"}`
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
        return r;
      },
      { total: 0 },
    );
}
