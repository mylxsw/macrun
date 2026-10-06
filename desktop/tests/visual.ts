/**
 * Browser-only visual fixture. Mounts the actual App through a fake in-memory
 * Tauri bridge; never connects to a worker, server, Keychain or native APIs.
 * This file is not imported by src/ or included in the production Vite entry.
 */
import type { AppState, Snapshot, Task } from "../src/types";

if (!import.meta.env.DEV || (window as any).__TAURI_INTERNALS__) {
  throw new Error(
    "The visual fixture requires a development browser, not Tauri.",
  );
}

const query = new URLSearchParams(location.search);
const overlay = query.has("overlay");
const pairing = query.has("pairing");
const legacy = query.get("legacy");
const legacyDetected = legacy === "running" || legacy === "stopped";
const configured = overlay || (!pairing && !legacyDetected);
const failure = query.get("error");
const fixtureState = query.get("state");
const delayMs = Math.min(5000, Math.max(0, Number(query.get("delay")) || 0));
const requestedTasks = Math.min(
  5000,
  Math.max(0, Number(query.get("tasks")) || 0),
);
const requestedActive = Math.min(
  requestedTasks,
  Math.max(0, Number(query.get("active") ?? 3) || 0),
);
const isActive = (task: Task) =>
  ["accepted", "running", "awaiting_approval"].includes(task.status);
const statuses = [
  "running",
  "awaiting_approval",
  "accepted",
  "succeeded",
  "failed",
  "cancelled",
  "timed_out",
  "unknown",
  "denied",
];
const history: Task[] = Array.from({ length: requestedTasks }, (_, index) => {
  const status =
    index < requestedActive
      ? statuses[index % 3]
      : statuses[3 + ((index - requestedActive) % 6)];
  const task: Task = {
    task_id: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
    kind:
      index % 5 === 0 ? "mcp.call" : index % 7 === 0 ? "sync" : "exec.start",
    status,
    arguments: {
      command: `printf 'fixture-task-${index + 1}'; npm run verify -- --workspace=/tmp/macrun-visual-fixture/project-${index % 20}/packages/long-directory-name`,
      cwd: `/tmp/macrun-visual-fixture/project-${index % 20}`,
      ...(index % 5 === 0
        ? {
            server: "computer",
            tool: index % 2 ? "computer.screenshot" : "computer.click",
            arguments: { x: 412, y: 288 },
          }
        : {}),
      ...(index % 7 === 0
        ? { remote_root: `/tmp/macrun-visual-fixture/mirror-${index % 20}` }
        : {}),
    },
    started_at: Date.now() - index * 1000,
    output_tail: `fixture-task-${index + 1}\nNo real command was executed.\n`,
  };
  if (task.kind === "mcp.call")
    task.desktop_tier = index % 2 ? "observe" : "control";
  if (status === "awaiting_approval")
    task.approval_deadline = Date.now() + 41_000;
  if (!isActive(task)) task.ended_at = task.started_at + 500;
  if (["failed", "timed_out", "unknown"].includes(status))
    task.error = {
      message:
        status === "unknown"
          ? "测试执行被中断，操作可能已生效；请先核对。"
          : "测试任务失败，可查看任务详情。",
    };
  if (task.kind === "sync")
    task.progress = { received: 42, total: 42, bytes: 2048 };
  if (status === "succeeded") task.result = { exit_code: 0 };
  return task;
});
const callbacks = new Map<
  number,
  { callback: (value: any) => void; once: boolean }
>();
const listeners = new Map<number, { event: string; callback: number }>();
let nextId = 1;
const now = Date.now();
const snapshot: Snapshot = {
  version: "0.2.0",
  protocol: 2,
  connection: {
    state: fixtureState === "reconnecting" ? "reconnecting" : "connected",
    server: "203.0.113.10:7443",
    since: now,
    rtt_ms: 38,
  },
  policy: { paused: fixtureState === "paused", desktop_enabled: overlay },
  safety: {
    restrict_paths: true,
    roots: ["/tmp/macrun-visual-fixture"],
    approval: "risk",
    retention_days: 30,
    yield_until: 0,
    desktop: { observe: "allow", control: "allow", high: "allow" },
  },
  allow_rules: [],
  workspaces: requestedTasks
    ? Array.from({ length: 24 }, (_, index) => ({
        root: `/tmp/macrun-visual-fixture/project-${index}`,
        time: now - index * 1000,
        status: index % 6 === 0 ? "failed" : "succeeded",
      }))
    : [],
  backends: requestedTasks
    ? [
        {
          name: "computer",
          state: "ready",
          session: "fixture-session",
          command: "/tmp/macrun-visual-fixture/mock-backend",
          tool_count: 4,
          tiers: {
            "computer.screenshot": "observe",
            "computer.windows": "observe",
            "computer.click": "control",
            "computer.kill_app": "high",
          },
        },
      ]
    : [],
  tasks: [],
  total_tasks: 0,
  active_count: 0,
  today_summary: { total: 0 },
};
if (overlay) {
  history.unshift({
    task_id: "fixture-desktop-action",
    kind: "mcp.call",
    status: "running",
    arguments: {
      server: "computer",
      tool: "computer.click",
      arguments: { x: 412, y: 288 },
    },
    started_at: now - 1000,
  });
}
// Mirrors the worker: summaries drop output and keep only the exit code.
// ?scenario=v4: a realistic day for design review — two servers, two
// projects running, a desktop operation, a waiting approval, finished
// sessions, commands that exited non-zero and an older sync timeout.
if (query.get("scenario") === "v4") {
  const min = 60_000;
  const t = (
    id: string,
    ago: number,
    length: number,
    over: Partial<Task> & { command?: string; cwd?: string },
  ): Task => {
    const { command, cwd, ...rest } = over;
    const task: Task = {
      task_id: `00000000-0000-4000-8000-v4${id.padStart(10, "0")}`,
      kind: "exec.start",
      status: "succeeded",
      arguments: { command, cwd },
      started_at: now - ago,
      ended_at: now - ago + length,
      connection_id: "primary",
      connection_name: "203.0.113.10:7443",
      result: { exit_code: 0 },
      output_tail: "fixture output — no command was executed\n",
      ...rest,
    };
    if (isActive(task)) delete task.ended_at;
    return task;
  };
  const gul207 = "/tmp/typeflux-gul207/typeflux";
  const gul210 = "/tmp/typeflux-gul210/typeflux";
  history.splice(
    0,
    history.length,
    t("1", 356_000, 0, {
      status: "running",
      result: undefined,
      command: `swift build 2>&1 | grep -E "error:|Build complete"; swift build --build-tests 2>&1 | grep -E "error:|Build complete"; swift test --skip-build > /tmp/gul207-final.log 2>&1; echo TEST_EXIT $?; grep -E "Executed [0-9]+ tests" /tmp/gul207-final.log | tail -1`,
      cwd: gul207,
      output_tail:
        "Compiling TypeFlux\nBuild complete! (70.48 secs)\nBuild complete! (35.54 secs)\n",
    }),
    t("2", 9 * min, 1000, {
      kind: "sync",
      command: undefined,
      arguments: { remote_root: gul207 },
      progress: { received: 142, total: 142, bytes: 3_400_000 },
    }),
    t("3", 13 * min, 36_000, {
      command: "swift test --enable-code-coverage",
      cwd: gul207,
      output_tail:
        "Test Suite 'All tests' started\nExecuted 214 tests, with 0 failures (0 unexpected) in 12.408 seconds\n",
    }),
    t("4", 14 * min, 300, {
      status: "failed",
      result: { exit_code: 1 },
      command: "ls .build/out/Products/Debug | head",
      cwd: gul207,
      output_tail: "ls: .build/out/Products/Debug: No such file or directory\n",
    }),
    t("5", 15 * min, 400, {
      command: `P=$PWD; TB="$P/.build/out/Products/Debug"; cp -R "$TB/TypeFlux.app" /tmp/tf.app; codesign --force -s - /tmp/tf.app`,
      cwd: gul207,
    }),
    t("6", 18 * min, 2000, {
      status: "failed",
      result: { exit_code: 1 },
      command: "grep -rn 'legacyHotkey' Sources | head",
      cwd: gul207,
    }),
    t("7", 147_000, 0, {
      status: "running",
      result: undefined,
      command: `ls -d /tmp/typeflux-gul207/typeflux/.build 2>/dev/null && cp -cR /tmp/typeflux-gul207/typeflux/.build ./.build 2>/dev/null; swift build --build-tests 2>&1 | tail -15`,
      cwd: gul210,
      output_tail: "[812/1043] Compiling TypeFluxCore Hotkeys.swift\n",
    }),
    t("8", 2000, 0, {
      kind: "mcp.call",
      status: "running",
      result: undefined,
      connection_id: "dev-box",
      connection_name: "dev-box",
      desktop_tier: "control",
      command: undefined,
      arguments: {
        server: "computer",
        tool: "click",
        arguments: { label: "Run", window: "Xcode — TypeFlux.xcodeproj" },
      },
    }),
    t("9", 19_000, 0, {
      status: "awaiting_approval",
      result: undefined,
      approval_deadline: now + 41_000,
      command:
        "ps aux | grep -i macrun | grep -v grep | head; system_profiler SPDisplaysDataType | grep -i resolution",
      cwd: gul207,
    }),
    ...Array.from({ length: 11 }, (_, i) =>
      t(`a${i}`, 115 * min - i * min, 20_000, {
        command: i === 4 ? "go test ./... 2>&1 | tail -20" : "make lint",
        cwd: "/tmp/typeflux-gul206/typeflux-api",
        ...(i === 2
          ? { status: "failed" as const, result: { exit_code: 2 } }
          : {}),
      }),
    ),
    ...Array.from({ length: 9 }, (_, i) =>
      t(`b${i}`, 145 * min - i * 2 * min, 30_000, {
        command: "swift test --filter Baseline",
        cwd: "/tmp/typeflux-gul206/typeflux-baseline",
      }),
    ),
    ...Array.from({ length: 6 }, (_, i) =>
      t(`d${i}`, 30 * min - i * 3 * min, 1500, {
        kind: "mcp.call",
        connection_id: "dev-box",
        connection_name: "dev-box",
        desktop_tier: i % 2 ? "observe" : "control",
        command: undefined,
        arguments: {
          server: "computer",
          tool: i % 2 ? "get_window_state" : "click",
          arguments: i % 2 ? { pid: 812 } : { x: 812, y: 64 },
        },
      }),
    ),
    t("y1", 26 * 60 * min, 120_000, {
      kind: "sync",
      status: "timed_out",
      result: undefined,
      command: undefined,
      arguments: { remote_root: "/Users/me/Workspace/codes/gul-199-ios-v4" },
      error: { message: "timed_out: sync timeout" },
    }),
  );
  snapshot.workspaces = [
    {
      root: gul207,
      time: now - 9 * min,
      status: "succeeded",
      connection_id: "primary",
    },
    {
      root: gul210,
      time: now - 20 * min,
      status: "succeeded",
      connection_id: "primary",
    },
    {
      root: "/Users/me/Workspace/codes/gul-199-ios-v4",
      time: now - 26 * 60 * min,
      status: "timed_out",
      connection_id: "primary",
      error: { message: "timed_out: sync timeout" },
    },
  ];
  snapshot.backends = [
    {
      name: "computer",
      state: "ready",
      session: "fixture-session",
      command: "/Applications/CuaDriver.app/Contents/MacOS/cua-driver",
      tool_count: 34,
      tiers: {
        get_window_state: "observe",
        screenshot: "observe",
        click: "control",
        type_text: "control",
        kill_app: "high",
      },
    },
  ];
  snapshot.policy.desktop_enabled = true;
  snapshot.safety = {
    restrict_paths: false,
    roots: [],
    approval: "direct",
    retention_days: 30,
    yield_until: 0,
    desktop: { observe: "allow", control: "allow", high: "allow" },
  };
  snapshot.allow_rules = [
    {
      id: "fixture-rule",
      kind: "exec.start",
      scope: "similar",
      program: "system_profiler",
      cwd: "/tmp/typeflux-gul207",
      created_at: now - 3 * min,
      expires_at: now + 12 * min,
    },
  ];
  snapshot.connections = [
    {
      id: "primary",
      name: "203.0.113.10:7443",
      connection: {
        ...snapshot.connection,
        since: now - 16 * min,
        rtt_ms: 232,
      },
      policy: snapshot.policy,
    },
    {
      id: "dev-box",
      name: "dev-box",
      connection: {
        state: "connected",
        server: "198.51.100.7:7443",
        since: now - 130 * min,
        rtt_ms: 41,
      },
      policy: snapshot.policy,
    },
  ];
  snapshot.connection = snapshot.connections[0].connection;
}
function summarize(task: Task): Task {
  const { output_tail: _output, result, ...summary } = task;
  return structuredClone(
    result?.exit_code === undefined
      ? summary
      : { ...summary, result: { exit_code: result.exit_code } },
  );
}
const exitedNonZero = (task: Task) =>
  task.status === "failed" &&
  !task.error &&
  typeof task.result?.exit_code === "number" &&
  task.result.exit_code !== 0;
function syncHistory() {
  history.sort(
    (a, b) => b.started_at - a.started_at || b.task_id.localeCompare(a.task_id),
  );
  const recent = history.slice(0, 200);
  snapshot.tasks = [...recent, ...history.slice(200).filter(isActive)].map(
    summarize,
  );
  snapshot.total_tasks = history.length;
  snapshot.active_count = history.filter(isActive).length;
  const counts: Record<string, number> = {};
  for (const task of history)
    counts[task.status] = (counts[task.status] || 0) + 1;
  Object.assign(snapshot, { task_counts: counts });
  const today = new Date().toDateString();
  const todays = history.filter(
    (task) => new Date(task.started_at).toDateString() === today,
  );
  const summary: Record<string, number> = { total: todays.length };
  for (const task of todays) {
    summary[task.status] = (summary[task.status] || 0) + 1;
    if (exitedNonZero(task)) summary.exited = (summary.exited || 0) + 1;
  }
  snapshot.today_summary = summary;
}
syncHistory();
const app: AppState & { legacy_detected: boolean } = {
  worker_running:
    configured && !["starting", "offline"].includes(fixtureState || ""),
  worker_starting: configured && fixtureState === "starting",
  snapshot: configured && fixtureState !== "starting" ? snapshot : null,
  settings: {
    server: configured ? snapshot.connection.server : "",
    cert: configured ? "/tmp/macrun-visual-fixture/certificate.der" : "",
    token_file: configured ? "/dev/null" : "",
    backend_config: configured ? "/tmp/macrun-visual-fixture/worker.toml" : "",
    keychain_account: configured ? "fixture-only" : "",
    certificate_fingerprint: configured
      ? "4fa19c07-fixture-not-a-real-fingerprint"
      : "",
  },
  preferences: {
    language:
      query.get("language") === "zh-CN"
        ? "zh-CN"
        : query.get("language") === "en"
          ? "en"
          : "system",
    show_overlay: true,
    yield_input: false,
    notifications: false,
    keep_awake: false,
    auto_connect: false,
  },
  data_dir: "/tmp/macrun-visual-fixture",
  legacy_detected: legacyDetected,
  legacy_running: legacy === "running",
  autostart: false,
  platform: "macos",
};
if (query.get("scenario") === "v4")
  app.settings.connections = [
    {
      id: "dev-box",
      name: "dev-box",
      server: "198.51.100.7:7443",
      cert: "/tmp/macrun-visual-fixture/dev-box.der",
      token_file: "/dev/null",
    },
  ];
const calls: { command: string; args: Record<string, any> }[] = [];
let appReads = 0;
function emit(event: string, payload?: unknown) {
  for (const [id, listener] of listeners) {
    if (listener.event !== event) continue;
    const entry = callbacks.get(listener.callback);
    entry?.callback({ event, id, payload: structuredClone(payload) });
    if (entry?.once) callbacks.delete(listener.callback);
  }
}
function update() {
  syncHistory();
  if (!app.worker_running) return;
  emit("worker-state", snapshot);
}
function unregisterListener(_event: string, id: number) {
  const listener = listeners.get(id);
  if (listener) callbacks.delete(listener.callback);
  listeners.delete(id);
}

async function invoke(command: string, args: Record<string, any> = {}) {
  calls.push({ command, args: structuredClone(args) });
  if (command === "plugin:event|listen") {
    const id = nextId++;
    listeners.set(id, { event: args.event, callback: args.handler });
    if (args.event === "worker-state" && app.worker_running)
      setTimeout(update, 0);
    return id;
  }
  if (command === "plugin:event|unlisten")
    return unregisterListener(args.event, args.eventId);
  if (command === "app_state") {
    if (
      failure === "app_state" ||
      (query.has("refresh_error") && appReads++ > 0)
    )
      throw new Error("测试应用状态读取失败");
    return structuredClone(app);
  }
  if (["set_main_mode", "resize_panel"].includes(command)) return null;
  if (delayMs && command !== "start_worker")
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  if (failure === command || (command === "control" && failure === args.action))
    throw new Error(
      `测试操作失败：${command === "control" ? args.action : command}`,
    );
  if (command === "save_settings") {
    app.settings = {
      ...structuredClone(args.settings),
      keychain_account: "fixture-only",
      certificate_fingerprint: "4fa19c07-fixture-not-a-real-fingerprint",
    };
    return null;
  }
  if (command === "save_preferences") {
    app.preferences = structuredClone(args.preferences);
    return null;
  }
  if (command === "autostart") {
    app.autostart = !!args.enabled;
    return null;
  }
  if (command === "stop_worker") {
    if (snapshot.active_count)
      throw new Error("测试任务仍在运行，请先停止任务");
    app.worker_running = false;
    emit("worker-unavailable");
    return null;
  }
  if (command === "migrate_legacy") {
    if (failure === "migration")
      throw new Error("测试迁移失败：原配置已保留，旧服务状态未变化");
    if (!app.legacy_detected) throw new Error("测试夹具未检测到旧版配置");
    app.settings = {
      ...app.settings,
      server: snapshot.connection.server,
      cert: "/tmp/macrun-visual-fixture/migration/certificate.der",
      token_file: "/dev/null",
      backend_config: "/tmp/macrun-visual-fixture/migration/worker.toml",
      keychain_account: "fixture-only",
      certificate_fingerprint: "4fa19c07-fixture-not-a-real-fingerprint",
    };
    app.legacy_detected = false;
    app.legacy_running = false;
    return {
      backup: "/tmp/macrun-visual-fixture/migration/launchagent.plist.bak",
      disabled: "/tmp/macrun-visual-fixture/launchagent.plist.macrun-disabled",
      note: "模拟迁移完成；路径仅用于界面展示，不会创建文件。",
    };
  }
  if (command === "cua_status")
    return {
      state: "ready",
      version: "Cua Driver · UI fixture",
      configured: true,
      permissions: { accessibility: true, screen_recording: true },
    };
  if (command === "grant_cua_permissions")
    return "fixture permission verification";
  if (command === "pair") {
    if (
      failure === "pair" ||
      !String(args.uri).startsWith("macrun://fixture/")
    ) {
      throw new Error(
        "测试配对失败：请使用 macrun://fixture/pair?code=UI-REVIEW",
      );
    }
    app.settings = {
      ...app.settings,
      server: snapshot.connection.server,
      cert: "/tmp/macrun-visual-fixture/certificate.der",
      keychain_account: "fixture-only",
      certificate_fingerprint: "4fa19c07-fixture-not-a-real-fingerprint",
    };
    return {
      fingerprint: app.settings.certificate_fingerprint,
      protocol: 2,
      credentials: "keychain",
      connection_id: "primary",
    };
  }
  if (command === "start_worker") {
    if (app.worker_starting) throw new Error("测试执行器正在启动，请等待授权");
    if (app.legacy_running)
      throw new Error("测试旧执行器仍在运行，请先完成迁移");
    if (!app.settings.server) throw new Error("测试连接尚未配置");
    app.worker_starting = true;
    emit("worker-starting", true);
    try {
      if (delayMs) await new Promise((resolve) => setTimeout(resolve, delayMs));
      if (failure === "start") throw new Error("测试执行器启动失败");
      app.worker_running = true;
      app.snapshot = snapshot;
      queueMicrotask(update);
      return null;
    } finally {
      app.worker_starting = false;
      emit("worker-starting", false);
    }
  }
  if (command === "connection_check") {
    return {
      checks: ["UDP 7443 可达", "证书指纹一致", "令牌认证", "协议版本一致"].map(
        (name, index) => ({
          name,
          ok: failure !== "connection" || index !== 0,
        }),
      ),
      error: failure === "connection" ? "测试连接失败：UDP 无响应" : null,
    };
  }
  if (command === "control") {
    const payload = args.args || {};
    if (args.action.startsWith("metrics_")) {
      const metrics = (task: Task, index: number) => ({
        schema_version: 1,
        origin: "worker",
        wall_ms: 200 + index * 31,
        complete: !isActive(task),
        phases: [
          {
            name: task.kind === "sync" ? "receive_install" : "process_run",
            wall_ms: 120 + index * 20,
            count: 1,
            max_ms: 120 + index * 20,
          },
          { name: "hash", wall_ms: 20, count: 3, max_ms: 10 },
        ],
        bytes: {
          payload: task.kind === "sync" ? 1048576 + index * 4096 : 0,
          declared_outputs: 1000000,
          artifacts: 4096,
          stored_log: 320,
        },
        files: { changed: 42, completed: 42 },
        outputs: [
          { name: "build/MyApp.zip", bytes: 1000000, status: "complete" },
        ],
        samples: [
          { offset_ms: 10, payload_bytes: 0 },
          { offset_ms: 1010, payload_bytes: 500000 },
          { offset_ms: 2010, payload_bytes: 1000000 },
        ],
        project_id: "fixture-project",
        project_name: "Macrun 测试项目",
        first_payload_offset_ms: 10,
        last_payload_offset_ms: 2010,
        transport: "quic",
        rtt_ms: 12,
      });
      const rows = history
        .filter(
          (t) =>
            (!payload.kind || payload.kind === t.kind) &&
            (!payload.status || payload.status === t.status),
        )
        .map((t, i) => ({
          ...t,
          connection_id: "primary",
          connection_name: "测试服务器",
          metrics: metrics(t, i),
        }));
      if (args.action === "metrics_detail")
        return rows.find((t) => t.task_id === payload.task_id);
      const counts: Record<string, number> = {};
      rows.forEach((t) => (counts[t.status] = (counts[t.status] || 0) + 1));
      const filtered = rows
        .filter(
          (t) => !payload.cursor || t.started_at < payload.cursor.started_at,
        )
        .slice(0, (payload.limit || 50) + 1);
      const more = filtered.length > (payload.limit || 50);
      filtered.splice(payload.limit || 50);
      const last = filtered.at(-1);
      return {
        as_of: Date.now(),
        total: rows.length,
        measured: rows.length,
        counts,
        success_denominator:
          rows.length - (counts.cancelled || 0) - (counts.denied || 0),
        latency: { n: rows.length, p50_ms: 300, p95_ms: 1800, p99_ms: 2200 },
        bytes: { payload: 23000000, artifacts: 4096 },
        phases_ms: {},
        operations: filtered,
        next_cursor:
          more && last
            ? {
                started_at: last.started_at,
                task_id: last.task_id,
                connection_id: "primary",
              }
            : null,
        series: Array.from({ length: 24 }, (_, i) => ({
          time: Date.now() - i * 3600000,
          count: i,
          p50_ms: 200 + i * 5,
          p95_ms: i % 7 === 0 ? null : 500 + i * 17,
          payload_bytes: i * 15000,
        })),
        projects: { "fixture-project": "Macrun 测试项目 · 测试服务器" },
        errors: [],
      };
    }
    if (args.action === "task_list") {
      const text = String(payload.query || "")
        .trim()
        .toLowerCase();
      const matches = history.filter(
        (task) =>
          (payload.kind == null || payload.kind === task.kind) &&
          [
            task.task_id,
            task.kind,
            task.arguments.command,
            task.arguments.tool,
            task.arguments.cwd,
            task.arguments.remote_root,
            task.arguments.path,
          ]
            .filter((part) => typeof part === "string")
            .join(" ")
            .toLowerCase()
            .includes(text),
      );
      const counts: Record<string, number> = {};
      for (const task of matches) {
        counts[task.status] = (counts[task.status] || 0) + 1;
        if (exitedNonZero(task)) counts.exited = (counts.exited || 0) + 1;
      }
      const groups: Record<string, string[]> = {
        active: ["accepted", "running", "awaiting_approval"],
        attention: ["failed", "timed_out", "unknown", "denied"],
      };
      const filtered = matches.filter(
        (task) =>
          !payload.status ||
          payload.status === "all" ||
          (groups[payload.status]
            ? groups[payload.status].includes(task.status) &&
              !(payload.status === "attention" && exitedNonZero(task))
            : task.status === payload.status),
      );
      const cursor = payload.cursor;
      const remaining = filtered.filter(
        (task) =>
          !cursor ||
          task.started_at < cursor.started_at ||
          (task.started_at === cursor.started_at &&
            task.task_id < cursor.task_id),
      );
      const limit = Math.max(
        1,
        Math.min(100, payload.limit === undefined ? 50 : Number(payload.limit)),
      );
      const page = remaining.slice(0, limit),
        last = page.at(-1);
      return {
        tasks: page.map(summarize),
        total: history.length,
        filtered_total: filtered.length,
        counts,
        next_cursor:
          remaining.length > page.length && last
            ? { started_at: last.started_at, task_id: last.task_id }
            : null,
      };
    }
    if (args.action === "snapshot") return structuredClone(snapshot);
    if (args.action === "pause") snapshot.policy.paused = !!payload.paused;
    else if (args.action === "safety")
      snapshot.safety = structuredClone(payload);
    else if (args.action === "yield")
      snapshot.safety.yield_until = Date.now() + 10000;
    else if (args.action === "desktop")
      snapshot.policy.desktop_enabled = !!args.args.enabled;
    else if (args.action === "self_test") {
      const task: Task = {
        task_id: `fixture-self-test-${history.length}`,
        kind: "exec.start",
        status: "succeeded",
        arguments: { command: "hostname", cwd: "/tmp/macrun-visual-fixture" },
        started_at: now,
        ended_at: now + 10,
        result: { exit_code: 0 },
        output_tail: "visual-fixture\n",
      };
      history.unshift(task);
      update();
      return { task_id: task.task_id, status: "accepted" };
    } else if (args.action === "task_detail") {
      const task = history.find((task) => task.task_id === args.args.task_id);
      if (!task) throw new Error("测试任务不存在");
      const detail: any = structuredClone(task);
      if (task.output_tail !== undefined) {
        const output = new TextEncoder().encode(task.output_tail);
        const length = Math.max(
          1,
          Math.min(65536, Number(payload.tail_bytes) || 8192),
        );
        const offset = Math.max(0, output.length - length);
        detail.output = {
          text: new TextDecoder().decode(output.slice(offset)),
          size: output.length,
          offset,
          next_offset: output.length,
          eof: true,
        };
        delete detail.output_tail;
      }
      return detail;
    } else if (args.action === "stop_all") {
      snapshot.policy = { paused: true, desktop_enabled: false };
      for (const task of history.filter(isActive)) {
        task.status = "cancelled";
        task.ended_at = Date.now();
      }
    } else if (args.action === "revoke_rule") {
      snapshot.allow_rules = (snapshot.allow_rules || []).filter(
        (rule) => rule.id !== payload.rule_id,
      );
    } else if (["cancel", "approve"].includes(args.action)) {
      const task = history.find((task) => task.task_id === payload.task_id);
      if (!task) throw new Error("测试任务不存在");
      if (
        args.action === "approve" &&
        payload.allow &&
        payload.scope !== "once"
      )
        snapshot.allow_rules = [
          ...(snapshot.allow_rules || []),
          {
            id: `fixture-rule-${task.task_id}`,
            kind: task.kind,
            scope: payload.scope,
            program: String(task.arguments.command || "").split(/\s+/)[0],
            cwd: task.arguments.cwd,
            server: task.arguments.server,
            tool: payload.scope === "similar" ? task.arguments.tool : null,
            tier: payload.scope === "session" ? task.desktop_tier : null,
            created_at: Date.now(),
            expires_at:
              payload.scope === "similar" ? Date.now() + 15 * 60_000 : null,
          },
        ];
      task.status =
        args.action === "approve" && payload.allow
          ? "succeeded"
          : args.action === "approve"
            ? "denied"
            : "cancelled";
      task.ended_at = Date.now();
    } else if (args.action === "tools") {
      return {
        session: "fixture-session",
        result: {
          tools: [
            {
              name: "computer.screenshot",
              description: "测试观察工具",
              inputSchema: { type: "object", properties: {} },
            },
            {
              name: "computer.click",
              description: "测试点击工具",
              inputSchema: {
                type: "object",
                properties: { x: { type: "number" }, y: { type: "number" } },
              },
            },
          ],
        },
      };
    } else if (args.action === "restart_backend") {
      return { ok: true };
    } else if (args.action === "observe") {
      const task: Task = {
        task_id: `fixture-observe-${history.length}`,
        kind: "mcp.call",
        status: "succeeded",
        arguments: {
          server: payload.server,
          tool: payload.tool,
          local_observation: true,
        },
        started_at: Date.now(),
        ended_at: Date.now(),
      };
      history.unshift(task);
      update();
      return { task_id: task.task_id, status: "accepted" };
    } else throw new Error(`视觉夹具未实现控制：${args.action}`);
    update();
    return {};
  }
  if (command === "open_main") {
    emit("navigate", args.route);
    return null;
  }
  if (command === "request_quit") {
    emit("exit-requested");
    return null;
  }
  if (command === "exit_app") {
    document.documentElement.dataset.fixtureExitRequested = "true";
    return null;
  }
  if (command === "permissions")
    return {
      accessibility: query.get("permissions") === "granted",
      screen_recording: query.get("permissions") === "granted",
      graphical_session: true,
      keep_awake: false,
    };
  if (command === "input_status") return { available: false };
  if (command === "backend_config")
    return "[mcp.computer]\ncommand = '/tmp/macrun-visual-fixture/mock-backend'\n";
  if (command === "save_backends") return null;
  if (
    [
      "open_log",
      "diagnostics",
      "open_workspace",
      "open_permission",
      "notify_task",
    ].includes(command)
  ) {
    document.documentElement.dataset.fixtureLastAction = command;
    return command === "diagnostics"
      ? "/tmp/macrun-visual-fixture/diagnostics.json"
      : null;
  }
  throw new Error(`视觉夹具未实现操作：${command}；不会访问真实系统。`);
}

(window as any).__TAURI_INTERNALS__ = {
  invoke,
  transformCallback(callback: (value: any) => void, once = false) {
    const id = nextId++;
    callbacks.set(id, { callback, once });
    return id;
  },
  unregisterCallback(id: number) {
    callbacks.delete(id);
  },
};
(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener };
(window as any).__MACRUN_VISUAL_FIXTURE__ = { calls, app, emit };

await import("../src/main.tsx");
const frame = () =>
  new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
await frame();
await frame();
await document.fonts.ready;
// ?page=tasks|desktop|settings opens a page the way a native route does.
const page = query.get("page");
if (page) {
  await new Promise((resolve) => setTimeout(resolve, 200));
  emit("navigate", page);
  await frame();
}
await frame();
document.documentElement.dataset.visualReady = "true";
