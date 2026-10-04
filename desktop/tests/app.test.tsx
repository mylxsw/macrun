// @vitest-environment jsdom
import React from "react";
import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Root } from "react-dom/client";
import type { Snapshot, Task } from "../src/types";
import { statuses, selectTasks } from "../src/model.mjs";

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
let historyRecords: Task[] = [];

function historyPage(args: any) {
  const all = (
    selectTasks(historyRecords, "all", (args.query || "").trim()) as Task[]
  )
    .filter((task) => !args.kind || task.kind === args.kind)
    .sort(
      (a, b) =>
        b.started_at - a.started_at || b.task_id.localeCompare(a.task_id),
    );
  const counts = all.reduce<Record<string, number>>((result, task) => {
    result[task.status] = (result[task.status] || 0) + 1;
    return result;
  }, {});
  const filtered = all.filter(
    (task) =>
      !args.status || args.status === "all" || task.status === args.status,
  );
  const after = filtered.filter(
    (task) =>
      !args.cursor ||
      task.started_at < args.cursor.started_at ||
      (task.started_at === args.cursor.started_at &&
        task.task_id < args.cursor.task_id),
  );
  const rows = after.slice(0, args.limit || 50);
  const last = rows.at(-1)!;
  return structuredClone({
    tasks: rows,
    total: historyRecords.length,
    filtered_total: filtered.length,
    counts,
    next_cursor:
      after.length > rows.length
        ? { started_at: last.started_at, task_id: last.task_id }
        : null,
  });
}

function largeHistory(count = 1500) {
  const now = Date.now();
  historyRecords = Array.from({ length: count }, (_, i) => ({
    task_id: `history-${String(i).padStart(4, "0")}`,
    kind: "exec.start",
    status: i % 3 === 0 ? "failed" : "succeeded",
    started_at: now - i * 1000,
    ended_at: now,
    arguments: {
      command: `archive command ${i}`,
      cwd: `/archives/project-${i}`,
      path: `/files/item-${i}`,
    },
    output_tail: `tail-${i}`,
  }));
  app.snapshot.tasks = historyRecords.slice(
    0,
    200,
  ) as typeof app.snapshot.tasks;
  app.snapshot.total_tasks = count;
  app.snapshot.active_count = 0;
}

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
  historyRecords = app.snapshot.tasks;
  sessionStorage.clear();
  window.history.replaceState(null, "", "/");
  (window as any).__TAURI_INTERNALS__ = {};
  document.body.innerHTML = '<div id="root"></div>';
  bridge.invoke.mockImplementation(async (command, args) => {
    if (command === "app_state") return structuredClone(app);
    if (command === "control" && args.action === "task_list")
      return historyPage(args.args);
    if (command === "control" && args.action === "task_detail") {
      const task = historyRecords.find(
        (task) => task.task_id === args.args.task_id,
      );
      if (!task) throw new Error("task does not exist");
      return structuredClone({ ...task, output: { text: task.output_tail } });
    }
    if (command === "control" && args.action === "pause") {
      app.snapshot.policy.paused = args.args.paused;
    }
    if (command === "control" && args.action === "tools")
      return { session: "test-session", result: { tools: [] } };
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
    await waitFor(() =>
      expect(detail().getByText(`task-${status}`)).toBeTruthy(),
    );
    expect(taskList().getAllByRole("button")).toHaveLength(1);
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
    await waitFor(() =>
      expect(detail().getByText(`task-${status}`)).toBeTruthy(),
    );
    expect(taskList().getAllByRole("button")).toHaveLength(1);
  }
  await user.clear(input);
  await user.type(input, "does-not-exist");
  await waitFor(() =>
    expect(taskList().queryAllByRole("button")).toHaveLength(0),
  );
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
  await user.click(detail().getByRole("button", { name: "前往桌面控制核对" }));
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

test("large task history pages without mounting every row and searches beyond the snapshot", async () => {
  largeHistory();
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "任务" }));
  await waitFor(() => expect(detail().getByText("history-0000")).toBeTruthy());
  expect(taskList().getAllByRole("button")).toHaveLength(50);
  expect(screen.getByRole("button", { name: /^失败\s*500$/ })).toBeTruthy();
  const pages = within(screen.getByRole("navigation", { name: "任务分页" }));
  expect(pages.getByText("共 1500 条 · 第 1 页")).toBeTruthy();
  await user.click(pages.getByRole("button", { name: "下一页" }));
  await waitFor(() => expect(detail().getByText("history-0050")).toBeTruthy());
  expect(taskList().getAllByRole("button")).toHaveLength(50);
  expect(taskList().queryByText("archive command 0")).toBeNull();
  await user.click(pages.getByRole("button", { name: "上一页" }));
  await waitFor(() => expect(detail().getByText("history-0000")).toBeTruthy());
  await user.type(
    screen.getByRole("textbox", { name: "搜索任务" }),
    "/ARCHIVES/project-1499",
  );
  await waitFor(() => expect(detail().getByText("history-1499")).toBeTruthy());
  expect(taskList().getAllByRole("button")).toHaveLength(1);
  expect(screen.getByRole("button", { name: /^全部\s*1$/ })).toBeTruthy();
  expect(screen.getByRole("button", { name: /^失败\s*0$/ })).toBeTruthy();
  expect(detail().getByText("tail-1499")).toBeTruthy();
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "task_detail",
    args: { task_id: "history-1499", tail_bytes: 8192 },
  });
});

test("native navigation loads an old task by id even outside the current page and snapshot", async () => {
  largeHistory();
  await mount();
  await emit("navigate", "tasks:history-1498");
  await waitFor(() => expect(detail().getByText("history-1498")).toBeTruthy());
  expect(detail().getByText("tail-1498")).toBeTruthy();
  expect(taskList().getAllByRole("button")).toHaveLength(50);
  expect(taskList().queryByText("archive command 1498")).toBeNull();
});

test("offline cached history remains pageable and cannot send live task controls", async () => {
  largeHistory();
  const user = userEvent.setup();
  await mount();
  await emit("worker-unavailable");
  bridge.invoke.mockClear();
  await user.click(navigation().getByRole("button", { name: "任务" }));
  const pages = within(screen.getByRole("navigation", { name: "任务分页" }));
  expect(pages.getByText("共 200 条 · 第 1 页")).toBeTruthy();
  for (let i = 1; i <= 3; i++) {
    await user.click(pages.getByRole("button", { name: "下一页" }));
    expect(
      detail().getByText(`history-${String(i * 50).padStart(4, "0")}`),
    ).toBeTruthy();
  }
  expect(
    pages.getByRole("button", { name: "下一页" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(detail().queryByRole("button", { name: "取消任务" })).toBeNull();
  expect(
    bridge.invoke.mock.calls.some(([command]) => command === "control"),
  ).toBe(false);
});

test("startup feedback leaves navigation usable until the starting event clears", async () => {
  const user = userEvent.setup();
  await mount();
  await emit("worker-starting", true);
  expect(
    screen.getByText(/正在启动执行器。如 macOS 弹出钥匙串授权/),
  ).toBeTruthy();
  await user.click(navigation().getByRole("button", { name: "设置与安全" }));
  expect(
    screen.getByRole("heading", { level: 1, name: "设置与安全" }),
  ).toBeTruthy();
  await emit("worker-starting", false);
  expect(
    screen.queryByText(/正在启动执行器。如 macOS 弹出钥匙串授权/),
  ).toBeNull();
});

test("live task directory actions retain the task identity for native validation", async () => {
  const user = userEvent.setup();
  await mount();
  const card = screen
    .getByRole("heading", { name: "command-running" })
    .closest("article")!;
  await user.click(
    within(card).getByRole("button", { name: "在终端打开目录" }),
  );
  expect(bridge.invoke).toHaveBeenCalledWith("open_workspace", {
    root: "/work/running",
    terminal: true,
    taskId: "task-running",
  });
});

test("overlapping actions remain busy until the last action completes", async () => {
  await mount();
  const original = bridge.invoke.getMockImplementation()!;
  let finishPause!: () => void, finishStop!: () => void;
  bridge.invoke.mockImplementation((command, args) => {
    if (command === "control" && args.action === "pause")
      return new Promise<void>((resolve) => {
        finishPause = resolve;
      });
    if (command === "control" && args.action === "stop_all")
      return new Promise<void>((resolve) => {
        finishStop = resolve;
      });
    return original(command, args);
  });
  const pause = screen.getByRole("button", { name: "暂停接收新任务" });
  const stop = screen.getByRole("button", { name: /^全部停止/ });
  act(() => {
    fireEvent.click(pause);
    fireEvent.click(stop);
  });
  expect(pause.hasAttribute("disabled")).toBe(true);
  await act(async () => {
    finishPause();
  });
  expect(pause.hasAttribute("disabled")).toBe(true);
  await act(async () => {
    finishStop();
  });
  expect(pause.hasAttribute("disabled")).toBe(false);
});

test("main tools dialog merges paginated tools by name and stops at the last page", async () => {
  app.snapshot.backends.push({
    name: "computer",
    state: "ready",
    command: "fixture",
    session: "fixture-session",
    tool_count: 0,
  });
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation((command, args) => {
    if (command === "control" && args.action === "tools")
      return Promise.resolve({
        session: "fixture-session",
        result: args.args.cursor
          ? { tools: [{ name: "observe" }, { name: "click" }] }
          : { tools: [{ name: "observe" }], nextCursor: "next" },
      });
    return original(command, args);
  });
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "桌面控制" }));
  await user.click(screen.getByRole("button", { name: "查看工具列表" }));
  const dialog = within(screen.getByRole("dialog", { name: "后端工具列表" }));
  expect(dialog.getByText("已读取 1 个工具")).toBeTruthy();
  await user.click(dialog.getByRole("button", { name: "读取更多工具" }));
  expect(dialog.getByText("已读取 2 个工具")).toBeTruthy();
  expect(dialog.queryByRole("button", { name: "读取更多工具" })).toBeNull();
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "tools",
    args: { server: "computer", cursor: "next" },
  });
});

test("closing the tools dialog while a page loads prevents a late response reopening it", async () => {
  app.snapshot.backends.push({
    name: "computer",
    state: "ready",
    command: "fixture",
    session: "fixture-session",
    tool_count: 0,
  });
  const original = bridge.invoke.getMockImplementation()!;
  let finish!: (value: any) => void;
  bridge.invoke.mockImplementation((command, args) => {
    if (command === "control" && args.action === "tools") {
      if (args.args.cursor)
        return new Promise((resolve) => {
          finish = resolve;
        });
      return Promise.resolve({
        session: "fixture-session",
        result: { tools: [{ name: "observe" }], nextCursor: "next" },
      });
    }
    return original(command, args);
  });
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "桌面控制" }));
  await user.click(screen.getByRole("button", { name: "查看工具列表" }));
  await user.click(screen.getByRole("button", { name: "读取更多工具" }));
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  await act(async () =>
    finish({
      session: "fixture-session",
      result: { tools: [{ name: "click" }] },
    }),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("leaving the desktop page invalidates an initial tool request even after returning", async () => {
  app.snapshot.backends.push({
    name: "computer",
    state: "ready",
    command: "fixture",
  });
  const original = bridge.invoke.getMockImplementation()!;
  let finish!: (value: any) => void;
  bridge.invoke.mockImplementation((command, args) => {
    if (command === "control" && args.action === "tools")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return original(command, args);
  });
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "桌面控制" }));
  await user.click(screen.getByRole("button", { name: "查看工具列表" }));
  await user.click(navigation().getByRole("button", { name: "设置与安全" }));
  await user.click(navigation().getByRole("button", { name: "桌面控制" }));
  await act(async () =>
    finish({
      session: "old-session",
      result: { tools: [{ name: "old-tool" }] },
    }),
  );
  expect(screen.queryByRole("dialog", { name: "后端工具列表" })).toBeNull();
  expect(
    screen
      .getByRole("button", { name: "查看工具列表" })
      .hasAttribute("disabled"),
  ).toBe(false);
  await user.click(screen.getByRole("button", { name: "查看工具列表" }));
  await act(async () =>
    finish({
      session: "new-session",
      result: { tools: [{ name: "new-tool" }] },
    }),
  );
  expect(
    screen.getByRole("dialog", { name: "后端工具列表" }).textContent,
  ).toContain("new-tool");
});

test.each([
  {},
  { session: "", result: { tools: [] } },
  { session: "session", result: { tools: {} } },
  { session: "session", result: { tools: [null] } },
  { session: "session", result: { tools: [{ name: 42 }] } },
  { session: "session", result: { tools: [], nextCursor: {} } },
])(
  "invalid initial tools response stays out of the modal and shows a retryable error: %j",
  async (invalid) => {
    app.snapshot.backends.push({
      name: "computer",
      state: "ready",
      command: "fixture",
    });
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) =>
      command === "control" && args.action === "tools"
        ? Promise.resolve(invalid)
        : original(command, args),
    );
    const user = userEvent.setup();
    await mount();
    await user.click(navigation().getByRole("button", { name: "桌面控制" }));
    await user.click(screen.getByRole("button", { name: "查看工具列表" }));
    expect(screen.queryByRole("dialog", { name: "后端工具列表" })).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain(
      "后端未返回有效工具列表，请重试。",
    );
    expect(
      screen
        .getByRole("button", { name: "查看工具列表" })
        .hasAttribute("disabled"),
    ).toBe(false);
  },
);
