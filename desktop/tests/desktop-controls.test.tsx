// @vitest-environment jsdom
import React from "react";
import { afterEach, expect, test, vi } from "vitest";
import {
  act as reactAct,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  BackendPanel,
  Replay,
  ToolResult,
  WorkspaceList,
} from "../src/Features";
import type { AppState, Snapshot, Task } from "../src/types";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
const snapshot = {
  safety: {
    restrict_paths: false,
    roots: [],
    approval: "risk",
    retention_days: 30,
    yield_until: 0,
  },
  policy: { paused: false, desktop_enabled: true },
  backends: [{ name: "fixture", state: "ready", command: "/fixture" }],
  connection: { state: "connected", server: "test:7443", since: 0 },
  tasks: [],
  workspaces: [],
  today_summary: {},
  total_tasks: 0,
  active_count: 0,
  protocol: 2,
  version: "test",
} satisfies Snapshot;
const app = {
  preferences: {
    auto_connect: false,
    show_overlay: false,
    yield_input: true,
    keep_awake: false,
    notifications: false,
  },
  settings: {
    server: "test:7443",
    cert: "/cert",
    token_file: "/token",
    backend_config: "",
  },
  worker_running: false,
  snapshot,
  data_dir: "/test",
  legacy_running: false,
  autostart: false,
  platform: "macos",
} satisfies AppState;
const toolList = {
  session: "session",
  result: { tools: [{ name: "screenshot", inputSchema: { type: "object" } }] },
};
const task = (id: string, status = "succeeded"): Task => ({
  task_id: id,
  kind: "mcp.call",
  status,
  arguments: { tool: id },
  started_at: 1,
});
// Installation detection is independent of the tool/paging fixtures below.
const backendRead = (read: (...args: any[]) => Promise<any>) =>
  (command: string, args?: Record<string, unknown>) =>
    command === "cua_status"
      ? Promise.resolve({ state: "missing" })
      : read(command, args);

test("workspace cards show six initially and expose all rows through the more action", async () => {
  const user = userEvent.setup();
  const workspaces = Array.from({ length: 24 }, (_, i) => ({
    root: `/long-directory/workspace-${i}`,
    status: "succeeded",
    time: 1,
  }));
  render(
    <WorkspaceList snapshot={{ ...snapshot, workspaces }} act={vi.fn()} />,
  );
  expect(screen.getAllByLabelText(/打开工作区/)).toHaveLength(6);
  expect(screen.getByText("已显示 6 / 24 个工作区")).toBeTruthy();
  for (let i = 0; i < 3; i++)
    await user.click(screen.getByRole("button", { name: "查看更多工作区" }));
  expect(screen.getAllByLabelText(/打开工作区/)).toHaveLength(24);
  expect(screen.queryByRole("button", { name: "查看更多工作区" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "收起", exact: true }));
  expect(screen.getAllByLabelText(/打开工作区/)).toHaveLength(6);
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function chooseTool(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByText("后端实拍与状态核对"));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "后端" }),
    "fixture",
  );
  await user.click(screen.getByRole("button", { name: "读取工具" }));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "工具（1）" }),
    "screenshot",
  );
}

test("permission checks use independent reads and retain a visible failure without implying backend access", async () => {
  const act = vi.fn();
  const read = vi.fn(async (command: string) => {
    if (command === "permissions") throw new Error("permission read refused");
    return { available: false };
  });
  render(
    <BackendPanel
      snapshot={snapshot}
      app={app}
      act={act}
      read={read}
      mode="permissions"
    />,
  );
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "permission read refused",
  );
  expect(screen.getAllByText("尚未检查")).toHaveLength(4);
  expect(screen.getByText(/以上权限仅检查 Macrun 应用/)).toBeTruthy();
  expect(act).not.toHaveBeenCalled();
  expect(
    screen
      .getByRole("button", { name: "检查系统权限" })
      .hasAttribute("disabled"),
  ).toBe(false);
});

test("reading tools locks the backend selection and exposes failure locally", async () => {
  const pending = deferred<any>();
  const user = userEvent.setup();
  const read = vi.fn().mockReturnValue(pending.promise);
  const act = vi.fn();
  render(
    <BackendPanel
      snapshot={snapshot}
      app={null}
      act={act}
      read={backendRead(read)}
      mode="advanced"
    />,
  );
  await user.click(screen.getByText("后端实拍与状态核对"));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "后端" }),
    "fixture",
  );
  await user.click(screen.getByRole("button", { name: "读取工具" }));
  expect(
    screen.getByRole("combobox", { name: "后端" }).hasAttribute("disabled"),
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "读取工具中…" }));
  expect(read).toHaveBeenCalledTimes(1);
  await reactAct(async () => pending.reject(new Error("backend unavailable")));
  expect(screen.getByRole("alert").textContent).toBe("backend unavailable");
  expect(
    screen.getByRole("combobox", { name: "后端" }).hasAttribute("disabled"),
  ).toBe(false);
  expect(act).not.toHaveBeenCalled();
});

test("tool discovery can read the next page without losing the chosen tool and is disabled offline", async () => {
  const user = userEvent.setup(),
    act = vi.fn();
  const read = vi
    .fn()
    .mockResolvedValueOnce({
      ...toolList,
      result: { ...toolList.result, nextCursor: "next" },
    })
    .mockResolvedValueOnce({
      session: "session",
      result: {
        tools: [
          {
            name: "observe",
            description: "read screen",
            inputSchema: { type: "object" },
          },
        ],
      },
    });
  const props = { app: null, act, read: backendRead(read), mode: "advanced" as const };
  const { rerender } = render(<BackendPanel snapshot={snapshot} {...props} />);
  await chooseTool(user);
  await user.click(screen.getByRole("button", { name: "读取更多工具" }));
  expect(read).toHaveBeenLastCalledWith("control", {
    action: "tools",
    args: { server: "fixture", session: "session", cursor: "next" },
  });
  expect(screen.getByRole("combobox", { name: "工具（2）" })).toHaveProperty(
    "value",
    "screenshot",
  );
  expect(screen.getByRole("option", { name: "observe" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "读取更多工具" })).toBeNull();
  rerender(<BackendPanel snapshot={null} {...props} />);
  for (const name of ["读取工具", "重启后端", "执行观察并核对"])
    expect(screen.getByRole("button", { name }).hasAttribute("disabled")).toBe(
      true,
    );
});

test("observation rejects JSON scalars and a failed backend restart preserves discovered tools", async () => {
  const user = userEvent.setup(),
    act = vi.fn().mockResolvedValue(undefined),
    read = vi.fn().mockResolvedValue(toolList);
  render(
    <BackendPanel
      snapshot={snapshot}
      app={null}
      act={act}
      read={backendRead(read)}
      mode="advanced"
    />,
  );
  await chooseTool(user);
  await user.clear(screen.getByRole("textbox", { name: "工具参数 JSON" }));
  await user.type(
    screen.getByRole("textbox", { name: "工具参数 JSON" }),
    "null",
  );
  await user.click(screen.getByRole("button", { name: "执行观察并核对" }));
  expect(screen.getByRole("alert").textContent).toMatch(/JSON 对象/);
  expect(act).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "重启后端" }));
  expect(screen.getByRole("combobox", { name: "工具（1）" })).toHaveProperty(
    "value",
    "screenshot",
  );
  expect(screen.getByText("操作未完成，请查看错误提示后重试。")).toBeTruthy();
});

test("an observation read failure stops polling and retry only reads the original task", async () => {
  vi.useFakeTimers();
  const read = vi
    .fn()
    .mockResolvedValueOnce(toolList)
    .mockRejectedValueOnce(new Error("disconnected"))
    .mockResolvedValue({
      ...task("shot"),
      result: {
        result: { content: [{ type: "text", text: "verified result" }] },
      },
    });
  const act = vi
    .fn()
    .mockResolvedValue({ task_id: "shot", status: "accepted" });
  render(
    <BackendPanel
      snapshot={snapshot}
      app={null}
      act={act}
      read={backendRead(read)}
      mode="advanced"
    />,
  );
  fireEvent.click(screen.getByText("后端实拍与状态核对"));
  fireEvent.change(screen.getByRole("combobox", { name: "后端" }), {
    target: { value: "fixture" },
  });
  await reactAct(async () => {
    fireEvent.click(screen.getByRole("button", { name: "读取工具" }));
  });
  fireEvent.change(screen.getByRole("combobox", { name: "工具（1）" }), {
    target: { value: "screenshot" },
  });
  await reactAct(async () => {
    fireEvent.click(screen.getByRole("button", { name: "执行观察并核对" }));
  });
  await reactAct(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(screen.getByRole("alert").textContent).toMatch(/disconnected/);
  await reactAct(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(read).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole("button", { name: "重新读取状态" }));
  await reactAct(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(screen.getByText("verified result")).toBeTruthy();
  expect(act).toHaveBeenCalledTimes(1);
  expect(read).toHaveBeenLastCalledWith("control", {
    action: "task_detail",
    args: { task_id: "shot" },
  });
  await reactAct(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(read).toHaveBeenCalledTimes(3);
});

test("backend config draft survives save failure and cannot be replaced while dirty", async () => {
  const user = userEvent.setup(),
    read = vi.fn().mockResolvedValue("# existing config"),
    act = vi.fn().mockResolvedValue(undefined);
  render(
    <BackendPanel
      snapshot={snapshot}
      app={app}
      act={act}
      read={backendRead(read)}
      mode="advanced"
    />,
  );
  await user.click(screen.getByText("添加 / 编辑 MCP 后端"));
  await user.click(screen.getByRole("button", { name: "读取配置" }));
  await user.type(screen.getByRole("textbox"), "\n# edited");
  expect(
    screen.getByRole("button", { name: "读取配置" }).hasAttribute("disabled"),
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "保存配置" }));
  expect(screen.getByRole("alert").textContent).toMatch(/操作未完成/);
  expect(screen.getByRole("textbox")).toHaveProperty(
    "value",
    "# existing config\n# edited",
  );
  expect(screen.getByText("更改尚未保存。")).toBeTruthy();
});

test("replay ignores a late selection response and displays the selected task error", async () => {
  const first = deferred<any>(),
    second = deferred<any>(),
    user = userEvent.setup();
  const read = vi.fn((_c, args) =>
    args.args.task_id === "first" ? first.promise : second.promise,
  );
  render(
    <Replay
      tasks={[task("first"), task("second", "failed")]}
      act={vi.fn()}
      read={read}
    />,
  );
  await user.click(screen.getByRole("button", { name: /first.*成功/ }));
  await user.click(screen.getByRole("button", { name: /second.*失败/ }));
  await reactAct(async () =>
    second.resolve({
      ...task("second", "failed"),
      error: { message: "screen access denied" },
    }),
  );
  await reactAct(async () => first.resolve(task("first")));
  const detail = screen
    .getByRole("heading", { name: "操作详情" })
    .closest(".replay-detail") as HTMLElement;
  expect(within(detail).getByRole("alert").textContent).toBe(
    "screen access denied",
  );
  expect(detail.textContent).toContain("second");
  expect(detail.textContent).not.toContain("first");
});

test("replay read failure is visible and evicts records outside the latest eight", async () => {
  const user = userEvent.setup(),
    read = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockImplementation(async (_c, args) => task(args.args.task_id));
  const props = { act: vi.fn(), read };
  const { rerender } = render(<Replay tasks={[task("first")]} {...props} />);
  await user.click(screen.getByRole("button", { name: /first.*成功/ }));
  expect(screen.getByRole("alert").textContent).toMatch(/offline/);
  expect(screen.queryByText("这次操作没有截图")).toBeNull();
  await user.click(screen.getByRole("button", { name: /first.*成功/ }));
  rerender(<Replay tasks={[task("second")]} {...props} />);
  rerender(<Replay tasks={[task("first")]} {...props} />);
  await user.click(screen.getByRole("button", { name: /first.*成功/ }));
  expect(read).toHaveBeenCalledTimes(3);
});

test("malformed and unsupported tool content cannot crash the view and tool errors are visible", () => {
  const { rerender } = render(
    <ToolResult value={{ result: { content: { unexpected: true } } }} />,
  );
  rerender(
    <ToolResult
      value={{
        result: {
          isError: true,
          content: [
            null,
            { type: "image", mimeType: "image/svg+xml", data: "unsafe" },
            { type: "text", text: "permission denied" },
          ],
        },
      }}
    />,
  );
  expect(screen.getByRole("alert").textContent).toMatch(/执行失败/);
  expect(screen.getByText("permission denied")).toBeTruthy();
  expect(screen.queryByRole("img")).toBeNull();
});

test("structured tool results expose window identifiers while preserving text and images", async () => {
  const user = userEvent.setup();
  render(
    <ToolResult
      value={{
        result: {
          content: [
            { type: "text", text: "Found 5 window(s)." },
            { type: "image", mimeType: "image/png", data: "ZmFrZS1maXh0dXJl" },
          ],
          structuredContent: {
            windows: [{ window_id: 527, app_name: "Fixture App" }],
          },
        },
      }}
    />,
  );
  expect(screen.getByText("Found 5 window(s).")).toBeTruthy();
  expect(screen.getByRole("img", { name: "后端观察截图" })).toBeTruthy();
  const summary = screen.getByText("结构化结果");
  expect(summary.closest("details")?.hasAttribute("open")).toBe(false);
  await user.click(summary);
  expect(summary.closest("details")?.hasAttribute("open")).toBe(true);
  expect(screen.getByLabelText("结构化结果 JSON").textContent).toContain(
    '"window_id": 527',
  );
});

test("structured-only results remain readable and large strings or arrays have bounded previews", async () => {
  const user = userEvent.setup();
  const { rerender } = render(
    <ToolResult value={{ result: { structuredContent: { window_id: 42 } } }} />,
  );
  await user.click(screen.getByText("结构化结果"));
  expect(screen.getByLabelText("结构化结果 JSON").textContent).toContain(
    '"window_id": 42',
  );
  const long = "BASE64".repeat(100000);
  rerender(
    <ToolResult
      value={{
        result: {
          isError: true,
          structuredContent: {
            window_id: 42,
            data: long,
            windows: Array.from({ length: 1000 }, (_, window_id) => ({
              window_id,
              description: long,
            })),
          },
        },
      }}
    />,
  );
  const preview = screen.getByLabelText("结构化结果 JSON").textContent!;
  expect(preview.length).toBeLessThan(16100);
  expect(preview).toContain('"window_id": 42');
  expect(preview).not.toContain(long);
  expect(screen.getByText(/预览已截断/)).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toMatch(/执行失败/);
});

test("tool JSON input disables native text correction and spelling substitution", async () => {
  const user = userEvent.setup();
  render(
    <BackendPanel
      snapshot={snapshot}
      app={null}
      act={vi.fn()}
      read={vi.fn().mockResolvedValue(toolList)}
      mode="advanced"
    />,
  );
  await chooseTool(user);
  const input = screen.getByRole("textbox", { name: "工具参数 JSON" });
  expect(input.getAttribute("autocorrect")).toBe("off");
  expect(input.getAttribute("autocapitalize")).toBe("off");
  expect(input.getAttribute("spellcheck")).toBe("false");
});

test("replay reveals a newly selected detail once and respects reduced motion", async () => {
  const scroll = vi.fn();
  const originalScroll = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "scrollIntoView",
  );
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: scroll,
  });
  let reducedMotion = false;
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: reducedMotion })),
  );
  try {
    const user = userEvent.setup();
    const read = vi.fn(async (_command, args) => task(args.args.task_id));
    const props = { act: vi.fn(), read };
    const { rerender } = render(
      <Replay tasks={[task("first"), task("second")]} {...props} />,
    );
    expect(scroll).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /first.*成功/ }));
    expect(scroll).toHaveBeenCalledExactlyOnceWith({
      block: "nearest",
      behavior: "smooth",
    });
    rerender(
      <Replay
        tasks={[task("first"), task("second"), task("background-update")]}
        {...props}
      />,
    );
    expect(scroll).toHaveBeenCalledTimes(1);
    reducedMotion = true;
    await user.click(screen.getByRole("button", { name: /second.*成功/ }));
    expect(scroll).toHaveBeenCalledTimes(2);
    expect(scroll).toHaveBeenLastCalledWith({
      block: "nearest",
      behavior: "auto",
    });
  } finally {
    vi.unstubAllGlobals();
    if (originalScroll)
      Object.defineProperty(
        HTMLElement.prototype,
        "scrollIntoView",
        originalScroll,
      );
    else delete (HTMLElement.prototype as any).scrollIntoView;
  }
});

test("returning from System Settings refreshes permissions without implying backend denial", async () => {
  let granted = false;
  const read = vi.fn(async (command: string) => command === "permissions"
    ? { accessibility: granted, screen_recording: granted, graphical_session: true }
    : { available: true });
  const mutate = vi.fn();
  const view = render(<BackendPanel snapshot={snapshot} app={app} act={mutate} read={read} mode="permissions" />);
  expect(await screen.findByText("当前应用未获授权；不代表桌面后端未授权")).toBeTruthy();
  expect(screen.getByText("当前应用未获授权；截图由后端完成，无需为此重复授权")).toBeTruthy();
  const accessibilityRow = screen.getByText("辅助功能 · Macrun Desktop").closest(".requirement-row")!;
  expect(accessibilityRow.querySelector(".warn")).toBeNull();
  granted = true;
  fireEvent(window, new Event("focus"));
  expect(await screen.findByText("当前应用已授权；后端点击和输入仍使用后端自己的权限")).toBeTruthy();
  expect(accessibilityRow.querySelector(".ok")).toBeTruthy();
  expect(mutate).not.toHaveBeenCalled();
  view.unmount();
  const before = read.mock.calls.length;
  fireEvent(window, new Event("focus"));
  expect(read.mock.calls.length).toBe(before);
});
