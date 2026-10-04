// @vitest-environment jsdom
import React from "react";
import { afterEach, expect, test, vi } from "vitest";
import {
  act as reactAct,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  Approvals,
  Pairing,
  SafetyPanel,
  BackendPanel,
  Replay,
} from "../src/Features";
import { SettingsPage } from "../src/SettingsPage";
import type { AppState, Snapshot } from "../src/types";
afterEach(cleanup);
const safety = {
  restrict_paths: true,
  roots: ["/work"],
  approval: "all",
  retention_days: 30,
  yield_until: 0,
};
const snapshot = {
  safety,
  policy: { paused: false, desktop_enabled: false },
  backends: [],
  connection: { state: "connected", server: "example:7443", since: 0 },
  tasks: [],
  workspaces: [],
  today_summary: {},
  total_tasks: 0,
  active_count: 0,
  protocol: 2,
  version: "0.1.0",
} satisfies Snapshot;
const app = {
  settings: {
    server: "example:7443",
    cert: "/cert.der",
    token_file: "/token",
    backend_config: "",
  },
  preferences: {
    auto_connect: false,
    show_overlay: true,
    yield_input: true,
    keep_awake: false,
    notifications: true,
  },
  worker_running: false,
  snapshot,
  data_dir: "/test",
  legacy_running: false,
  autostart: false,
  platform: "macos",
} satisfies AppState;

test("approval buttons only send the selected task decision", async () => {
  const act = vi.fn().mockResolvedValue({ handled: true }),
    user = userEvent.setup();
  render(
    <Approvals
      tasks={
        [
          {
            task_id: "one",
            status: "awaiting_approval",
            arguments: { command: "echo hello", cwd: "/work" },
          },
        ] as any
      }
      act={act}
      disabled={false}
    />,
  );
  await user.click(screen.getByRole("button", { name: "允许一次" }));
  expect(act).toHaveBeenLastCalledWith(
    "control",
    {
      action: "approve",
      args: { task_id: "one", allow: true, scope: "once" },
    },
    "已允许这一次",
  );
  await user.click(screen.getByRole("button", { name: "拒绝" }));
  expect(act).toHaveBeenLastCalledWith(
    "control",
    {
      action: "approve",
      args: { task_id: "one", allow: false, scope: "once" },
    },
    "已拒绝，Agent 会收到 approval_rejected",
  );
});

test("unsaved safety edits survive incoming snapshots and save failure", async () => {
  const act = vi.fn().mockResolvedValue(undefined),
    user = userEvent.setup();
  const { rerender } = render(
    <SafetyPanel snapshot={snapshot} act={act} disabled={false} />,
  );
  await user.click(screen.getByRole("button", { name: "添加目录" }));
  await user.type(
    screen.getByRole("textbox", { name: "允许的绝对目录" }),
    "/new",
  );
  await user.click(screen.getByRole("button", { name: "添加", exact: true }));
  await user.click(screen.getByRole("button", { name: "移除 /work" }));
  rerender(
    <SafetyPanel
      snapshot={{ ...snapshot, safety: { ...safety, roots: ["/remote"] } }}
      act={act}
      disabled={false}
    />,
  );
  expect(screen.getByText("/new")).toBeTruthy();
  expect(screen.queryByText("/remote")).toBeNull();
  await user.click(screen.getByRole("button", { name: "保存安全设置" }));
  expect(act.mock.calls[0][1].args.roots).toEqual(["/new"]);
  expect(screen.getByText("更改尚未保存")).toBeTruthy();
  expect(screen.getByText("/new")).toBeTruthy();
});

test("safety saving keeps a retention change received while editing paths", async () => {
  const act = vi.fn().mockResolvedValue(true),
    user = userEvent.setup();
  const { rerender } = render(
    <SafetyPanel
      snapshot={snapshot}
      act={act}
      disabled={false}
      includeRetention={false}
    />,
  );
  await user.click(screen.getByRole("button", { name: "风险命令先确认" }));
  rerender(
    <SafetyPanel
      snapshot={{ ...snapshot, safety: { ...safety, retention_days: 90 } }}
      act={act}
      disabled={false}
      includeRetention={false}
    />,
  );
  await user.click(screen.getByRole("button", { name: "保存安全设置" }));
  expect(act.mock.calls[0][1].args).toMatchObject({
    approval: "risk",
    retention_days: 90,
  });
});

test("pairing preserves the code after a failure and requires successful connection checks", async () => {
  let pairAttempts = 0;
  const act = vi.fn(async (command: string) => {
      if (command === "pair")
        return ++pairAttempts === 1 ? undefined : { fingerprint: "pin" };
      if (command === "connection_check")
        return { checks: [{ name: "证书校验", ok: false }] };
      return true;
    }),
    user = userEvent.setup();
  render(<Pairing act={act} running={false} />);
  const input = screen.getByPlaceholderText("macrun://pair/…");
  await user.type(input, "macrun://pair/test");
  await user.click(screen.getByRole("button", { name: "连接", exact: true }));
  expect((input as HTMLInputElement).value).toBe("macrun://pair/test");
  expect(screen.getByRole("alert").textContent).toContain("配对未完成");
  await user.click(screen.getByRole("button", { name: "连接", exact: true }));
  await waitFor(() => expect(screen.getByText("pin")).toBeTruthy());
  expect(act).toHaveBeenCalledWith("start_worker");
  expect(
    screen.getByRole("button", { name: "继续" }).hasAttribute("disabled"),
  ).toBe(true);
});

test("pairing cannot continue on an empty check list and finishes only after saving desktop policy", async () => {
  let checkAttempts = 0;
  const onComplete = vi.fn();
  const act = vi.fn(async (command: string) => {
      if (command === "pair") return { fingerprint: "pin" };
      if (command === "connection_check")
        return {
          checks: ++checkAttempts === 1 ? [] : [{ name: "证书校验", ok: true }],
        };
      return true;
    }),
    user = userEvent.setup();
  render(<Pairing act={act} running={false} onComplete={onComplete} />);
  await user.type(
    screen.getByPlaceholderText("macrun://pair/…"),
    "macrun://pair/test",
  );
  await user.click(screen.getByRole("button", { name: "连接", exact: true }));
  expect(
    screen.getByRole("button", { name: "继续" }).hasAttribute("disabled"),
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "重新检查" }));
  await user.click(screen.getByRole("button", { name: "继续" }));
  await user.click(screen.getByRole("button", { name: "稍后再说" }));
  expect(act).toHaveBeenCalledWith(
    "control",
    { action: "desktop", args: { enabled: false } },
    "仅启用命令与文件能力",
  );
  expect(onComplete).toHaveBeenCalledWith("main");
});

test("invalid observation JSON never dispatches a backend action", async () => {
  const act = vi.fn().mockResolvedValue({
      session: "session",
      result: {
        tools: [{ name: "observe", inputSchema: { type: "object" } }],
      },
    }),
    user = userEvent.setup();
  render(
    <BackendPanel
      snapshot={{
        ...snapshot,
        policy: { paused: false, desktop_enabled: true },
        backends: [{ name: "fixture", state: "running", command: "/fixture" }],
      }}
      act={act}
      read={act}
      app={null}
      mode="advanced"
    />,
  );
  await user.click(screen.getByText("后端实拍与状态核对"));
  await user.selectOptions(screen.getAllByRole("combobox")[0], "fixture");
  await user.click(screen.getByRole("button", { name: "读取工具" }));
  await user.selectOptions(screen.getAllByRole("combobox")[1], "observe");
  const input = screen.getByRole("textbox");
  await user.clear(input);
  await user.type(input, "bad json");
  await user.click(screen.getByRole("button", { name: "执行观察并核对" }));
  expect(screen.getByText("工具参数必须为有效 JSON")).toBeTruthy();
  expect(act).toHaveBeenCalledTimes(1);
});

test("manual connection is collapsed and unsaved edits survive incoming app state and save failure", async () => {
  const act = vi.fn().mockResolvedValue(undefined),
    user = userEvent.setup();
  const props = {
    snapshot,
    busy: false,
    available: true,
    act,
    refresh: vi.fn().mockResolvedValue(undefined),
    setError: vi.fn(),
    onPair: vi.fn(),
  };
  const { rerender } = render(<SettingsPage app={app} {...props} />);
  expect(screen.queryByRole("textbox", { name: "服务器地址" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "手动连接配置" }));
  const input = screen.getByRole("textbox", { name: "服务器地址" });
  await user.clear(input);
  await user.type(input, "edited:7443");
  rerender(
    <SettingsPage
      app={{ ...app, settings: { ...app.settings, server: "remote:7443" } }}
      {...props}
    />,
  );
  expect((input as HTMLInputElement).value).toBe("edited:7443");
  await user.click(screen.getByRole("button", { name: "保存配置" }));
  expect(act.mock.calls[0][1].settings.server).toBe("edited:7443");
  expect(screen.getByText("更改尚未保存。")).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "启动并连接" }).hasAttribute("disabled"),
  ).toBe(true);
});

test("settings disable disconnect with active tasks and retain all other preference values", async () => {
  const act = vi.fn().mockResolvedValue(true),
    user = userEvent.setup();
  render(
    <SettingsPage
      app={{ ...app, worker_running: true }}
      snapshot={{ ...snapshot, active_count: 1 }}
      busy={false}
      available
      act={act}
      refresh={vi.fn()}
      setError={vi.fn()}
      onPair={vi.fn()}
    />,
  );
  expect(
    screen.getByRole("button", { name: "断开连接" }).hasAttribute("disabled"),
  ).toBe(true);
  await user.click(
    screen.getByRole("checkbox", { name: /需要确认、失败或结果未知时发送通知/ }),
  );
  expect(act).toHaveBeenCalledWith("save_preferences", {
    preferences: { ...app.preferences, notifications: false },
  });
});

test("replay fetches screenshots only after an explicit action and reuses the record", async () => {
  const user = userEvent.setup();
  const task = {
    task_id: "shot",
    kind: "mcp.call",
    status: "succeeded",
    arguments: { tool: "screenshot" },
    started_at: 1,
  };
  const act = vi.fn().mockResolvedValue({
    ...task,
    result: {
      result: {
        content: [{ type: "image", mimeType: "image/png", data: "aW1hZ2U=" }],
      },
    },
  });
  render(<Replay tasks={[task]} act={act} read={act} />);
  expect(act).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "显示截图" }));
  expect(act).toHaveBeenCalledWith("control", {
    action: "task_detail",
    args: { task_id: "shot" },
  });
  expect(
    screen
      .getByRole("img", { name: "screenshot的记录截图" })
      .getAttribute("src"),
  ).toBe("data:image/png;base64,aW1hZ2U=");
  await user.click(screen.getByRole("button", { name: /screenshot.*成功/ }));
  expect(act).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("heading", { name: "操作详情" })).toBeTruthy();
});

test("keeping the Mac awake can be turned off without losing other preferences", async () => {
  const user = userEvent.setup(),
    act = vi.fn().mockResolvedValue({
      accessibility: true,
      screen_recording: true,
      graphical_session: true,
    });
  render(
    <BackendPanel
      snapshot={snapshot}
      act={act}
      read={act}
      app={{ ...app, preferences: { ...app.preferences, keep_awake: true } }}
      mode="permissions"
    />,
  );
  await waitFor(() =>
    expect(
      screen
        .getByRole("button", { name: "检查系统权限" })
        .hasAttribute("disabled"),
    ).toBe(false),
  );
  await user.click(screen.getByRole("button", { name: "关闭保持唤醒" }));
  expect(act).toHaveBeenCalledWith("save_preferences", {
    preferences: { ...app.preferences, keep_awake: false },
  });
});

test("replay reloads a running record after completion and then caches its screenshot", async () => {
  const user = userEvent.setup();
  const task = {
    task_id: "progressing-shot",
    kind: "mcp.call",
    status: "running",
    arguments: { tool: "screenshot" },
    started_at: 1,
  };
  const completed = { ...task, status: "succeeded", ended_at: 2 };
  const act = vi
    .fn()
    .mockResolvedValueOnce(task)
    .mockResolvedValue({
      ...completed,
      result: {
        result: {
          content: [{ type: "image", mimeType: "image/png", data: "ZG9uZQ==" }],
        },
      },
    });
  const { rerender } = render(<Replay tasks={[task]} act={act} read={act} />);
  await user.click(screen.getByRole("button", { name: /screenshot.*运行中/ }));
  expect(act).toHaveBeenCalledTimes(1);
  expect(screen.getByText("操作进行中，点按刷新")).toBeTruthy();
  rerender(<Replay tasks={[completed]} act={act} read={act} />);
  expect(act).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole("button", { name: /screenshot.*成功/ }));
  expect(act).toHaveBeenCalledTimes(2);
  expect(
    screen
      .getByRole("img", { name: "screenshot的记录截图" })
      .getAttribute("src"),
  ).toBe("data:image/png;base64,ZG9uZQ==");
  await user.click(screen.getByRole("button", { name: /screenshot.*成功/ }));
  expect(act).toHaveBeenCalledTimes(2);
});

test("directory restrictions are visibly off until enabled and explicitly saved", async () => {
  const user = userEvent.setup(),
    act = vi.fn().mockResolvedValue(undefined);
  render(
    <SafetyPanel
      snapshot={{ ...snapshot, safety: { ...safety, restrict_paths: false } }}
      act={act}
      disabled={false}
    />,
  );
  const restriction = screen.getByRole("checkbox", {
    name: "限制工作目录",
  }) as HTMLInputElement;
  expect(restriction.checked).toBe(false);
  expect(
    screen.getByText(
      "目录限制未开启，以下目录不会约束命令、文件读写或同步目标。",
    ),
  ).toBeTruthy();
  expect(restriction.closest("details")).toBeNull();
  await user.click(restriction);
  expect(act).not.toHaveBeenCalled();
  expect(screen.getByText("更改尚未保存")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "保存安全设置" }));
  expect(act.mock.calls[0][1].args).toMatchObject({
    restrict_paths: true,
    roots: ["/work"],
  });
});

const migrationProps = () => ({
  app: { ...app, legacy_running: true },
  snapshot,
  busy: false,
  available: true,
  act: vi.fn(),
  refresh: vi.fn(),
  setError: vi.fn(),
  onPair: vi.fn(),
});

test("migration dialog traps focus and cancelling or Escape never migrates", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  render(<SettingsPage {...props} />);
  const trigger = screen.getByRole("button", { name: "迁移", exact: true });
  await user.click(trigger);
  const dialog = screen.getByRole("dialog", { name: "迁移旧执行器" });
  const cancel = within(dialog).getByRole("button", { name: "取消" });
  const confirm = within(dialog).getByRole("button", { name: "确认迁移" });
  expect(document.activeElement).toBe(cancel);
  await user.tab({ shift: true });
  expect(document.activeElement).toBe(confirm);
  await user.tab();
  expect(document.activeElement).toBe(cancel);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(trigger);
  await user.click(trigger);
  await user.click(screen.getByRole("button", { name: "取消" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(props.act).not.toHaveBeenCalled();
});

test("migration submits once while pending and offers an explicit start after success", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  let finishMigration!: (value: unknown) => void;
  props.act
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishMigration = resolve;
        }),
    )
    .mockResolvedValue(true);
  const { rerender } = render(<SettingsPage {...props} />);
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  await user.dblClick(screen.getByRole("button", { name: "确认迁移" }));
  expect(props.act).toHaveBeenCalledTimes(1);
  expect(props.act).toHaveBeenCalledWith(
    "migrate_legacy",
    {},
    "旧服务已迁移，可以启动桌面连接",
  );
  expect(
    screen.getByRole("button", { name: "正在迁移…" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(
    screen.getByRole("button", { name: "取消" }).hasAttribute("disabled"),
  ).toBe(true);
  await user.keyboard("{Escape}");
  expect(screen.getByRole("dialog")).toBeTruthy();
  await reactAct(async () =>
    finishMigration({ backup: "/test/legacy.plist.bak" }),
  );
  rerender(<SettingsPage {...props} app={{ ...app, legacy_running: false }} />);
  const dialog = screen.getByRole("dialog", { name: "迁移完成" });
  expect(within(dialog).getByText("/test/legacy.plist.bak")).toBeTruthy();
  expect(props.act).toHaveBeenCalledTimes(1);
  await user.click(within(dialog).getByRole("button", { name: "启动并连接" }));
  expect(props.act).toHaveBeenLastCalledWith(
    "start_worker",
    {},
    "已请求启动执行器",
  );
  expect(
    within(dialog).getByText("已请求启动执行器，可在连接区域查看连接状态。"),
  ).toBeTruthy();
  rerender(
    <SettingsPage
      {...props}
      app={{ ...app, legacy_running: false, worker_running: true }}
    />,
  );
  await user.click(within(dialog).getByRole("button", { name: "完成" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: "断开连接" }),
  );
});

test("a refused native migration stays in the dialog and shows the actual error", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  props.act.mockResolvedValue(undefined);
  const { rerender } = render(<SettingsPage {...props} />);
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  await user.click(screen.getByRole("button", { name: "确认迁移" }));
  rerender(
    <SettingsPage
      {...props}
      actionError="旧执行器有未结束任务，请先停止任务再迁移"
    />,
  );
  const dialog = screen.getByRole("dialog", { name: "迁移旧执行器" });
  expect(within(dialog).getByRole("alert").textContent).toContain(
    "旧执行器有未结束任务，请先停止任务再迁移",
  );
  expect(
    within(dialog)
      .getByRole("button", { name: "重试迁移" })
      .hasAttribute("disabled"),
  ).toBe(false);
  expect(props.act).toHaveBeenCalledTimes(1);
});

test("a thrown migration error remains visible and never starts the executor", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  props.act.mockRejectedValue(new Error("无法停止旧服务；备份已保留"));
  render(<SettingsPage {...props} />);
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  await user.click(screen.getByRole("button", { name: "确认迁移" }));
  expect(screen.getByRole("dialog", { name: "迁移旧执行器" })).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain(
    "无法停止旧服务；备份已保留",
  );
  expect(props.act).toHaveBeenCalledTimes(1);
});

test("migration close focuses the connection heading when its action is disabled", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  props.act.mockResolvedValue({ backup: "/test/legacy.plist.bak" });
  const { rerender } = render(<SettingsPage {...props} />);
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  await user.click(screen.getByRole("button", { name: "确认迁移" }));
  rerender(
    <SettingsPage
      {...props}
      app={{ ...app, legacy_running: false, worker_running: true }}
      snapshot={{ ...snapshot, active_count: 1 }}
    />,
  );
  await user.click(screen.getByRole("button", { name: "完成" }));
  expect(
    screen.getByRole("button", { name: "断开连接" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(document.activeElement).toBe(
    screen.getByRole("heading", { name: "连接", exact: true }),
  );
});

test("migration yields Escape and Tab to a dialog rendered above it", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  const { rerender } = render(<SettingsPage {...props} />);
  await user.click(screen.getByRole("button", { name: "迁移", exact: true }));
  const topKey = vi.fn((event: React.KeyboardEvent) => {
    event.preventDefault();
  });
  rerender(
    <>
      <SettingsPage {...props} />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="退出确认"
        onKeyDown={topKey}
      >
        <button>继续运行</button>
      </div>
    </>,
  );
  const topButton = screen.getByRole("button", { name: "继续运行" });
  await user.click(topButton);
  await user.keyboard("{Escape}");
  expect(topKey).toHaveBeenCalled();
  expect(screen.getByRole("dialog", { name: "迁移旧执行器" })).toBeTruthy();
  await user.tab();
  expect(document.activeElement).toBe(topButton);
  rerender(<SettingsPage {...props} />);
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(props.act).not.toHaveBeenCalled();
});

test("a stopped legacy configuration has a prominent migration entry and survives cancellation", async () => {
  const props = migrationProps(),
    user = userEvent.setup();
  render(
    <SettingsPage
      {...props}
      app={{ ...app, legacy_running: false, legacy_detected: true }}
    />,
  );
  const banner = screen.getByRole("region", { name: "旧版 Macrun 迁移" });
  expect(within(banner).getByText("检测到旧版 Macrun")).toBeTruthy();
  expect(
    within(banner).getByText(
      "已有配置，无需重新配对。迁移后保留任务记录和同步状态，由桌面应用统一管理。",
    ),
  ).toBeTruthy();
  expect(
    within(banner).getByText("已找到旧版配置，旧执行器当前未运行。"),
  ).toBeTruthy();
  const safetySection = screen
    .getByRole("heading", { name: "安全边界", exact: true })
    .closest("section")!;
  expect(
    banner.compareDocumentPosition(safetySection) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).not.toBe(0);
  expect(props.act).not.toHaveBeenCalled();
  const migrate = within(banner).getByRole("button", {
    name: "迁移",
    exact: true,
  });
  await user.click(migrate);
  await user.click(screen.getByRole("button", { name: "取消" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByRole("region", { name: "旧版 Macrun 迁移" })).toBe(banner);
  expect(document.activeElement).toBe(migrate);
  expect(props.act).not.toHaveBeenCalled();
});

test("the migration banner distinguishes a running legacy service and hides when absent", () => {
  const props = migrationProps();
  const { rerender } = render(<SettingsPage {...props} />);
  expect(
    screen.getByText("旧执行器正在运行；迁移前请确认任务已经结束。"),
  ).toBeTruthy();
  rerender(
    <SettingsPage
      {...props}
      app={{ ...app, legacy_running: false, legacy_detected: false }}
    />,
  );
  expect(screen.queryByRole("region", { name: "旧版 Macrun 迁移" })).toBeNull();
});

test("manual save locks edits until the pending request finishes and preserves failed drafts", async () => {
  let finish!: (value: unknown) => void;
  const act = vi.fn(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const user = userEvent.setup();
  render(
    <SettingsPage
      app={app}
      snapshot={snapshot}
      available
      busy={false}
      act={act}
      refresh={vi.fn()}
      setError={vi.fn()}
      onPair={vi.fn()}
      manualOpen
    />,
  );
  const server = screen.getByRole("textbox", {
    name: "服务器地址",
  }) as HTMLInputElement;
  await user.clear(server);
  await user.type(server, "pending-save:7443");
  const save = screen.getByRole("button", { name: "保存配置" });
  await user.click(save);
  expect(server.disabled).toBe(true);
  expect(save.hasAttribute("disabled")).toBe(true);
  await user.click(save);
  expect(act).toHaveBeenCalledTimes(1);
  await reactAct(async () => finish(false));
  expect(server.disabled).toBe(false);
  expect(server.value).toBe("pending-save:7443");
  expect(screen.getByText("更改尚未保存。")).toBeTruthy();
});

test("editing a paired token file switches manual saving away from the old keychain credential", async () => {
  const act = vi.fn().mockResolvedValue(undefined),
    user = userEvent.setup();
  render(
    <SettingsPage
      app={{
        ...app,
        settings: {
          ...app.settings,
          token_file: "/dev/null",
          keychain_account: "old-account",
        },
      }}
      snapshot={snapshot}
      available
      busy={false}
      act={act}
      refresh={vi.fn()}
      setError={vi.fn()}
      onPair={vi.fn()}
      manualOpen
    />,
  );
  const token = screen.getByRole("textbox", { name: "令牌文件路径" });
  await user.clear(token);
  await user.type(token, "/test/new-token");
  await user.click(screen.getByRole("button", { name: "保存配置" }));
  expect(act).toHaveBeenCalledWith(
    "save_settings",
    {
      settings: expect.objectContaining({
        token_file: "/test/new-token",
        keychain_account: "",
      }),
    },
    "连接配置已保存",
  );
});

test("starting state explains native authorization and blocks duplicate connection edits", () => {
  const props = {
    snapshot,
    available: false,
    busy: false,
    act: vi.fn(),
    refresh: vi.fn(),
    setError: vi.fn(),
    onPair: vi.fn(),
    manualOpen: true,
  };
  const { rerender } = render(
    <SettingsPage {...props} app={{ ...app, worker_starting: true }} />,
  );
  expect(
    screen.getByRole("button", { name: "正在启动…" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(
    screen.getByRole("button", { name: "重新配对" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(
    (screen.getByRole("textbox", { name: "服务器地址" }) as HTMLInputElement)
      .disabled,
  ).toBe(true);
  expect(screen.getByText(/如果 macOS 请求访问钥匙串/)).toBeTruthy();
  rerender(
    <SettingsPage {...props} app={{ ...app, worker_starting: false }} />,
  );
  expect(
    screen.getByRole("button", { name: "启动并连接" }).hasAttribute("disabled"),
  ).toBe(false);
  expect(screen.queryByText(/如果 macOS 请求访问钥匙串/)).toBeNull();
});

test("connection checks cannot show an old success after connection settings change", async () => {
  let finish!: (value: unknown) => void;
  const act = vi.fn(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const props = {
    snapshot,
    available: true,
    busy: false,
    act,
    refresh: vi.fn(),
    setError: vi.fn(),
    onPair: vi.fn(),
  };
  const user = userEvent.setup();
  const { rerender } = render(<SettingsPage {...props} app={app} />);
  await user.click(screen.getByRole("button", { name: "测试连通性" }));
  rerender(
    <SettingsPage
      {...props}
      app={{ ...app, settings: { ...app.settings, server: "changed:7443" } }}
    />,
  );
  await reactAct(async () =>
    finish({ checks: [{ name: "Old connection", ok: true }] }),
  );
  expect(screen.queryByText(/Old connection/)).toBeNull();
  expect(screen.getByText("changed:7443")).toBeTruthy();
});
