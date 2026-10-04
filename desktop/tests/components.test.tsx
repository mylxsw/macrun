// @vitest-environment jsdom
import React from "react";
import { afterEach, expect, test, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
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
  expect(act).toHaveBeenLastCalledWith("control", {
    action: "approve",
    args: { task_id: "one", allow: true },
  });
  await user.click(screen.getByRole("button", { name: "拒绝" }));
  expect(act).toHaveBeenLastCalledWith("control", {
    action: "approve",
    args: { task_id: "one", allow: false },
  });
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
        backends: [{ name: "fixture", state: "running", command: "/fixture" }],
      }}
      act={act}
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
    screen.getByRole("checkbox", { name: /任务失败或结果未知时发送通知/ }),
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
  render(<Replay tasks={[task]} act={act} />);
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
  const { rerender } = render(<Replay tasks={[task]} act={act} />);
  await user.click(screen.getByRole("button", { name: /screenshot.*运行中/ }));
  expect(act).toHaveBeenCalledTimes(1);
  expect(screen.getByText("操作进行中，点按刷新")).toBeTruthy();
  rerender(<Replay tasks={[completed]} act={act} />);
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
