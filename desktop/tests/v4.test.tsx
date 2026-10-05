// @vitest-environment jsdom
import React from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Overview, StepMark } from "../src/Overview";
import { Access } from "../src/Access";
import { ThisMac } from "../src/ThisMac";
import { desktopChecks, health, healthLine } from "../src/health";
import {
  clock,
  dayLabel,
  lastLine,
  serverViews,
  span,
  stopwatch,
  taskDuration,
} from "../src/format";
import type { AppState, Snapshot, Task } from "../src/types";

beforeEach(() => localStorage.clear());
afterEach(cleanup);

const now = Date.now();
const task = (id: string, over: Partial<Task> = {}): Task => ({
  task_id: id,
  kind: "exec.start",
  status: "succeeded",
  arguments: { command: `cmd-${id}`, cwd: "/tmp/typeflux-gul207/typeflux" },
  started_at: now - 60_000,
  ended_at: now - 50_000,
  result: { exit_code: 0 },
  ...over,
});

function makeSnapshot(tasks: Task[], over: Partial<Snapshot> = {}): Snapshot {
  return {
    version: "0.2.0",
    protocol: 2,
    connection: {
      state: "connected",
      server: "203.0.113.10:7443",
      since: now - 600_000,
      rtt_ms: 30,
    },
    policy: { paused: false, desktop_enabled: true },
    safety: {
      restrict_paths: false,
      roots: [],
      approval: "direct",
      retention_days: 30,
      yield_until: 0,
      desktop: { observe: "allow", control: "allow", high: "allow" },
    },
    workspaces: [],
    backends: [],
    tasks,
    total_tasks: tasks.length,
    active_count: tasks.filter((t) =>
      ["accepted", "running", "awaiting_approval"].includes(t.status),
    ).length,
    today_summary: { total: tasks.length },
    ...over,
  };
}

const app: AppState = {
  preferences: {
    show_overlay: true,
    yield_input: false,
    notifications: false,
    keep_awake: true,
    auto_connect: true,
  },
  worker_running: true,
  snapshot: null,
  settings: {
    server: "203.0.113.10:7443",
    cert: "",
    token_file: "",
    backend_config: "",
    certificate_fingerprint: "fp-123",
  },
  data_dir: "/tmp",
  legacy_running: false,
  autostart: false,
  platform: "macos",
};

test("format helpers produce compact, human readable times", () => {
  expect(stopwatch(356_000)).toBe("5:56");
  expect(stopwatch(3_723_000)).toBe("1:02:03");
  expect(stopwatch(-5)).toBe("0:00");
  expect(span(20_000)).toBe("不到 1 分钟");
  expect(span(18 * 60_000)).toBe("18 分钟");
  expect(span(3 * 3_600_000)).toBe("3 小时");
  expect(span(50 * 3_600_000)).toBe("2 天");
  expect(dayLabel(now, now)).toBe("今天");
  expect(dayLabel(now - 86_400_000, now)).toBe("昨天");
  expect(dayLabel(new Date(2026, 9, 3, 12).getTime(), now)).toBe("10月3日");
  expect(lastLine("a\nBuild complete!\n\n  ")).toBe("Build complete!");
  expect(lastLine()).toBe("");
  expect(clock(new Date(2026, 0, 1, 9, 5).getTime())).toBe("09:05");
  expect(taskDuration(task("a"))).toBe("0:10");
  expect(taskDuration(task("u", { status: "unknown" }))).toBe("—");
});

test("server labels hide addresses and keep readable names", () => {
  const snapshot = makeSnapshot([], {
    connections: [
      {
        id: "primary",
        name: "203.0.113.10:7443",
        connection: makeSnapshot([]).connection,
        policy: { paused: false, desktop_enabled: true },
      },
      {
        id: "b",
        name: "10.0.0.2:7443",
        connection: {
          state: "reconnecting",
          server: "10.0.0.2:7443",
          since: 0,
        },
        policy: { paused: false, desktop_enabled: true },
      },
    ],
  });
  const settings = {
    server: "203.0.113.10:7443",
    connections: [
      {
        id: "b",
        name: "10.0.0.2:7443",
        server: "10.0.0.2:7443",
        cert: "",
        token_file: "",
      },
      {
        id: "c",
        name: "Build box",
        server: "build:7443",
        cert: "",
        token_file: "",
      },
    ],
  };
  const views = serverViews(settings, snapshot, true);
  expect(views.map((v) => v.label)).toEqual([
    "主服务器",
    "服务器 2",
    "Build box",
  ]);
  expect(views[0].connection?.state).toBe("connected");
  expect(views[1].connection?.state).toBe("reconnecting");
  expect(views[2].connection).toBeUndefined();
  // A stopped worker's last snapshot is not presented as live.
  expect(serverViews(settings, snapshot, false)[0].connection).toBeUndefined();
  // Without saved settings, the snapshot's own list is used.
  expect(serverViews(undefined, snapshot, true)).toHaveLength(2);
  expect(serverViews(undefined, null, true)).toEqual([]);
});

test("desktop checks only claim what Macrun can observe", () => {
  const empty = desktopChecks(makeSnapshot([]), null);
  expect(empty.map((c) => [c.key, c.ok])).toEqual([
    ["backend", false],
    ["session", null],
    ["enabled", true],
  ]);
  const backend = {
    name: "computer",
    state: "not_started",
    command: "cua",
  };
  const unverified = desktopChecks(makeSnapshot([], { backends: [backend] }), {
    graphical_session: false,
  });
  expect(unverified.find((c) => c.key === "backend")?.detail).toBe(
    "computer · 首次调用时启动",
  );
  expect(unverified.find((c) => c.key === "verified")?.ok).toBe(false);
  expect(unverified.find((c) => c.key === "session")?.ok).toBe(false);
  const failedCall = task("bad", { kind: "mcp.call", status: "unknown" });
  expect(
    desktopChecks(
      makeSnapshot([failedCall], { backends: [backend] }),
      null,
    ).find((c) => c.key === "verified")?.detail,
  ).toMatch(/没有成功/);
  const ok = desktopChecks(
    makeSnapshot([task("call", { kind: "mcp.call" })], {
      backends: [{ ...backend, state: "ready", tool_count: 34 }],
    }),
    { graphical_session: true },
  );
  expect(ok.every((c) => c.ok)).toBe(true);
  expect(ok[0].detail).toBe("computer · 已读取 34 个工具");

  const h = health(makeSnapshot([]), app, true, null);
  expect(h).toMatchObject({
    commands: true,
    online: 1,
    servers: 1,
    desktopMissing: 1,
  });
  expect(healthLine(h)).toBe("命令可用 · 桌面需要 1 步");
  expect(healthLine({ ...h, desktopMissing: 0 })).toBe("一切就绪");
  expect(healthLine({ ...h, desktopEnabled: false })).toBe(
    "命令可用 · 桌面已关闭",
  );
  expect(healthLine(health(makeSnapshot([]), app, false, null))).toBe(
    "命令不可用",
  );
});

test("step marks separate the agent's exit codes from Macrun problems", () => {
  const { container, rerender } = render(
    <StepMark
      task={task("x", { status: "failed", result: { exit_code: 2 } })}
    />,
  );
  expect(container.textContent).toBe("退出 2");
  rerender(<StepMark task={task("x", { status: "awaiting_approval" })} />);
  expect(container.textContent).toBe("待确认");
  rerender(
    <StepMark
      task={task("x", { status: "timed_out", error: { message: "t" } })}
    />,
  );
  expect(screen.getByLabelText("超时")).toBeTruthy();
  rerender(<StepMark task={task("x", { status: "running" })} />);
  expect(screen.getByLabelText("进行中")).toBeTruthy();
  rerender(<StepMark task={task("x", { status: "cancelled" })} />);
  expect(container.textContent).toBe("已取消");
});

function renderOverview(tasks: Task[], over: Partial<Snapshot> = {}) {
  const snapshot = makeSnapshot(tasks, over);
  const props = {
    snapshot,
    app,
    available: true,
    act: vi.fn().mockResolvedValue(true),
    isPending: () => false,
    control: vi.fn(),
    onTask: vi.fn(),
    onPage: vi.fn(),
    health: health(snapshot, app, true, { graphical_session: true }),
    servers: serverViews(app.settings, snapshot, true),
  };
  render(<Overview {...props} />);
  return props;
}

test("the overview groups running work by project and folds helper commands", async () => {
  const user = userEvent.setup();
  const props = renderOverview(
    [
      task("run", {
        status: "running",
        ended_at: undefined,
        started_at: now - 356_000,
        arguments: {
          command: "swift build 2>&1 | tail -3; swift test 2>&1 | tail -1",
          cwd: "/tmp/typeflux-gul207/typeflux",
        },
        output_tail: "Compiling\nBuild complete! (35.54 secs)\n",
      }),
      task("exit", { status: "failed", result: { exit_code: 1 } }),
      task("other", {
        status: "running",
        ended_at: undefined,
        arguments: { command: "make", cwd: "/w/lib" },
      }),
    ],
    { today_summary: { total: 3, failed: 1, exited: 1 } },
  );
  expect(
    screen.getByRole("heading", { name: "Agent 正在 2 个项目上工作" }),
  ).toBeTruthy();
  const card = within(screen.getByRole("article", { name: "typeflux 进行中" }));
  expect(
    card.getByRole("heading", { name: "swift build → swift test" }),
  ).toBeTruthy();
  expect(card.getByText("+ 2 个辅助命令")).toBeTruthy();
  expect(card.getByText("Build complete! (35.54 secs)")).toBeTruthy();
  expect(card.getByText("gul207")).toBeTruthy();
  expect(card.getByText("退出 1")).toBeTruthy();
  // A non-zero exit is not a Macrun problem but is still disclosed.
  expect(screen.getByText(/另有 1 条命令退出码非零/)).toBeTruthy();
  expect(screen.queryByRole("region", { name: "需要你" })).toBeNull();
  await user.click(card.getByRole("button", { name: "取消任务" }));
  expect(props.control).toHaveBeenCalledWith("cancel", { task_id: "run" });
  await user.click(card.getByRole("button", { name: "在终端打开目录" }));
  expect(props.act).toHaveBeenCalledWith("open_workspace", {
    root: "/tmp/typeflux-gul207/typeflux",
    terminal: true,
    taskId: "run",
  });
  await user.click(screen.getByRole("button", { name: /命令可用/ }));
  expect(props.onPage).toHaveBeenCalledWith("desktop");
});

test("recent Macrun problems ask for attention once and can be dismissed", async () => {
  const user = userEvent.setup();
  const problem = task("sync", {
    kind: "sync",
    status: "timed_out",
    arguments: { remote_root: "/m/gul-199-ios-v4" },
    error: { message: "timed_out: sync timeout" },
    ended_at: now - 1000,
  });
  const rejected = task("no", {
    status: "denied",
    error: { message: "x", code: "approval_rejected" },
  });
  renderOverview([problem, rejected], {
    today_summary: { total: 2, timed_out: 1, denied: 1 },
    policy: { paused: true, desktop_enabled: true },
  });
  const need = within(screen.getByRole("region", { name: "需要你" }));
  expect(need.getByText("gul-199-ios-v4 · 同步超时")).toBeTruthy();
  // The person's own rejection is not raised back to them.
  expect(need.queryByText(/已拒绝/)).toBeNull();
  expect(need.getByText("已暂停接收新任务")).toBeTruthy();
  await user.click(need.getByRole("button", { name: "知道了" }));
  expect(need.queryByText("gul-199-ios-v4 · 同步超时")).toBeNull();
  expect(
    JSON.parse(localStorage.getItem("macrun-dismissed-problems")!),
  ).toEqual(["sync"]);
  expect(
    screen.getByRole("heading", { name: "已暂停接收新任务" }),
  ).toBeTruthy();
});

test("an approval is titled by its project and the overview stays idle-friendly", () => {
  renderOverview([
    task("wait", {
      status: "awaiting_approval",
      ended_at: undefined,
      approval_deadline: now + 41_000,
      arguments: {
        command: "ps aux | grep x",
        cwd: "/tmp/typeflux-gul207/typeflux",
      },
    }),
  ]);
  expect(screen.getByText("typeflux 想运行一条命令")).toBeTruthy();
  expect(
    screen.getByRole("heading", { name: "有 1 个请求等你确认" }),
  ).toBeTruthy();
  expect(screen.getByText("请先处理上方的确认请求")).toBeTruthy();
});

test("presets apply command and desktop rules together and recognise custom", async () => {
  const user = userEvent.setup();
  const act = vi.fn().mockResolvedValue(true);
  const snapshot = makeSnapshot([]);
  const { rerender } = render(
    <Access snapshot={snapshot} act={act} available pending={false} />,
  );
  expect(
    screen.getByRole("radio", { name: /^放手/ }).getAttribute("aria-checked"),
  ).toBe("true");
  await user.click(screen.getByRole("radio", { name: /^平衡/ }));
  expect(act).toHaveBeenCalledWith(
    "control",
    {
      action: "safety",
      args: {
        ...snapshot.safety,
        approval: "risk",
        desktop: { observe: "allow", control: "allow", high: "confirm" },
      },
    },
    "已切换到“平衡”",
  );
  // Clicking the current preset sends nothing.
  act.mockClear();
  await user.click(screen.getByRole("radio", { name: /^放手/ }));
  expect(act).not.toHaveBeenCalled();
  rerender(
    <Access
      snapshot={{
        ...snapshot,
        safety: {
          ...snapshot.safety,
          desktop: { observe: "deny", control: "allow", high: "allow" },
        },
      }}
      act={act}
      available
      pending={false}
    />,
  );
  expect(screen.getByText(/当前为自定义设置/)).toBeTruthy();
  expect(
    screen
      .getAllByRole("radio")
      .every((r) => r.getAttribute("aria-checked") === "false"),
  ).toBe(true);
  rerender(
    <Access
      snapshot={{
        ...snapshot,
        safety: { ...snapshot.safety, desktop: undefined },
      }}
      act={act}
      available
      pending={false}
    />,
  );
  for (const radio of screen.getAllByRole("radio"))
    expect(radio.hasAttribute("disabled")).toBe(true);
});

function renderMac(over: Partial<React.ComponentProps<typeof ThisMac>> = {}) {
  const snapshot = makeSnapshot([], {
    backends: [
      { name: "computer", state: "ready", command: "cua", tool_count: 3 },
    ],
  });
  const props = {
    snapshot,
    app,
    available: true,
    act: vi.fn().mockResolvedValue(true),
    busy: false,
    health: health(snapshot, app, true, { graphical_session: true }),
    servers: serverViews(app.settings, snapshot, true),
    onPair: vi.fn(),
    onSettings: vi.fn(),
    desktopToggle: vi.fn(),
    desktopPending: false,
    technical: <p>technical-content</p>,
    ...over,
  };
  render(<ThisMac {...props} />);
  return props;
}

test("This Mac lists readiness, servers and the next desktop step", async () => {
  const user = userEvent.setup();
  const props = renderMac();
  expect(screen.getByText("命令与同步：可以使用")).toBeTruthy();
  expect(screen.getByText("桌面：还差 1 步")).toBeTruthy();
  const servers = within(screen.getByLabelText("服务器列表"));
  expect(servers.getByText("主服务器")).toBeTruthy();
  expect(servers.queryByText(/203\.0\.113/)).toBeNull();
  await user.click(servers.getByLabelText("主服务器 的更多操作"));
  await user.click(servers.getByRole("menuitem", { name: "显示地址与证书" }));
  expect(servers.getByLabelText("服务器地址").textContent).toBe(
    "203.0.113.10:7443 · 证书 fp-123",
  );
  await user.click(servers.getByLabelText("主服务器 的更多操作"));
  await user.click(servers.getByRole("menuitem", { name: "重新配对" }));
  expect(props.onPair).toHaveBeenCalledWith(false);
  await user.click(screen.getByRole("button", { name: "添加服务器" }));
  expect(props.onPair).toHaveBeenCalledWith(true);
  const checks = within(screen.getByLabelText("桌面控制检查"));
  await user.click(
    checks.getByRole("checkbox", { name: "允许 Agent 操作桌面" }),
  );
  expect(props.desktopToggle).toHaveBeenCalled();
  await user.click(checks.getByRole("button", { name: "实拍核对" }));
  expect(
    (document.querySelector(".v4-technical") as HTMLDetailsElement).open,
  ).toBe(true);
  await user.click(screen.getByRole("checkbox", { name: "防止 Mac 自动休眠" }));
  expect(props.act).toHaveBeenCalledWith("save_preferences", {
    preferences: { ...app.preferences, keep_awake: false },
  });
});

test("This Mac offers the right first step when the executor is not ready", async () => {
  const user = userEvent.setup();
  const stopped = { ...app, worker_running: false };
  const props = renderMac({
    app: stopped,
    available: false,
    health: health(null, stopped, false, null),
    servers: serverViews(stopped.settings, null, false),
  });
  expect(screen.getByText("命令与同步：执行器未运行")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "启动并连接" }));
  expect(props.act).toHaveBeenCalledWith(
    "start_worker",
    {},
    "已请求启动执行器",
  );
  cleanup();
  const unpaired = {
    ...app,
    worker_running: false,
    settings: { ...app.settings, server: "" },
  };
  const next = renderMac({
    app: unpaired,
    available: false,
    health: health(null, unpaired, false, null),
    servers: [],
  });
  expect(screen.getByText("命令与同步：尚未配对")).toBeTruthy();
  await user.click(screen.getAllByRole("button", { name: "配对服务器" })[0]);
  expect(next.onPair).toHaveBeenCalled();
  expect(screen.getByText("尚未配对服务器")).toBeTruthy();
});

test("the overview side column summarises finished projects and this Mac", async () => {
  const user = userEvent.setup();
  const sleepy = {
    ...app,
    preferences: { ...app.preferences, keep_awake: false },
  };
  const snapshot = makeSnapshot(
    [
      task("sync", {
        kind: "sync",
        status: "running",
        ended_at: undefined,
        arguments: { remote_root: "/m/proj" },
        progress: { received: 3, total: 10, bytes: 4096 },
      }),
      task("desk", {
        kind: "mcp.call",
        status: "running",
        ended_at: undefined,
        arguments: {
          server: "computer",
          tool: "click",
          arguments: { x: 1, y: 2 },
        },
      }),
      task("done", { arguments: { command: "make", cwd: "/w/finished" } }),
      task("broken", {
        status: "unknown",
        arguments: { command: "x", cwd: "/w/broken" },
        ended_at: now - 3 * 3_600_000,
      }),
    ],
    { policy: { paused: false, desktop_enabled: false }, total_tasks: 500 },
  );
  const props = {
    snapshot,
    app: sleepy,
    available: true,
    act: vi.fn(),
    isPending: () => false,
    control: vi.fn(),
    onTask: vi.fn(),
    onPage: vi.fn(),
    health: health(snapshot, sleepy, true, null),
    servers: serverViews(sleepy.settings, snapshot, true),
  };
  render(<Overview {...props} />);
  expect(
    screen.getByText(/3 个文件中已收到|10 个文件中已收到 3 个/),
  ).toBeTruthy();
  expect(screen.getByRole("button", { name: "让 Agent 停下" })).toBeTruthy();
  expect(screen.getByText("Mac 可能自动休眠")).toBeTruthy();
  expect(screen.getByText("桌面控制已关闭")).toBeTruthy();
  expect(screen.getByText(/命令可用 · 桌面已关闭/)).toBeTruthy();
  expect(screen.getByText(/完整历史在活动页/)).toBeTruthy();
  // Older problems stay out of "需要你" but are marked in the finished list.
  expect(screen.queryByRole("region", { name: "需要你" })).toBeNull();
  await user.click(screen.getByRole("button", { name: /broken/ }));
  expect(props.onTask).toHaveBeenCalledWith(
    expect.objectContaining({ task_id: "broken" }),
  );
  await user.click(screen.getByRole("button", { name: "查看活动 ›" }));
  expect(props.onPage).toHaveBeenCalledWith("tasks");
  cleanup();
  render(
    <Overview
      {...props}
      snapshot={null}
      available={false}
      app={{ ...sleepy, worker_running: false }}
      health={health(null, sleepy, false, null)}
    />,
  );
  expect(screen.getByRole("heading", { name: "执行器未运行" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "检查连接" }));
  expect(props.onPage).toHaveBeenCalledWith("desktop");
});
