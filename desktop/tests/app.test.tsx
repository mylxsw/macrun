// @vitest-environment jsdom
import React from "react";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Root } from "react-dom/client";
import type { Snapshot } from "../src/types";
import { statuses } from "../src/model.mjs";

const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: any }) => void>(),
  roots: [] as Root[],
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, listener: (event: any) => void) => {
    bridge.listeners.set(name, listener);
    return () => bridge.listeners.delete(name);
  }),
}));
vi.mock("react-dom/client", async (original) => {
  const actual = await original<typeof import("react-dom/client")>();
  return {
    ...actual,
    createRoot: (...args: Parameters<typeof actual.createRoot>) => {
      const root = actual.createRoot(...args);
      bridge.roots.push(root);
      return root;
    },
  };
});

function fixture() {
  const now = Date.now();
  const tasks = Object.keys(statuses).map((status, index) => ({
    task_id: `task-${status}`,
    kind: "exec.start",
    status,
    arguments: {
      command: `command-${status}`,
      cwd: `/work/${status}`,
    } as Record<string, any>,
    started_at: now - (index + 1) * 1000,
    ended_at: ["accepted", "running", "awaiting_approval"].includes(status)
      ? undefined
      : now,
    output_tail: `output-${status}`,
  }));
  Object.assign(
    tasks.find((task) => task.status === "succeeded")!,
    {
      kind: "sync",
      arguments: { command: "upload source", remote_root: "/work/sync-target" },
    },
  );
  Object.assign(
    tasks.find((task) => task.status === "failed")!,
    {
      kind: "file.read",
      arguments: { command: "read artifact", path: "/work/notes/README.md" },
    },
  );
  Object.assign(
    tasks.find((task) => task.status === "unknown")!,
    {
      kind: "mcp.call",
      arguments: {
        server: "computer",
        tool: "computer.click",
        arguments: { x: 412, y: 288 },
      },
      error: { message: "Execution interrupted; effects may have occurred." },
    },
  );
  return {
    preferences: {
      show_overlay: true,
      yield_input: true,
      notifications: false,
      keep_awake: false,
      auto_connect: false,
    },
    worker_running: true,
    settings: {
      server: "127.0.0.1:7443",
      cert: "/test/cert.der",
      token_file: "",
      backend_config: "",
      keychain_account: "test-account",
      certificate_fingerprint: "test-fingerprint",
    },
    data_dir: "/test/macrun",
    legacy_running: false,
    legacy_detected: false,
    autostart: false,
    platform: "macos",
    snapshot: {
      version: "0.2.0",
      protocol: 2,
      connection: {
        state: "connected",
        server: "127.0.0.1:7443",
        since: now - 10000,
        rtt_ms: 2,
      },
      policy: { paused: false, desktop_enabled: false },
      safety: {
        restrict_paths: true,
        roots: ["/work"],
        approval: "risk",
        retention_days: 30,
        yield_until: 0,
      },
      workspaces: [],
      backends: [] as Snapshot["backends"],
      tasks,
      total_tasks: tasks.length,
      active_count: 3,
      today_summary: {
        total: tasks.length,
        succeeded: 1,
        failed: 1,
        unknown: 1,
      },
    },
  };
}
let app = fixture();

beforeEach(() => {
  vi.resetModules();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
  bridge.invoke.mockReset();
  bridge.listeners.clear();
  app = fixture();
  sessionStorage.clear();
  window.history.replaceState(null, "", "/");
  (window as any).__TAURI_INTERNALS__ = {};
  document.body.innerHTML = '<div id="root"></div>';
  bridge.invoke.mockImplementation(async (command, args) => {
    if (command === "app_state") return structuredClone(app);
    if (command === "control" && args.action === "pause") {
      app.snapshot.policy.paused = args.args.paused;
    }
    if (command === "permissions")
      return {
        accessibility: false,
        screen_recording: false,
        graphical_session: true,
        keep_awake: false,
      };
    if (command === "input_status") return { available: false };
    return {};
  });
});
afterEach(async () => {
  await act(async () => {
    bridge.roots.splice(0).forEach((root) => root.unmount());
  });
  document.body.innerHTML = "";
  document.documentElement.className = "";
  delete (window as any).__TAURI_INTERNALS__;
  vi.unstubAllGlobals();
});
async function mount(search = "") {
  window.history.replaceState(null, "", `/${search}`);
  await act(async () => {
    await import("../src/main");
  });
  await waitFor(() => expect(bridge.listeners.has("navigate")).toBe(true));
  if (app.worker_running)
    await emit("worker-state", structuredClone(app.snapshot));
}
async function emit(name: string, payload?: any) {
  await act(async () => {
    bridge.listeners.get(name)?.({ payload });
  });
}
function navigation() {
  return within(screen.getByRole("navigation", { name: "主导航" }));
}
function taskList() {
  return within(screen.getByRole("region", { name: "任务列表" }));
}
function detail() {
  return within(screen.getByRole("region", { name: "任务详情" }));
}

test("navigation keeps all nine task states independently usable", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "任务" }));
  for (const [status, label] of Object.entries(statuses)) {
    await user.click(
      screen.getByRole("button", { name: new RegExp(`^${label}\\s*1$`) }),
    );
    expect(taskList().getAllByRole("button")).toHaveLength(1);
    expect(detail().getByText(`task-${status}`)).toBeTruthy();
  }
  await user.click(navigation().getByRole("button", { name: "桌面控制" }));
  expect(
    screen.getByRole("heading", { level: 1, name: "桌面控制" }),
  ).toBeTruthy();
  await user.click(navigation().getByRole("button", { name: "设置与安全" }));
  expect(
    screen.getByRole("heading", { level: 1, name: "设置与安全" }),
  ).toBeTruthy();
});

test("task search finds command, cwd, sync root, file path and exact task id", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "任务" }));
  const input = screen.getByRole("textbox", { name: "搜索任务" });
  for (const [query, status] of [
    ["command-running", "running"],
    ["/work/accepted", "accepted"],
    ["/WORK/sync-target", "succeeded"],
    ["notes/readme", "failed"],
    ["task-unknown", "unknown"],
  ]) {
    await user.clear(input);
    await user.type(input, query);
    expect(taskList().getAllByRole("button")).toHaveLength(1);
    expect(detail().getByText(`task-${status}`)).toBeTruthy();
  }
  await user.clear(input);
  await user.type(input, "does-not-exist");
  expect(taskList().queryAllByRole("button")).toHaveLength(0);
  expect(detail().queryByRole("button", { name: "打开完整日志" })).toBeNull();
});

test("pause, resume and emergency stop dispatch distinct worker controls", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(screen.getByRole("button", { name: "暂停接收新任务" }));
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "pause",
    args: { paused: true },
  });
  expect(screen.getByRole("heading", { name: "command-running" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "恢复接收" }));
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "pause",
    args: { paused: false },
  });
  await user.click(screen.getByRole("button", { name: /^全部停止/ }));
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "stop_all",
    args: {},
  });
  expect(
    bridge.invoke.mock.calls
      .filter(([command]) => command === "control")
      .map(([, args]) => args.action),
  ).toEqual(["pause", "pause", "stop_all"]);
});

test("worker disconnect disables controls despite a cached snapshot until worker events resume", async () => {
  await mount();
  const pause = screen.getByRole("button", { name: "暂停接收新任务" });
  expect(pause.hasAttribute("disabled")).toBe(false);
  app.worker_running = false;
  await emit("worker-unavailable");
  expect(pause.hasAttribute("disabled")).toBe(true);
  await emit("worker-state", structuredClone(app.snapshot));
  expect(pause.hasAttribute("disabled")).toBe(false);
});

test("unknown results explain possible effects and offer observation without replay", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "任务" }));
  await user.click(screen.getByRole("button", { name: /^未知\s*1$/ }));
  expect(detail().getByText(/不会自动重放/)).toBeTruthy();
  expect(
    detail().queryByRole("button", { name: /取消任务|重试|重新执行|重放/ }),
  ).toBeNull();
  bridge.invoke.mockClear();
  await user.click(
    detail().getByRole("button", { name: "截一张当前屏幕核对" }),
  );
  expect(
    screen.getByRole("heading", { level: 1, name: "桌面控制" }),
  ).toBeTruthy();
  expect(
    bridge.invoke.mock.calls.some(
      ([command, args]) =>
        command === "control" &&
        ["submit", "exec.start", "mcp.call", "observe"].includes(args.action),
    ),
  ).toBe(false);
});

test("quit dialog traps Tab, closes with Escape and never exits implicitly", async () => {
  const user = userEvent.setup();
  await mount();
  navigation().getByRole("button", { name: "任务" }).focus();
  await emit("exit-requested");
  const dialog = within(screen.getByRole("dialog", { name: "退出 Macrun？" }));
  const cancel = dialog.getByRole("button", { name: "继续运行" });
  const confirm = dialog.getByRole("button", { name: "停止并退出" });
  expect(document.activeElement).toBe(cancel);
  await user.tab({ shift: true });
  expect(document.activeElement).toBe(confirm);
  await user.tab();
  expect(document.activeElement).toBe(cancel);
  await emit("exit-requested");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(
    navigation().getByRole("button", { name: "任务" }),
  );
  expect(
    bridge.invoke.mock.calls.some(([command]) => command === "exit_app"),
  ).toBe(false);
});

test("a successful migration stays successful when the following state refresh fails", async () => {
  app.worker_running = false;
  app.legacy_running = true;
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "设置与安全" }));
  const original = bridge.invoke.getMockImplementation()!;
  let failRefresh = false;
  bridge.invoke.mockImplementation(async (command, args) => {
    if (command === "migrate_legacy") {
      app.legacy_running = false;
      failRefresh = true;
      return { backup: "/test/legacy-backup.plist" };
    }
    if (command === "app_state" && failRefresh) {
      failRefresh = false;
      throw new Error("状态读取暂时不可用");
    }
    return original(command, args);
  });
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  await user.click(screen.getByRole("button", { name: "确认迁移" }));
  const dialog = within(screen.getByRole("dialog", { name: "迁移完成" }));
  expect(dialog.getByText("/test/legacy-backup.plist")).toBeTruthy();
  expect(dialog.queryByRole("button", { name: "重试迁移" })).toBeNull();
  expect(screen.getByRole("alert").textContent).toContain(
    "操作已完成，但界面状态刷新失败，请勿重复操作。",
  );
  expect(screen.getByRole("alert").textContent).toContain("状态读取暂时不可用");
  expect(
    bridge.invoke.mock.calls.filter(
      ([command]) => command === "migrate_legacy",
    ),
  ).toHaveLength(1);
});

test("native quit above a migration dialog keeps keyboard focus in the top dialog", async () => {
  app.worker_running = false;
  app.legacy_running = true;
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "设置与安全" }));
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  const migration = within(
    screen.getByRole("dialog", { name: "迁移旧执行器" }),
  );
  const migrationCancel = migration.getByRole("button", { name: "取消" });
  expect(document.activeElement).toBe(migrationCancel);
  await emit("exit-requested");
  const quit = within(screen.getByRole("dialog", { name: "退出 Macrun？" }));
  const quitCancel = quit.getByRole("button", { name: "继续运行" });
  const quitConfirm = quit.getByRole("button", { name: "停止并退出" });
  expect(document.activeElement).toBe(quitCancel);
  await user.tab({ shift: true });
  expect(document.activeElement).toBe(quitConfirm);
  await user.tab();
  expect(document.activeElement).toBe(quitCancel);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog", { name: "退出 Macrun？" })).toBeNull();
  expect(screen.getByRole("dialog", { name: "迁移旧执行器" })).toBeTruthy();
  expect(document.activeElement).toBe(migrationCancel);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(
    bridge.invoke.mock.calls.some(([command]) =>
      ["migrate_legacy", "exit_app"].includes(command),
    ),
  ).toBe(false);
});

test("closing quit restores an underlying tools dialog before its original trigger", async () => {
  app.snapshot.backends.push({
    name: "computer",
    state: "ready",
    command: "test-backend",
    session: "test-session",
    tool_count: 1,
  });
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "桌面控制" }));
  const trigger = screen.getByRole("button", { name: "查看工具列表" });
  await user.click(trigger);
  const tools = screen.getByRole("dialog", { name: "后端工具列表" });
  const toolsClose = within(tools).getByRole("button", { name: "关闭" });
  expect(document.activeElement).toBe(toolsClose);
  await emit("exit-requested");
  const quit = within(screen.getByRole("dialog", { name: "退出 Macrun？" }));
  await user.tab({ shift: true });
  expect(document.activeElement).toBe(
    quit.getByRole("button", { name: "停止并退出" }),
  );
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog", { name: "退出 Macrun？" })).toBeNull();
  expect(document.activeElement).toBe(toolsClose);
  await user.tab();
  expect(document.activeElement).toBe(toolsClose);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

test("a tray task opens its own detail and clears stale task filters in the main window", async () => {
  const user = userEvent.setup();
  await mount("?tray=1");
  await user.click(screen.getByRole("button", { name: /command-denied/ }));
  const open = bridge.invoke.mock.calls.find(
    ([command]) => command === "open_main",
  );
  expect(open).toEqual(["open_main", { route: "tasks:task-denied" }]);
  await act(async () => {
    bridge.roots.splice(0).forEach((root) => root.unmount());
  });
  vi.resetModules();
  document.body.innerHTML = '<div id="root"></div>';
  await mount();
  await user.click(navigation().getByRole("button", { name: "任务" }));
  await user.click(screen.getByRole("button", { name: /^未知\s*1$/ }));
  await user.type(
    screen.getByRole("textbox", { name: "搜索任务" }),
    "no match",
  );
  await emit("navigate", open![1].route);
  expect(detail().getByText("task-denied")).toBeTruthy();
  expect(taskList().getAllByRole("button")).toHaveLength(9);
  expect(
    (screen.getByRole("textbox", { name: "搜索任务" }) as HTMLInputElement)
      .value,
  ).toBe("");
});

test("first launch pairing yields to a native settings navigation", async () => {
  app.settings.server = "";
  app.worker_running = false;
  const user = userEvent.setup();
  await mount();
  expect(screen.getByRole("heading", { name: "粘贴配对码" })).toBeTruthy();
  await emit("navigate", "settings");
  expect(
    screen.getByRole("heading", { level: 1, name: "设置与安全" }),
  ).toBeTruthy();
  expect(screen.queryByRole("heading", { name: "粘贴配对码" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "配对服务器" }));
  expect(screen.getByRole("heading", { name: "粘贴配对码" })).toBeTruthy();
});

test.each([true, false])(
  "legacy configuration takes users to migration without pairing (running=%s)",
  async (running) => {
    app.settings.server = "";
    app.worker_running = false;
    app.legacy_detected = true;
    app.legacy_running = running;
    const user = userEvent.setup();
    await mount();
    expect(
      screen.getByRole("heading", { level: 1, name: "设置与安全" }),
    ).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "粘贴配对码" })).toBeNull();
    const migrate = screen.getByRole("button", { name: "迁移", exact: true });
    expect(
      bridge.invoke.mock.calls.some(
        ([command]) => command === "migrate_legacy" || command === "pair",
      ),
    ).toBe(false);
    await user.click(migrate);
    expect(screen.getByRole("dialog", { name: "迁移旧执行器" })).toBeTruthy();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "迁移", exact: true })).toBe(
      migrate,
    );
    expect(screen.queryByRole("heading", { name: "粘贴配对码" })).toBeNull();
    expect(
      bridge.invoke.mock.calls.some(
        ([command]) => command === "migrate_legacy" || command === "pair",
      ),
    ).toBe(false);
  },
);

test("an existing desktop connection opens the live page even with legacy files present", async () => {
  app.legacy_detected = true;
  await mount();
  expect(screen.getByRole("heading", { level: 1, name: "现场" })).toBeTruthy();
  expect(screen.queryByRole("heading", { name: "粘贴配对码" })).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(
    bridge.invoke.mock.calls.some(
      ([command]) => command === "migrate_legacy" || command === "pair",
    ),
  ).toBe(false);
});
