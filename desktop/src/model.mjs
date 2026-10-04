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
export const title = (t) =>
  t.arguments?.command ||
  (t.kind === "sync"
    ? `同步 → ${t.arguments?.remote_root || "工作区"}`
    : t.arguments?.tool || t.arguments?.path || t.kind);
export function selectTasks(tasks, filter, query) {
  const q = query.toLowerCase();
  return tasks.filter(
    (t) =>
      (filter === "all" || t.status === filter) &&
      `${title(t)} ${t.arguments?.cwd || ""} ${t.task_id}`
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
