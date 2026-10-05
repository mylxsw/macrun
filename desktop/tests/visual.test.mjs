import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Exercise the browser-only bridge in memory; never mount an app or native API.
const source = fs
  .readFileSync(new URL("./visual.ts", import.meta.url), "utf8")
  .split('await import("../src/main.tsx");')[0]
  .replace("import.meta.env.DEV", "true");
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.CommonJS,
  },
}).outputText;
function load(search = "") {
  const context = vm.createContext({
    exports: {},
    window: {},
    document: { documentElement: { dataset: {} } },
    location: { search },
    URLSearchParams,
    structuredClone,
    setTimeout,
    queueMicrotask,
    TextEncoder,
    TextDecoder,
  });
  new vm.Script(compiled).runInContext(context);
  return {
    ...context.window.__MACRUN_VISUAL_FIXTURE__,
    invoke: context.window.__TAURI_INTERNALS__.invoke,
  };
}
const control = (fixture, action, args = {}) =>
  fixture.invoke("control", { action, args });

test("visual fixture preserves configured, pairing, and both migration entry states", async () => {
  assert.ok(load().app.settings.server);
  assert.equal(load("?pairing=1").app.settings.server, "");
  for (const state of ["running", "stopped"]) {
    const fixture = load(`?legacy=${state}`);
    assert.equal(fixture.app.legacy_detected, true);
    assert.equal(fixture.app.legacy_running, state === "running");
    const result = await fixture.invoke("migrate_legacy");
    assert.match(result.backup, /^\/tmp\/macrun-visual-fixture\//);
    assert.equal(fixture.app.legacy_detected, false);
    assert.equal(fixture.app.worker_running, false);
    await fixture.invoke("start_worker");
    assert.equal(fixture.app.worker_running, true);
  }
});

test("visual history pages all 1500 records without duplicate ids or full detail payloads", async () => {
  const fixture = load("?tasks=1500");
  assert.equal(fixture.app.snapshot.tasks.length, 200);
  assert.equal(fixture.app.snapshot.active_count, 3);
  const seen = new Set();
  let cursor;
  do {
    const page = await control(fixture, "task_list", { cursor, limit: 100 });
    assert.equal(page.total, 1500);
    assert.equal(page.filtered_total, 1500);
    for (const task of page.tasks) {
      assert.equal(seen.has(task.task_id), false);
      seen.add(task.task_id);
      // Like the worker, rows keep only the exit code, never tool results.
      assert.ok(
        task.result === undefined ||
          Object.keys(task.result).join() === "exit_code",
      );
      assert.equal(task.output_tail, undefined);
    }
    cursor = page.next_cursor;
  } while (cursor);
  assert.equal(seen.size, 1500);
  const last = await control(fixture, "task_list", {
    query: "FIXTURE-TASK-1500",
  });
  assert.equal(last.tasks.length, 1);
  assert.equal(last.tasks[0].task_id, "00000000-0000-4000-8000-000000001500");
  const detail = await control(fixture, "task_detail", {
    task_id: last.tasks[0].task_id,
  });
  assert.match(detail.output.text, /fixture-task-1500/);
});

test("visual status and kind filters keep counts before the status filter and cap page size", async () => {
  const fixture = load("?tasks=1500");
  const all = await control(fixture, "task_list", {
    kind: "mcp.call",
    limit: 500,
  });
  assert.equal(all.tasks.length, 100);
  const failed = await control(fixture, "task_list", {
    kind: "mcp.call",
    status: "failed",
  });
  assert.equal(failed.filtered_total, all.counts.failed);
  assert.equal(JSON.stringify(failed.counts), JSON.stringify(all.counts));
  assert.ok(
    failed.tasks.every(
      (task) => task.kind === "mcp.call" && task.status === "failed",
    ),
  );
});

test("visual emergency stop changes only active tasks and preserves completed history", async () => {
  const fixture = load("?tasks=1500");
  const before = await control(fixture, "task_list");
  await control(fixture, "stop_all");
  const after = await control(fixture, "task_list");
  assert.equal(after.counts.succeeded, before.counts.succeeded);
  assert.equal(after.counts.failed, before.counts.failed);
  assert.equal(after.counts.cancelled, before.counts.cancelled + 3);
  assert.equal(fixture.app.snapshot.active_count, 0);
  assert.equal(fixture.app.snapshot.policy.paused, true);
});

test("visual migration failure preserves state and delayed startup announces pending and rejects duplicates", async () => {
  const failed = load("?legacy=running&error=migration");
  const before = JSON.stringify(failed.app);
  await assert.rejects(failed.invoke("migrate_legacy"), /测试迁移失败/);
  assert.equal(JSON.stringify(failed.app), before);
  const fixture = load("?state=offline&delay=30&error=start");
  const attempt = fixture.invoke("start_worker");
  assert.equal(fixture.app.worker_starting, true);
  await assert.rejects(fixture.invoke("start_worker"), /正在启动/);
  await assert.rejects(attempt, /测试执行器启动失败/);
  assert.equal(fixture.app.worker_starting, false);
  assert.equal(fixture.app.worker_running, false);
});

test("visual refresh failure occurs after the initial state and named read errors remain visible", async () => {
  const fixture = load("?refresh_error=1");
  assert.ok(await fixture.invoke("app_state"));
  await assert.rejects(fixture.invoke("app_state"), /读取失败/);
  await assert.rejects(
    control(load("?tasks=1500&error=task_list"), "task_list"),
    /task_list/,
  );
});

test("visual snapshot retains active tasks beyond its recent history window", () => {
  const fixture = load("?tasks=1500&active=250");
  assert.equal(fixture.app.snapshot.tasks.length, 250);
  assert.equal(fixture.app.snapshot.active_count, 250);
  assert.equal(fixture.app.snapshot.total_tasks, 1500);
});

test("visual v4 scenario separates non-zero exits from Macrun problems", async () => {
  const fixture = load("?scenario=v4");
  const summary = fixture.app.snapshot.today_summary;
  assert.ok(summary.exited >= 2);
  const attention = await control(fixture, "task_list", {
    status: "attention",
    limit: 100,
  });
  assert.ok(
    attention.tasks.every(
      (task) => !(task.status === "failed" && task.result?.exit_code),
    ),
  );
  const all = await control(fixture, "task_list", { limit: 100 });
  assert.equal(all.counts.exited, summary.exited);
  assert.equal(fixture.app.settings.connections.length, 1);
  assert.equal(fixture.app.snapshot.connections.length, 2);
});
