// @vitest-environment jsdom
import React from "react";
import { afterEach, expect, test, vi } from "vitest";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Approvals, AllowRules, DesktopTiers } from "../src/Features";
import type { Snapshot, Task } from "../src/types";
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const waiting = (over: Partial<Task> = {}): Task => ({
  task_id: "cmd",
  kind: "exec.start",
  status: "awaiting_approval",
  arguments: { command: "ps aux | grep macrun", cwd: "/tmp" },
  started_at: Date.now(),
  approval_deadline: Date.now() + 41_000,
  ...over,
});

const snapshot = {
  safety: {
    restrict_paths: false,
    roots: [],
    approval: "direct",
    retention_days: 30,
    yield_until: 0,
    desktop: { observe: "allow", control: "allow", high: "allow" },
  },
  policy: { paused: false, desktop_enabled: true },
  backends: [
    {
      name: "computer",
      state: "ready",
      command: "cua-driver mcp",
      tool_count: 4,
      tiers: {
        get_window_state: "observe",
        zoom: "observe",
        click: "control",
        kill_app: "high",
      },
    },
  ],
  connection: { state: "connected", server: "example:7443", since: 0 },
  tasks: [],
  workspaces: [],
  today_summary: {},
  total_tasks: 0,
  active_count: 0,
  protocol: 2,
  version: "0.2.0",
} satisfies Snapshot;

test("approval prompt explains the risk, counts down and offers scoped approvals", async () => {
  const act = vi.fn().mockResolvedValue({ handled: true }),
    user = userEvent.setup();
  render(<Approvals tasks={[waiting()]} act={act} disabled={false} />);
  const prompt = within(screen.getByRole("region", { name: "等待你确认" }));
  expect(prompt.getByText("管道")).toBeTruthy();
  expect(prompt.getByText("未知程序 ps")).toBeTruthy();
  expect(prompt.getByRole("timer").textContent).toMatch(/^4[01] 秒后自动拒绝/);
  await user.click(prompt.getByRole("button", { name: "15 分钟内允许同类" }));
  expect(act.mock.calls.at(-1)?.[1]).toEqual({
    action: "approve",
    args: { task_id: "cmd", allow: true, scope: "similar" },
  });
  await user.click(prompt.getByRole("button", { name: "本次运行允许此目录" }));
  expect(act.mock.calls.at(-1)?.[1].args.scope).toBe("session");
});

test("approval countdown reaches zero without going negative", () => {
  vi.useFakeTimers();
  const start = Date.now();
  render(
    <Approvals
      tasks={[waiting({ approval_deadline: start + 2_000 })]}
      act={vi.fn()}
      disabled={false}
    />,
  );
  expect(screen.getByRole("timer").textContent).toMatch(/^2 秒/);
  act(() => {
    vi.advanceTimersByTime(5_000);
  });
  expect(screen.getByRole("timer").textContent).toMatch(/^0 秒/);
});

test("desktop approvals name the tier and the compact prompt pages through requests", async () => {
  const user = userEvent.setup();
  render(
    <Approvals
      compact
      tasks={[
        waiting(),
        waiting({
          task_id: "desk",
          kind: "mcp.call",
          desktop_tier: "high",
          arguments: { server: "computer", tool: "kill_app" },
        }),
      ]}
      act={vi.fn()}
      disabled={false}
    />,
  );
  expect(screen.getAllByRole("region", { name: "等待你确认" })).toHaveLength(1);
  // The menu bar keeps the long-lived "session" scope out of its narrow prompt.
  expect(screen.queryByRole("button", { name: /本次运行/ })).toBeNull();
  expect(screen.getByText("1 / 2")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "下一条待确认" }));
  expect(screen.getByText("computer · kill_app")).toBeTruthy();
  expect(screen.getByText("桌面 · 不可撤回的操作")).toBeTruthy();
  expect(screen.getByText("Agent 想操作桌面")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "上一条待确认" }));
  expect(screen.getByText("ps aux | grep macrun")).toBeTruthy();
});

test("nothing renders without waiting requests", () => {
  const { container } = render(
    <Approvals
      tasks={[{ ...waiting(), status: "running" }]}
      act={vi.fn()}
      disabled={false}
    />,
  );
  expect(container.textContent).toBe("");
});

test("desktop tiers list discovered tools and save one tier at a time", async () => {
  const act = vi.fn().mockResolvedValue(true),
    user = userEvent.setup();
  render(<DesktopTiers snapshot={snapshot} act={act} disabled={false} />);
  const high = within(screen.getByRole("group", { name: "桌面工具：不可撤回的操作" }));
  expect(high.getByRole("button", { name: "允许" }).getAttribute("aria-pressed")).toBe("true");
  expect(screen.getByText("get_window_state · zoom")).toBeTruthy();
  expect(screen.getByText("kill_app")).toBeTruthy();
  await user.click(high.getByRole("button", { name: "先问" }));
  expect(act).toHaveBeenCalledWith(
    "control",
    {
      action: "safety",
      args: {
        ...snapshot.safety,
        desktop: { observe: "allow", control: "allow", high: "confirm" },
      },
    },
    "桌面“不可撤回的操作”已设为“先问”",
  );
});

test("desktop tiers stay read-only until the worker reports a policy", () => {
  render(
    <DesktopTiers
      snapshot={{
        ...snapshot,
        safety: { ...snapshot.safety, desktop: undefined },
        backends: [],
      }}
      act={vi.fn()}
      disabled={false}
    />,
  );
  for (const button of screen.getAllByRole("button"))
    expect(button.hasAttribute("disabled")).toBe(true);
  expect(screen.getByText(/读取工具列表后显示/)).toBeTruthy();
});

test("temporary approvals describe their scope and can be revoked", async () => {
  const act = vi.fn().mockResolvedValue({ removed: true }),
    user = userEvent.setup();
  const { rerender } = render(
    <AllowRules
      rules={[
        {
          id: "r1",
          kind: "exec.start",
          scope: "similar",
          program: "git",
          cwd: "/work/app",
          created_at: 0,
          expires_at: Date.now() + 60_000,
        },
        {
          id: "r2",
          kind: "mcp.call",
          scope: "session",
          server: "computer",
          tier: "control",
          created_at: 0,
          expires_at: null,
        },
      ]}
      act={act}
      disabled={false}
    />,
  );
  expect(screen.getByText("git · /work/app 及子目录")).toBeTruthy();
  expect(screen.getByText("computer · 所有“点击和输入”工具")).toBeTruthy();
  expect(screen.getByText("直到执行器重启")).toBeTruthy();
  await user.click(screen.getAllByRole("button", { name: "撤销" })[0]);
  expect(act).toHaveBeenCalledWith(
    "control",
    { action: "revoke_rule", args: { rule_id: "r1" } },
    "已撤销临时允许",
  );
  rerender(<AllowRules rules={[]} act={act} disabled={false} />);
  expect(screen.getByText(/^暂无临时允许/)).toBeTruthy();
});
