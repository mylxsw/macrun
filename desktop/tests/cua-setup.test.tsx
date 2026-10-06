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
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CuaSetup } from "../src/CuaSetup";
import { Pairing } from "../src/Features";
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});
const ready = {
  state: "ready",
  version: "cua-driver fixture",
  configured: false,
};

test("detects missing driver, installs once and re-detects before offering configuration", async () => {
  let installed = false;
  let complete!: () => void;
  const read = vi.fn(async () =>
    installed ? ready : { state: "missing", detail: "尚未安装" },
  );
  const act = vi.fn(async () => {
    await new Promise<void>((r) => {
      complete = r;
    });
    installed = true;
    return ready;
  });
  render(<CuaSetup act={act} read={read} />);
  const install = await screen.findByRole("button", {
    name: "一键安装 Cua Driver",
  });
  fireEvent.click(install);
  fireEvent.click(install);
  expect(act).toHaveBeenCalledTimes(1);
  expect(install.hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("status").textContent).toContain("正在下载");
  complete();
  await screen.findByRole("button", { name: "接入 Macrun（空闲时重连）" });
  expect(read).toHaveBeenCalledTimes(2);
  expect(
    screen.queryByRole("button", { name: "一键安装 Cua Driver" }),
  ).toBeNull();
});

test("installation failures allow retry, and unsupported systems cannot install", async () => {
  const user = userEvent.setup();
  const act = vi
    .fn()
    .mockRejectedValueOnce(new Error("download failed"))
    .mockResolvedValue(true);
  const read = vi.fn().mockResolvedValue({ state: "missing" });
  const view = render(<CuaSetup act={act} read={read} />);
  await user.click(
    await screen.findByRole("button", { name: "一键安装 Cua Driver" }),
  );
  expect(screen.getByRole("alert").textContent).toContain("download failed");
  read.mockResolvedValue(ready);
  await user.click(screen.getByRole("button", { name: "一键安装 Cua Driver" }));
  expect(screen.queryByRole("alert")).toBeNull();
  view.unmount();
  render(
    <CuaSetup
      act={act}
      read={async () => ({ state: "unsupported", detail: "需要 macOS 14" })}
    />,
  );
  await screen.findByText("需要 macOS 14");
  expect(
    screen.queryByRole("button", { name: "一键安装 Cua Driver" }),
  ).toBeNull();
});

test.each([true, false])(
  "configuration reconnects only an idle previously running worker (%s)",
  async (running) => {
    const calls: string[] = [];
    const read = vi.fn(async (command) =>
      command === "app_state" ? { worker_running: running } : ready,
    );
    const act = vi.fn(async (command) => {
      calls.push(command);
      return true;
    });
    render(<CuaSetup act={act} read={read} />);
    await userEvent.setup().click(
      await screen.findByRole("button", {
        name: "接入 Macrun（空闲时重连）",
      }),
    );
    expect(calls).toEqual(
      running
        ? ["stop_worker", "configure_cua_driver", "start_worker"]
        : ["configure_cua_driver"],
    );
    if (running)
      expect(act).toHaveBeenCalledWith("stop_worker", { onlyIfIdle: true });
  },
);

test("active work prevents configuration and a failed save restores the original connection", async () => {
  const user = userEvent.setup();
  const read = vi.fn(async (command) =>
    command === "app_state" ? { worker_running: true } : ready,
  );
  const act = vi.fn(async (command) => {
    if (command === "stop_worker") throw new Error("仍有任务运行");
    return true;
  });
  render(<CuaSetup act={act} read={read} />);
  await user.click(
    await screen.findByRole("button", { name: "接入 Macrun（空闲时重连）" }),
  );
  expect(act).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("alert").textContent).toContain("仍有任务运行");
  act.mockImplementation(async (command) => {
    if (command === "configure_cua_driver") throw new Error("save failed");
    return true;
  });
  await user.click(
    screen.getByRole("button", { name: "接入 Macrun（空闲时重连）" }),
  );
  expect(act).toHaveBeenCalledWith("start_worker", undefined);
  expect(screen.getByRole("alert").textContent).toContain("save failed");
});

test("grant failure stays retryable and returning from System Settings refreshes detection", async () => {
  const read = vi.fn().mockResolvedValue({ ...ready, configured: true });
  const act = vi
    .fn()
    .mockRejectedValueOnce(new Error("permission denied"))
    .mockResolvedValue(true);
  const user = userEvent.setup();
  render(<CuaSetup act={act} read={read} />);
  await user.click(
    await screen.findByRole("button", { name: "授权 CuaDriver 截图与控制" }),
  );
  expect(screen.getByRole("alert").textContent).toContain("permission denied");
  await user.click(
    screen.getByRole("button", { name: "授权 CuaDriver 截图与控制" }),
  );
  expect(screen.getByText(/授权流程已结束/)).toBeTruthy();
  fireEvent.focus(window);
  await waitFor(() => expect(read).toHaveBeenCalledTimes(4));
});

test("grant waiting does not block detection or configuration, and restart status can recover", async () => {
  let finish!: () => void;
  const read = vi.fn(async (command) =>
    command === "app_state" ? { worker_running: false } : ready,
  );
  const act = vi.fn(async (command) => {
    if (command === "grant_cua_permissions")
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
    return true;
  });
  render(<CuaSetup act={act} read={read} />);
  const grant = await screen.findByRole("button", {
    name: "授权 CuaDriver 截图与控制",
  });
  fireEvent.click(grant);
  fireEvent.click(grant);
  expect(act).toHaveBeenCalledTimes(1);
  expect(grant.hasAttribute("disabled")).toBe(true);
  const detect = screen.getByRole("button", { name: "重新检测" });
  expect(detect.hasAttribute("disabled")).toBe(false);
  expect(screen.getByText(/可继续其他操作/)).toBeTruthy();
  read.mockResolvedValue({
    ...ready,
    permissions: { accessibility: true, screen_recording: null },
  });
  fireEvent.focus(window);
  await screen.findByText(/辅助功能已授权；屏幕录制暂无法确认/);
  read.mockResolvedValue({
    ...ready,
    permissions: { accessibility: true, screen_recording: true },
  });
  await userEvent.setup().click(detect);
  await screen.findByText(/辅助功能已授权；屏幕录制已授权/);
  expect(screen.queryByText(/等待 CuaDriver 授权与重启/)).toBeNull();
  expect(screen.getByText(/两项权限已授权/)).toBeTruthy();
  expect(grant.hasAttribute("disabled")).toBe(true);
  read.mockImplementation(async (command) =>
    command === "app_state" ? { worker_running: false } : ready,
  );
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "接入 Macrun（空闲时重连）" }));
  expect(act).toHaveBeenCalledWith("configure_cua_driver", undefined);
  await reactAct(async () => {
    finish();
  });
  expect(grant.hasAttribute("disabled")).toBe(false);
});

test("visible polling refreshes after a restart without overlapping reads and cleans up", async () => {
  vi.useFakeTimers();
  let finish!: (value: unknown) => void;
  const read = vi.fn(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const visibility = vi
    .spyOn(document, "visibilityState", "get")
    .mockReturnValue("visible");
  const view = render(<CuaSetup act={vi.fn()} read={read} />);
  await reactAct(async () => {});
  await reactAct(async () => {
    fireEvent.focus(window);
    await vi.advanceTimersByTimeAsync(10000);
  });
  expect(read).toHaveBeenCalledTimes(1);
  await reactAct(async () => {
    finish(ready);
  });
  visibility.mockReturnValue("hidden");
  await reactAct(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(read).toHaveBeenCalledTimes(1);
  read.mockResolvedValue({
    ...ready,
    permissions: { accessibility: false, screen_recording: true },
  });
  visibility.mockReturnValue("visible");
  await reactAct(async () => {
    fireEvent(document, new Event("visibilitychange"));
  });
  expect(screen.getByText(/辅助功能未授权；屏幕录制已授权/)).toBeTruthy();
  read.mockResolvedValue({
    ...ready,
    permissions: { accessibility: true, screen_recording: true },
  });
  await reactAct(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(screen.getByText(/辅助功能已授权；屏幕录制已授权/)).toBeTruthy();
  view.unmount();
  const calls = read.mock.calls.length;
  fireEvent.focus(window);
  fireEvent(document, new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(10000);
  expect(read).toHaveBeenCalledTimes(calls);
});

test("failed detection clears stale permission claims and recovers on focus", async () => {
  const read = vi.fn().mockResolvedValue({
    ...ready,
    permissions: { accessibility: true, screen_recording: true },
  });
  render(<CuaSetup act={vi.fn()} read={read} />);
  await screen.findByText(/辅助功能已授权；屏幕录制已授权/);
  read.mockRejectedValue(new Error("driver restarting"));
  fireEvent.focus(window);
  expect((await screen.findByRole("alert")).textContent).toContain(
    "driver restarting",
  );
  expect(
    screen.getByText(/辅助功能暂无法确认；屏幕录制暂无法确认/),
  ).toBeTruthy();
  read.mockResolvedValue({
    ...ready,
    permissions: { accessibility: true, screen_recording: true },
  });
  fireEvent.focus(window);
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  expect(screen.getByText(/辅助功能已授权；屏幕录制已授权/)).toBeTruthy();
});

test("unmounting during a grant stops detection and safely accepts a late result", async () => {
  let finish!: () => void;
  const read = vi.fn().mockResolvedValue(ready);
  const act = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(<CuaSetup act={act} read={read} />);
  fireEvent.click(
    await screen.findByRole("button", { name: "授权 CuaDriver 截图与控制" }),
  );
  view.unmount();
  await reactAct(async () => {
    finish();
  });
  expect(read).toHaveBeenCalledTimes(1);
});

test("failed or unknown executor state cannot configure and false actions are errors", async () => {
  const read = vi.fn(async (command) => (command === "app_state" ? {} : ready));
  const act = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<CuaSetup act={act} read={read} />);
  await user.click(
    await screen.findByRole("button", { name: "接入 Macrun（空闲时重连）" }),
  );
  expect(act).not.toHaveBeenCalled();
  expect(screen.getByRole("alert").textContent).toContain("状态尚未就绪");
  await user.click(
    screen.getByRole("button", { name: "授权 CuaDriver 截图与控制" }),
  );
  expect(screen.getByRole("alert").textContent).toContain("操作未完成");
});

test("pairing waits for checks and retries a saved pairing without exchanging again", async () => {
  let finish!: (value: unknown) => void;
  let starts = 0;
  const act = vi.fn(async (command: string) => {
    if (command === "pair")
      return { fingerprint: "saved-pin", connection_id: "new-server" };
    if (command === "start_worker") return ++starts === 1 ? undefined : true;
    if (command === "connection_check")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return true;
  });
  render(<Pairing act={act} running={false} />);
  const user = userEvent.setup();
  await user.type(
    screen.getByPlaceholderText("macrun://pair/…"),
    "macrun://pair/test",
  );
  await user.click(screen.getByRole("button", { name: "连接", exact: true }));
  expect(screen.getByRole("alert").textContent).toContain("配对已保存");
  await user.click(screen.getByRole("button", { name: "重新检查" }));
  expect(screen.getByRole("status").textContent).toContain("等待连接就绪");
  expect(
    screen.getByRole("button", { name: "继续" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(act).toHaveBeenCalledWith("connection_check", {
    connectionId: "new-server",
  });
  finish({
    checks: [{ name: "认证", ok: true }],
    phase: "connected",
    error: "",
  });
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "继续" }).hasAttribute("disabled"),
    ).toBe(false),
  );
  expect(act.mock.calls.filter(([c]) => c === "pair")).toHaveLength(1);
});
