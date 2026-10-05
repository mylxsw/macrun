import { test } from "node:test";
import assert from "node:assert/strict";
import {
  agentExit,
  needsAttention,
  attentionCount,
  shellSegments,
  stepSummary,
  taskHeadline,
  projectOf,
  groupSessions,
  explainTask,
  problemLabel,
  presets,
  presetOf,
  todaySummary,
} from "../src/model.mjs";

const exec = (command, extra = {}) => ({
  task_id: extra.task_id || command,
  kind: "exec.start",
  status: "succeeded",
  arguments: { command, cwd: "/work/app" },
  started_at: 1000,
  ...extra,
});

test("non-zero exits belong to the agent, Macrun errors need attention", () => {
  const exited = exec("grep x", { status: "failed", result: { exit_code: 1 } });
  const broken = exec("x", { status: "failed", error: { message: "spawn" } });
  const zero = exec("x", { status: "failed", result: { exit_code: 0 } });
  assert.equal(agentExit(exited), true);
  assert.equal(needsAttention(exited), false);
  assert.equal(agentExit(broken), false);
  assert.equal(needsAttention(broken), true);
  assert.equal(agentExit(zero), false);
  for (const status of ["timed_out", "unknown", "denied"])
    assert.equal(needsAttention(exec("x", { status })), true);
  for (const status of ["succeeded", "running", "cancelled"])
    assert.equal(needsAttention(exec("x", { status })), false);
  assert.equal(
    attentionCount({
      failed: 5,
      timed_out: 1,
      unknown: 2,
      denied: 1,
      exited: 4,
    }),
    5,
  );
  assert.equal(attentionCount({ exited: 3 }), 0);
  assert.equal(attentionCount(), 0);
});

test("today summary counts non-zero exits separately", () => {
  const now = new Date(2026, 9, 5, 12);
  const at = now.getTime();
  const summary = todaySummary(
    [
      exec("a", { status: "failed", result: { exit_code: 2 }, started_at: at }),
      exec("b", { status: "failed", error: { message: "x" }, started_at: at }),
    ],
    now,
  );
  assert.deepEqual(summary, { total: 2, failed: 2, exited: 1 });
});

test("shell segments split on operators outside quotes and redirections", () => {
  assert.deepEqual(shellSegments(`a 2>&1 | b && c || d; e & f\ng`), [
    "a 2>&1",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
  ]);
  assert.deepEqual(shellSegments(`echo "a;b|c" 'd&&e' && f`), [
    `echo "a;b|c" 'd&&e'`,
    "f",
  ]);
  assert.deepEqual(shellSegments(String.raw`echo a\;b; c &> log`), [
    String.raw`echo a\;b`,
    "c &> log",
  ]);
  assert.deepEqual(shellSegments(`echo "x \\" ; y"; z`), [
    `echo "x \\" ; y"`,
    "z",
  ]);
  assert.deepEqual(shellSegments(""), []);
});

test("step summary folds helpers and keeps short commands verbatim", () => {
  assert.deepEqual(stepSummary(""), {
    text: "",
    helpers: 0,
    summarized: false,
  });
  assert.deepEqual(stepSummary("swift test --enable-code-coverage"), {
    text: "swift test --enable-code-coverage",
    helpers: 0,
    summarized: false,
  });
  assert.equal(stepSummary("ls .build | head").text, "ls .build | head");
  const long = `swift build 2>&1 | grep -E "error:|Build complete"; swift build --build-tests 2>&1 | grep error; swift test --skip-build > /tmp/final.log 2>&1; echo TEST_EXIT $?; tail -1 /tmp/final.log`;
  assert.deepEqual(stepSummary(long), {
    text: "swift build → swift test",
    helpers: 4,
    summarized: true,
  });
  assert.equal(
    stepSummary(
      `P=$PWD; TB="$P/.build/out"; sudo env X=1 /usr/bin/codesign --force -s - "$TB/App.app"`,
    ).text,
    "codesign",
  );
  assert.equal(
    stepSummary("cd /work && npm run verify -- --fast").text,
    "npm run verify",
  );
  assert.equal(
    stepSummary("a; b; c; d; " + "x".repeat(60)).text,
    "a → b → c → …",
  );
  const helpersOnly = `ls -la /very/long/path/that/goes/on/and/on | grep something | tail -5 | head -2`;
  assert.deepEqual(stepSummary(helpersOnly), {
    text: "ls",
    helpers: 3,
    summarized: true,
  });
});

test("headlines describe sync and desktop calls in words", () => {
  assert.equal(taskHeadline({ kind: "sync", arguments: {} }), "同步文件");
  assert.equal(
    taskHeadline({
      kind: "mcp.call",
      arguments: { tool: "click", arguments: { label: "Run" } },
    }),
    "click · “Run”",
  );
  assert.equal(
    taskHeadline({
      kind: "mcp.call",
      arguments: { tool: "click", arguments: { x: 4, y: 5 } },
    }),
    "click · (4, 5)",
  );
  assert.equal(taskHeadline({ kind: "mcp.call", arguments: {} }), "桌面操作");
  assert.equal(taskHeadline(exec("make")), "make");
});

test("projects use the longest synced root and a readable source tag", () => {
  const workspaces = [
    { root: "/tmp/typeflux-gul207" },
    { root: "/tmp/typeflux-gul207/typeflux/" },
    { root: "/srv/other", connection_id: "b" },
  ];
  const p = projectOf(
    {
      kind: "exec.start",
      arguments: { cwd: "/tmp/typeflux-gul207/typeflux/Sources" },
    },
    workspaces,
  );
  assert.deepEqual(p, {
    key: "|/tmp/typeflux-gul207/typeflux",
    name: "typeflux",
    tag: "gul207",
    root: "/tmp/typeflux-gul207/typeflux",
    desktop: false,
  });
  assert.equal(
    projectOf({
      kind: "exec.start",
      arguments: { cwd: "/tmp/typeflux-gul203.jVAPDk/typeflux" },
    }).tag,
    "gul203",
  );
  assert.equal(
    projectOf({ kind: "exec.start", arguments: { cwd: "/Users/me/codes/app" } })
      .tag,
    "",
  );
  // Roots of another server never claim this server's task.
  assert.equal(
    projectOf(
      {
        kind: "exec.start",
        connection_id: "a",
        arguments: { cwd: "/srv/other/x" },
      },
      workspaces,
    ).root,
    "/srv/other/x",
  );
  assert.equal(
    projectOf({ kind: "sync", arguments: { remote_root: "/m/proj" } }).name,
    "proj",
  );
  assert.equal(
    projectOf({ kind: "file.read", arguments: { path: "/m/proj/a.txt" } }).root,
    "/m/proj",
  );
  assert.equal(projectOf({ kind: "x", arguments: {} }).root, "/");
  const desktop = projectOf({
    kind: "mcp.call",
    connection_id: "a",
    arguments: {},
  });
  assert.equal(desktop.key, "a|desktop");
  assert.equal(desktop.desktop, true);
});

test("tags drop the word shared with the project name", () => {
  assert.equal(
    projectOf({
      kind: "exec.start",
      arguments: { cwd: "/tmp/typeflux-gul206/typeflux-api" },
    }).tag,
    "gul206",
  );
});

test("sessions split on idle gaps and summarise their tasks", () => {
  const now = 10_000_000;
  const t = (id, start, extra = {}) =>
    exec(id, {
      task_id: id,
      started_at: start,
      ended_at: start + 100,
      ...extra,
    });
  const tasks = [
    t("a1", 1000),
    t("a2", 2000, { status: "failed", result: { exit_code: 1 } }),
    t("a3", 1000 + 31 * 60 * 1000, {
      status: "timed_out",
      error: { message: "t" },
    }),
    t("b1", 1500, { arguments: { command: "x", cwd: "/work/other" } }),
    t("live", now - 50, {
      arguments: { command: "make", cwd: "/work/live" },
      status: "running",
      ended_at: undefined,
    }),
  ];
  const sessions = groupSessions(tasks, [], undefined, now);
  assert.equal(sessions.length, 4);
  assert.equal(sessions[0].project.name, "live");
  assert.equal(sessions[0].active, true);
  assert.equal(sessions[0].end, now);
  const app = sessions.filter((s) => s.project.name === "app");
  assert.equal(app.length, 2);
  assert.deepEqual(
    app[1].tasks.map((x) => x.task_id),
    ["a2", "a1"],
  );
  assert.equal(app[1].exited, 1);
  assert.equal(app[1].problems, 0);
  assert.equal(app[0].problems, 1);
  assert.equal(app[0].start, 1000 + 31 * 60 * 1000);
});

test("explanations separate what happened, what the agent saw and what to do", () => {
  const exited = explainTask(
    exec("x", { status: "failed", result: { exit_code: 3 } }),
  );
  assert.equal(exited.agentExit, true);
  assert.match(exited.what, /退出码 3/);
  for (const code of [
    "approval_expired",
    "approval_rejected",
    "approval_interrupted",
  ]) {
    const e = explainTask(
      exec("x", { status: "denied", error: { message: "", code } }),
    );
    assert.ok(e.what && e.agent && e.you);
    assert.match(e.agent, new RegExp(code));
  }
  for (const status of [
    "denied",
    "timed_out",
    "unknown",
    "failed",
    "cancelled",
  ])
    assert.ok(
      explainTask(exec("x", { status, error: { message: "boom" } })).what,
    );
  assert.match(
    explainTask(
      exec("x", { status: "timed_out", error: { message: "timed_out" } }),
    ).what,
    /^超过时限后被停止。$/,
  );
  assert.match(
    explainTask(exec("x", { status: "failed", error: { message: "boom" } }))
      .what,
    /boom/,
  );
  assert.equal(explainTask(exec("x")), null);
  assert.equal(explainTask(exec("x", { status: "running" })), null);
  assert.equal(
    problemLabel(
      exec("x", { status: "denied", error: { code: "approval_expired" } }),
    ),
    "确认已过期",
  );
  assert.equal(
    problemLabel(
      exec("x", { status: "denied", error: { code: "approval_rejected" } }),
    ),
    "已拒绝",
  );
  assert.equal(problemLabel({ kind: "sync", status: "timed_out" }), "同步超时");
  assert.equal(problemLabel(exec("x", { status: "unknown" })), "未知");
});

test("presets are recognised exactly and anything else is custom", () => {
  for (const [key, p] of Object.entries(presets))
    assert.equal(
      presetOf({ approval: p.approval, desktop: { ...p.desktop } }),
      key,
    );
  assert.equal(
    presetOf({
      approval: "direct",
      desktop: { observe: "allow", control: "allow", high: "deny" },
    }),
    "custom",
  );
  assert.equal(presetOf({ approval: "direct" }), "custom");
  assert.equal(presetOf(null), "custom");
});
