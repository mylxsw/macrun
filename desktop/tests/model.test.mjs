import { test } from "node:test";
import assert from "node:assert/strict";
import {
  selectTasks,
  todaySummary,
  active,
  statuses,
  riskReasons,
  program,
  statusMatches,
} from "../src/model.mjs";
const now = new Date(2026, 9, 4, 12);
const tasks = Object.keys(statuses).map((status, i) => ({
  status,
  task_id: `task-${i}`,
  kind: "exec.start",
  arguments: { command: `command ${i}`, cwd: "/work/app" },
  started_at: now.getTime(),
}));
test("all nine states are independently filterable", () => {
  for (const state of Object.keys(statuses)) {
    assert.equal(selectTasks(tasks, state, "").length, 1);
  }
});
test("search matches command cwd and id", () => {
  assert.equal(selectTasks(tasks, "all", "/work/app").length, 9);
  assert.equal(selectTasks(tasks, "all", "task-3").length, 1);
  assert.equal(selectTasks(tasks, "all", "command 1").length, 1);
  assert.equal(selectTasks(tasks, "failed", "task-1").length, 0);
});
test("search includes sync roots and file paths even when another argument supplies the title", () => {
  const pathTasks = [
    {
      ...tasks[0],
      task_id: "sync-target",
      kind: "sync",
      arguments: { command: "upload source", remote_root: "/work/Counter" },
    },
    {
      ...tasks[1],
      task_id: "file-target",
      kind: "mcp.call",
      arguments: { tool: "read_file", path: "/work/notes/README.md" },
    },
  ];
  assert.deepEqual(
    selectTasks(pathTasks, "all", "/WORK/counter").map((t) => t.task_id),
    ["sync-target"],
  );
  assert.deepEqual(
    selectTasks(pathTasks, "all", "notes/readme").map((t) => t.task_id),
    ["file-target"],
  );
  assert.equal(selectTasks(pathTasks, "succeeded", "notes/readme").length, 0);
});
test("today summary uses local received date; active includes accepted", () => {
  assert.equal(
    todaySummary(
      [...tasks, { ...tasks[0], started_at: now.getTime() - 86400000 }],
      now,
    ).total,
    9,
  );
  assert.equal(tasks.filter(active).length, 3);
});
test("status groups match the worker's active and attention filters", () => {
  assert.deepEqual(
    selectTasks(tasks, "attention", "").map((t) => t.status).sort(),
    ["denied", "failed", "timed_out", "unknown"],
  );
  assert.deepEqual(
    selectTasks(tasks, "active", "").map((t) => t.status).sort(),
    ["accepted", "awaiting_approval", "running"],
  );
  assert.equal(statusMatches("all", "anything"), true);
  assert.equal(statusMatches("failed", "denied"), false);
});
test("risk reasons explain why a command waits for approval", () => {
  assert.deepEqual(riskReasons("ls -la"), []);
  assert.deepEqual(riskReasons("ps aux | grep macrun"), ["管道", "未知程序 ps"]);
  assert.deepEqual(riskReasons("echo hi > out; ls"), ["重定向", "组合命令"]);
  assert.deepEqual(riskReasons("echo $(id)"), ["命令替换"]);
  assert.equal(program("/usr/bin/git status"), "git");
  assert.equal(program(""), "");
});
