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
const failure = query.get("error");
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
    state: "connected",
    server: "203.0.113.10:7443",
    since: now,
    rtt_ms: 38,
  },
  policy: { paused: false, desktop_enabled: overlay },
  safety: {
    restrict_paths: true,
    roots: ["/tmp/macrun-visual-fixture"],
    approval: "risk",
    retention_days: 30,
    yield_until: 0,
  },
  workspaces: [],
  backends: [],
  tasks: [],
  total_tasks: 0,
  active_count: 0,
  today_summary: { total: 0 },
};
if (overlay) {
  snapshot.tasks.push({
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
  snapshot.active_count = 1;
  snapshot.total_tasks = 1;
}
const app: AppState = {
  worker_running: overlay,
  snapshot: overlay ? snapshot : null,
  settings: {
    server: overlay ? snapshot.connection.server : "",
    cert: "",
    token_file: "",
    backend_config: "",
  },
  preferences: {
    show_overlay: true,
    yield_input: false,
    notifications: false,
    keep_awake: false,
    auto_connect: false,
  },
  data_dir: "/tmp/macrun-visual-fixture",
  legacy_running: false,
  autostart: false,
  platform: "macos",
};
const calls: { command: string; args: Record<string, any> }[] = [];
function emit(event: string, payload?: unknown) {
  for (const [id, listener] of listeners) {
    if (listener.event !== event) continue;
    const entry = callbacks.get(listener.callback);
    entry?.callback({ event, id, payload: structuredClone(payload) });
    if (entry?.once) callbacks.delete(listener.callback);
  }
}
function update() {
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
  if (command === "app_state") return structuredClone(app);
  if (["set_main_mode", "resize_panel"].includes(command)) return null;
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
    };
  }
  if (command === "start_worker") {
    if (failure === "start") throw new Error("测试执行器启动失败");
    app.worker_running = true;
    app.snapshot = snapshot;
    queueMicrotask(update);
    return null;
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
    if (args.action === "desktop")
      snapshot.policy.desktop_enabled = !!args.args.enabled;
    else if (args.action === "self_test") {
      const task: Task = {
        task_id: "fixture-self-test",
        kind: "exec.start",
        status: "succeeded",
        arguments: { command: "hostname", cwd: "/tmp/macrun-visual-fixture" },
        started_at: now,
        ended_at: now + 10,
        result: { exit_code: 0 },
        output_tail: "visual-fixture\n",
      };
      snapshot.tasks = [task];
      snapshot.total_tasks = 1;
      update();
      return { task_id: task.task_id };
    } else if (args.action === "task_detail") {
      const task = snapshot.tasks.find(
        (task) => task.task_id === args.args.task_id,
      );
      if (!task) throw new Error("测试任务不存在");
      return structuredClone(task);
    } else if (args.action === "stop_all") {
      snapshot.policy = { paused: true, desktop_enabled: false };
      snapshot.tasks = snapshot.tasks.map((task) => ({
        ...task,
        status: "cancelled",
        ended_at: Date.now(),
      }));
      snapshot.active_count = 0;
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
      accessibility: false,
      screen_recording: false,
      graphical_session: true,
      keep_awake: false,
    };
  if (command === "input_status") return { available: false };
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
await frame();
document.documentElement.dataset.visualReady = "true";
