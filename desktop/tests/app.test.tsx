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
import { statuses, selectTasks, statusMatches } from "../src/model.mjs";

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
    (task) => !args.status || statusMatches(args.status, task.status),
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
function settingsButton() {
  return screen.getByRole("button", { name: "设置", exact: true });
}
function taskList() {
  return within(screen.getByRole("region", { name: "任务列表" }));
}
function detail() {
  return within(screen.getByRole("region", { name: "任务详情" }));
}

async function chooseStatus(
  user: ReturnType<typeof userEvent.setup>,
  status: string,
) {
  await user.selectOptions(
    screen.getByRole("combobox", { name: "更多筛选" }),
    status,
  );
}

test("navigation keeps all nine task states independently usable", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
  for (const status of Object.keys(statuses)) {
    await chooseStatus(user, status);
    await waitFor(() =>
      expect(detail().getByText(`task-${status}`)).toBeTruthy(),
    );
    expect(taskList().getAllByRole("button")).toHaveLength(1);
  }
  // "Needs attention" sums the Macrun-level problem states.
  await user.click(screen.getByRole("button", { name: /^需要关注\s*4$/ }));
  await waitFor(() =>
    expect(taskList().getAllByRole("button")).toHaveLength(4),
  );
  // Type filter is independent of status and reaches the worker query.
  await user.click(screen.getByRole("button", { name: /^全部\s*9$/ }));
  await user.click(screen.getByRole("button", { name: "桌面" }));
  await waitFor(() =>
    expect(taskList().getAllByRole("button")).toHaveLength(1),
  );
  expect(detail().getByText("task-unknown")).toBeTruthy();
  expect(
    bridge.invoke.mock.calls.some(
      ([command, args]) =>
        command === "control" &&
        args.action === "task_list" &&
        args.args.kind === "mcp.call",
    ),
  ).toBe(true);
  await user.click(navigation().getByRole("button", { name: "本机" }));
  expect(screen.getByRole("heading", { level: 1, name: "本机" })).toBeTruthy();
  await user.click(settingsButton());
  expect(screen.getByRole("heading", { level: 1, name: "设置" })).toBeTruthy();
});

test("task search finds command, cwd, sync root, file path and exact task id", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
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
  const receive = screen.getByRole("checkbox", { name: "接收新任务" });
  expect((receive as HTMLInputElement).checked).toBe(true);
  await user.click(receive);
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "pause",
    args: { paused: true },
  });
  expect(screen.getByRole("heading", { name: "command-running" })).toBeTruthy();
  // Paused shows up under "需要你" with a one-click resume.
  expect((receive as HTMLInputElement).checked).toBe(false);
  await user.click(screen.getByRole("button", { name: "恢复接收" }));
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "pause",
    args: { paused: false },
  });
  await user.click(screen.getByRole("button", { name: /^停止全部/ }));
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
  const pause = screen.getByRole("checkbox", { name: "接收新任务" });
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
  await user.click(navigation().getByRole("button", { name: "活动" }));
  await chooseStatus(user, "unknown");
  await waitFor(() => expect(detail().getByText(/不会自动重放/)).toBeTruthy());
  expect(
    detail().queryByRole("button", { name: /取消任务|重试|重新执行|重放/ }),
  ).toBeNull();
  bridge.invoke.mockClear();
  await user.click(detail().getByRole("button", { name: "前往本机核对" }));
  expect(screen.getByRole("heading", { level: 1, name: "本机" })).toBeTruthy();
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
  navigation().getByRole("button", { name: "活动" }).focus();
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
    navigation().getByRole("button", { name: "活动" }),
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
  await user.click(settingsButton());
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
  await user.click(settingsButton());
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
  await user.click(navigation().getByRole("button", { name: "本机" }));
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
  expect(document.activeElement).toBe(tools.querySelector("pre.term"));
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

test("a tray task opens its own detail and clears stale task filters in the main window", async () => {
  const user = userEvent.setup();
  await mount("?tray=1");
  await user.click(screen.getByRole("button", { name: /command-running/ }));
  const open = bridge.invoke.mock.calls.find(
    ([command]) => command === "open_main",
  );
  expect(open).toEqual(["open_main", { route: "tasks:task-running" }]);
  await act(async () => {
    bridge.roots.splice(0).forEach((root) => root.unmount());
  });
  vi.resetModules();
  document.body.innerHTML = '<div id="root"></div>';
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
  await chooseStatus(user, "unknown");
  await user.selectOptions(
    screen.getByRole("combobox", { name: "更多筛选" }),
    "kind:exec.start",
  );
  await user.type(
    screen.getByRole("textbox", { name: "搜索任务" }),
    "no match",
  );
  await emit("navigate", open![1].route);
  await waitFor(() => expect(detail().getByText("task-running")).toBeTruthy());
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
  expect(screen.getByRole("heading", { level: 1, name: "设置" })).toBeTruthy();
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
      screen.getByRole("heading", { level: 1, name: "设置" }),
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
  expect(screen.getByRole("heading", { level: 1, name: "概览" })).toBeTruthy();
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
  await user.click(navigation().getByRole("button", { name: "活动" }));
  await waitFor(() => expect(detail().getByText("history-0000")).toBeTruthy());
  expect(taskList().getAllByRole("button")).toHaveLength(50);
  expect(screen.getByRole("option", { name: /^失败\s*500$/ })).toBeTruthy();
  const pages = within(screen.getByRole("navigation", { name: "任务分页" }));
  expect(pages.getByText("共 1500 条 · 第 1 页")).toBeTruthy();
  await user.click(pages.getByRole("button", { name: "更早" }));
  await waitFor(() => expect(detail().getByText("history-0050")).toBeTruthy());
  expect(taskList().getAllByRole("button")).toHaveLength(50);
  expect(taskList().queryByText("archive command 0")).toBeNull();
  await user.click(pages.getByRole("button", { name: "较新" }));
  await waitFor(() => expect(detail().getByText("history-0000")).toBeTruthy());
  await user.type(
    screen.getByRole("textbox", { name: "搜索任务" }),
    "/ARCHIVES/project-1499",
  );
  await waitFor(() => expect(detail().getByText("history-1499")).toBeTruthy());
  expect(taskList().getAllByRole("button")).toHaveLength(1);
  expect(screen.getByRole("button", { name: /^全部\s*1$/ })).toBeTruthy();
  expect(screen.getByRole("option", { name: /^失败\s*0$/ })).toBeTruthy();
  expect(detail().getByText("tail-1499")).toBeTruthy();
  // The summary can render before the separate detail effect starts.
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("control", {
      action: "task_detail",
      args: { task_id: "history-1499", tail_bytes: 8192 },
    }),
  );
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
  await user.click(navigation().getByRole("button", { name: "活动" }));
  const pages = within(screen.getByRole("navigation", { name: "任务分页" }));
  expect(pages.getByText("共 200 条 · 第 1 页")).toBeTruthy();
  for (let i = 1; i <= 3; i++) {
    await user.click(pages.getByRole("button", { name: "更早" }));
    expect(
      detail().getByText(`history-${String(i * 50).padStart(4, "0")}`),
    ).toBeTruthy();
  }
  expect(
    pages.getByRole("button", { name: "更早" }).hasAttribute("disabled"),
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
  await user.click(settingsButton());
  expect(screen.getByRole("heading", { level: 1, name: "设置" })).toBeTruthy();
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

test("pending actions only disable their own controls and keep emergency stop available", async () => {
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
  const pause = screen.getByRole("checkbox", { name: "接收新任务" });
  const stop = screen.getByRole("button", { name: /^停止全部/ });
  act(() => {
    fireEvent.click(pause);
    fireEvent.click(stop);
  });
  expect(pause.hasAttribute("disabled")).toBe(true);
  await act(async () => {
    finishPause();
  });
  expect(pause.hasAttribute("disabled")).toBe(false);
  expect(stop.hasAttribute("disabled")).toBe(true);
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
  await user.click(navigation().getByRole("button", { name: "本机" }));
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
  await user.click(navigation().getByRole("button", { name: "本机" }));
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
  await user.click(navigation().getByRole("button", { name: "本机" }));
  await user.click(screen.getByRole("button", { name: "查看工具列表" }));
  await user.click(settingsButton());
  await user.click(navigation().getByRole("button", { name: "本机" }));
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
    await user.click(navigation().getByRole("button", { name: "本机" }));
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

test("menu bar puts stop first, hides the server address and uses real switches", async () => {
  const user = userEvent.setup();
  await mount("?tray=1");
  expect(
    screen.getByRole("heading", { name: "需要你确认 1 个请求" }),
  ).toBeTruthy();
  expect(screen.queryByText(/127\.0\.0\.1/)).toBeNull();
  expect(screen.queryByText("最近")).toBeNull();
  // Running work is listed once; the waiting request is only in the prompt.
  const working = within(screen.getByRole("region", { name: "正在进行" }));
  expect(working.queryByText("command-awaiting_approval")).toBeNull();
  expect(working.getByText("command-running")).toBeTruthy();
  await user.click(screen.getByRole("checkbox", { name: "接收新任务" }));
  await user.click(
    screen.getByRole("checkbox", { name: "允许 Agent 操作桌面" }),
  );
  await user.click(screen.getByRole("button", { name: "停止" }));
  expect(
    bridge.invoke.mock.calls
      .filter(([command]) => command === "control")
      .map(([, args]) => [args.action, args.args]),
  ).toEqual([
    ["pause", { paused: true }],
    ["desktop", { enabled: true }],
    ["stop_all", {}],
  ]);
});

test("idle menu bar collapses to status and switches", async () => {
  app.snapshot.tasks = app.snapshot.tasks.filter(
    (task) =>
      !["accepted", "running", "awaiting_approval"].includes(task.status),
  );
  app.snapshot.active_count = 0;
  await mount("?tray=1");
  expect(
    screen.getByRole("heading", { name: "就绪，等待 Agent" }),
  ).toBeTruthy();
  expect(screen.queryByRole("button", { name: "停止" })).toBeNull();
  expect(screen.queryByRole("region", { name: "正在进行" })).toBeNull();
  // Failed (no exit code) and unknown are Macrun problems.
  expect(screen.getByText(/今天 9 个任务 · 2 个问题/)).toBeTruthy();
  // The last finished projects confirm earlier work really ended.
  expect(
    within(screen.getByRole("region", { name: "最近" })).getAllByRole("button"),
  ).toHaveLength(2);
});

test("live page shows a waiting request once and desktop calls without a terminal", async () => {
  app.snapshot.tasks.push({
    task_id: "task-desktop",
    kind: "mcp.call",
    status: "running",
    desktop_tier: "observe",
    arguments: {
      server: "computer",
      tool: "get_window_state",
      arguments: { pid: 1 },
    },
    started_at: Date.now(),
  } as any);
  await mount();
  expect(screen.getAllByText("command-awaiting_approval")).toHaveLength(1);
  const card = screen
    .getByRole("heading", { name: "get_window_state" })
    .closest("article")!;
  expect(within(card as HTMLElement).getByText("操作中")).toBeTruthy();
  expect(
    within(card as HTMLElement).getByText('computer · {"pid":1}'),
  ).toBeTruthy();
  expect(card.querySelector(".term")).toBeNull();
  expect(screen.queryByText("127.0.0.1:7443")).toBeNull();
});

test("a new approval raises one notification when notifications are enabled", async () => {
  app.preferences.notifications = true;
  await mount();
  // The first snapshot only records what is already visible.
  expect(
    bridge.invoke.mock.calls.filter(([command]) => command === "notify_task"),
  ).toHaveLength(0);
  const next = structuredClone(app.snapshot);
  next.tasks.unshift({
    ...next.tasks.find((task) => task.status === "awaiting_approval")!,
    task_id: "task-new-approval",
  });
  await emit("worker-state", next);
  await emit("worker-state", structuredClone(next));
  expect(
    bridge.invoke.mock.calls.filter(([command]) => command === "notify_task"),
  ).toEqual([["notify_task", { status: "awaiting_approval" }]]);
});

test("expired approvals explain that nothing ran", async () => {
  const user = userEvent.setup();
  Object.assign(
    app.snapshot.tasks.find((task) => task.status === "denied")!,
    {
      error: {
        code: "approval_expired",
        message:
          "denied: approval expired after 60 seconds without a local decision",
      },
    },
  );
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
  await chooseStatus(user, "denied");
  await waitFor(() =>
    expect(detail().getByText(/60 秒内没有人处理这个确认请求/)).toBeTruthy(),
  );
});

test("visited pages preserve DOM and their own scroll position", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
  const main = document.querySelector<HTMLElement>("main.content")!;
  const list = screen.getByRole("region", { name: "任务列表" });
  main.scrollTop = 170;
  await user.click(navigation().getByRole("button", { name: "本机" }));
  expect(main.scrollTop).toBe(0);
  main.scrollTop = 80;
  await user.click(navigation().getByRole("button", { name: "活动" }));
  expect(screen.getByRole("region", { name: "任务列表" })).toBe(list);
  expect(main.scrollTop).toBe(170);
  await user.click(navigation().getByRole("button", { name: "本机" }));
  expect(main.scrollTop).toBe(80);
});

test("Cmd+A in a log selects only that log, while page controls remain outside the range", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
  await waitFor(() => expect(detail().getByText(/^output-/)).toBeTruthy());
  const log = document.querySelector<HTMLPreElement>(".v4-detail pre.term")!;
  log.focus();
  fireEvent.keyDown(log, { key: "a", metaKey: true });
  const selection = window.getSelection()!;
  expect(selection.toString()).toBe(log.textContent);
  expect(log.contains(selection.anchorNode)).toBe(true);
  expect(log.contains(selection.focusNode)).toBe(true);
});

test("server filter routes the history query and labels each server independently", async () => {
  const user = userEvent.setup();
  const saved = {
    id: "connection-two",
    name: "Build Server",
    server: "192.0.2.20:7443",
    cert: "/test/two.der",
    token_file: "",
  };
  Object.assign(app.settings, { connections: [saved] });
  app.snapshot.connections = [
    {
      id: "primary",
      name: "Main Server",
      connection: app.snapshot.connection,
      policy: app.snapshot.policy,
    },
    {
      id: saved.id,
      name: saved.name,
      connection: { ...app.snapshot.connection, server: saved.server },
      policy: app.snapshot.policy,
    },
  ];
  await mount();
  await user.click(navigation().getByRole("button", { name: "活动" }));
  await user.click(
    within(screen.getByRole("group", { name: "按服务器筛选" })).getByRole(
      "button",
      { name: saved.name },
    ),
  );
  await waitFor(() =>
    expect(
      bridge.invoke.mock.calls.some(
        ([name, args]) =>
          name === "control" &&
          args.action === "task_list" &&
          args.args.connection_id === saved.id,
      ),
    ).toBe(true),
  );
  await user.click(settingsButton());
  expect(
    within(
      document.querySelector<HTMLElement>(".server-connections")!,
    ).getByText(saved.name),
  ).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "添加服务器" }).hasAttribute("disabled"),
  ).toBe(false);
});

test("adding a server explains active tasks and restores keyboard focus on cancel", async () => {
  const user = userEvent.setup();
  await mount();
  await user.click(settingsButton());
  const trigger = screen.getByRole("button", { name: "添加服务器" });
  await user.click(trigger);
  const dialog = screen.getByRole("dialog", { name: "添加服务器" });
  expect(within(dialog).getByRole("status").textContent).toContain("3 个任务");
  expect(
    within(dialog)
      .getByRole("button", { name: "断开并添加" })
      .hasAttribute("disabled"),
  ).toBe(true);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(trigger);
  expect(
    bridge.invoke.mock.calls.some(([command]) => command === "stop_worker"),
  ).toBe(false);
});

test("idle add waits for shutdown, prevents repeat clicks and opens the add pairing flow", async () => {
  const user = userEvent.setup();
  app.snapshot.active_count = 0;
  const original = bridge.invoke.getMockImplementation()!;
  let finish: () => void = () => {};
  bridge.invoke.mockImplementation((command, args) =>
    command === "stop_worker"
      ? new Promise<void>((resolve) => {
          finish = () => {
            app.worker_running = false;
            resolve();
          };
        })
      : original(command, args),
  );
  await mount();
  await user.click(settingsButton());
  await user.click(screen.getByRole("button", { name: "添加服务器" }));
  const dialog = screen.getByRole("dialog", { name: "添加服务器" });
  await user.click(within(dialog).getByRole("button", { name: "断开并添加" }));
  expect(
    within(dialog)
      .getByRole("button", { name: "正在断开…" })
      .hasAttribute("disabled"),
  ).toBe(true);
  await user.keyboard("{Escape}");
  expect(screen.getByRole("dialog")).toBe(dialog);
  expect(screen.queryByRole("heading", { name: "粘贴配对码" })).toBeNull();
  await act(async () => finish());
  await waitFor(() =>
    expect(screen.getByRole("heading", { name: "粘贴配对码" })).toBeTruthy(),
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(
    bridge.invoke.mock.calls.filter(([command]) => command === "stop_worker"),
  ).toEqual([["stop_worker", { onlyIfIdle: true }]]);
  expect(
    screen.queryByRole("button", { name: "手动填写地址和证书" }),
  ).toBeNull();
});

test("failed idle check leaves the connection and dialog intact, then permits retry", async () => {
  const user = userEvent.setup();
  app.snapshot.active_count = 0;
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation((command, args) =>
    command === "stop_worker"
      ? Promise.reject(new Error("仍有任务运行，请等待任务结束后再添加服务器"))
      : original(command, args),
  );
  await mount();
  await user.click(settingsButton());
  await user.click(screen.getByRole("button", { name: "添加服务器" }));
  const dialog = screen.getByRole("dialog", { name: "添加服务器" });
  await user.click(within(dialog).getByRole("button", { name: "断开并添加" }));
  expect(within(dialog).getByRole("alert").textContent).toContain(
    "仍有任务运行",
  );
  expect(app.worker_running).toBe(true);
  expect(
    within(dialog)
      .getByRole("button", { name: "断开并添加" })
      .hasAttribute("disabled"),
  ).toBe(false);
});

test("stopped add goes straight to pairing without trying to disconnect", async () => {
  const user = userEvent.setup();
  app.worker_running = false;
  await mount();
  await user.click(settingsButton());
  await user.click(screen.getByRole("button", { name: "添加服务器" }));
  expect(screen.getByRole("heading", { name: "粘贴配对码" })).toBeTruthy();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(
    bridge.invoke.mock.calls.some(([command]) => command === "stop_worker"),
  ).toBe(false);
});

test("stopped worker does not show saved multi-server snapshots as online", async () => {
  const user = userEvent.setup();
  app.worker_running = false;
  app.snapshot.connections = ["primary", "extra"].map((id) => ({
    id,
    name: id,
    connection: app.snapshot.connection,
    policy: app.snapshot.policy,
  }));
  app.settings.connections = [
    {
      id: "extra",
      name: "Extra server",
      server: "192.0.2.20:7443",
      cert: "/test/two.der",
      token_file: "",
    },
  ];
  await mount();
  const sidebar = within(screen.getByLabelText("服务器状态"));
  expect(sidebar.getAllByText("未连接")).toHaveLength(2);
  expect(sidebar.queryByText(/ms$/)).toBeNull();
  await user.click(settingsButton());
  const list = within(
    document.querySelector<HTMLElement>(".server-connections")!,
  );
  expect(list.queryByText("已连接")).toBeNull();
  expect(list.getAllByText("未连接")).toHaveLength(2);
});
