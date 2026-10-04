// @vitest-environment jsdom
import React from "react";
import { afterEach, expect, test, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Approvals, Pairing, SafetyPanel, BackendPanel } from "../src/Features";
afterEach(cleanup);
const safety = {
  restrict_paths: true,
  roots: ["/work"],
  approval: "all",
  retention_days: 30,
  yield_until: 0,
};
test("approval buttons only send the selected task decision", async () => {
  const act = vi.fn().mockResolvedValue({ handled: true });
  const user = userEvent.setup();
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
    <SafetyPanel snapshot={{ safety } as any} act={act} disabled={false} />,
  );
  const input = screen.getByRole("textbox");
  await user.clear(input);
  await user.type(input, "/new");
  rerender(
    <SafetyPanel
      snapshot={{ safety: { ...safety, roots: ["/remote"] } } as any}
      act={act}
      disabled={false}
    />,
  );
  expect((input as HTMLTextAreaElement).value).toBe("/new");
  await user.click(screen.getByRole("button", { name: "保存安全设置" }));
  expect(act.mock.calls[0][1].args.roots).toEqual(["/new"]);
  expect((input as HTMLTextAreaElement).value).toBe("/new");
});
test("pairing advances only after successful exchange and starts worker", async () => {
  const act = vi
      .fn()
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ fingerprint: "pin" })
      .mockResolvedValue({}),
    user = userEvent.setup();
  render(<Pairing act={act} running={false} />);
  const input = screen.getByPlaceholderText("macrun://pair/…");
  await user.type(input, "macrun://pair/test");
  await user.click(screen.getByRole("button", { name: "安全配对" }));
  expect(screen.getByRole("button", { name: "安全配对" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "安全配对" }));
  await waitFor(() => expect(screen.getByText("pin")).toBeTruthy());
  expect(act).toHaveBeenCalledWith("start_worker");
  expect(
    screen.getByRole("button", { name: "继续" }).hasAttribute("disabled"),
  ).toBe(true);
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
      snapshot={{ backends: [{ name: "fixture" }] } as any}
      act={act}
      app={null}
    />,
  );
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
