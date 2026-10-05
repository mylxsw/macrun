// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Snapshot, Task } from "../src/types";
import {
  readTaskPage,
  useTaskHistory,
  useReplayHistory,
} from "../src/taskHistory";
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
const task = (id: string, status = "succeeded"): Task => ({
  task_id: id,
  kind: "exec.start",
  status,
  arguments: { command: id },
  started_at: 10,
  ended_at: 20,
});
const page = (
  tasks: Task[],
  next_cursor: { started_at: number; task_id: string } | null = null,
) => ({
  tasks,
  total: 1500,
  filtered_total: tasks.length,
  counts: { succeeded: tasks.length },
  next_cursor,
});
const snapshot = (tasks: Task[]) => ({ tasks, total_tasks: 1500 }) as Snapshot;
const initial = () => ({
  enabled: true,
  available: true,
  snapshot: snapshot([task("first")]),
  filter: "all",
  query: "",
  selected: "",
});
const deferred = <T,>() => {
  let resolve!: (result: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
};
beforeEach(() => {
  bridge.invoke.mockReset();
  bridge.invoke.mockImplementation(async (_command, args) =>
    args.action === "task_list"
      ? page([task("first")])
      : {
          ...task(args.args.task_id),
          output: { text: `output-${args.args.task_id}` },
        },
  );
});
afterEach(cleanup);

test("malformed history responses become recoverable read errors", async () => {
  bridge.invoke.mockResolvedValueOnce({});
  const { result } = renderHook(() => useTaskHistory(initial()));
  await waitFor(() =>
    expect(result.current.error).toContain("任务历史返回格式无效"),
  );
  expect(result.current.tasks).toEqual([]);
  act(() => result.current.reload());
  await waitFor(() =>
    expect(result.current.selectedTask?.output_tail).toBe("output-first"),
  );
  expect(result.current.error).toBe("");
  bridge.invoke.mockResolvedValueOnce({
    ...page([task("invalid")]),
    next_cursor: {},
  });
  await expect(readTaskPage({})).rejects.toThrow("任务历史返回格式无效");
});

test("a slow obsolete status request cannot overwrite a newer filter", async () => {
  const old = deferred<ReturnType<typeof page>>();
  bridge.invoke.mockImplementation(async (_command, args) => {
    if (args.action === "task_detail") return task(args.args.task_id);
    if (args.args.status === "failed") return old.promise;
    return page([task(args.args.status === "succeeded" ? "newer" : "first")]);
  });
  const props = initial();
  const { result, rerender } = renderHook(useTaskHistory, {
    initialProps: props,
  });
  await waitFor(() => expect(result.current.tasks[0]?.task_id).toBe("first"));
  rerender({ ...props, filter: "failed" });
  await waitFor(() => expect(result.current.loading).toBe(true));
  expect(result.current.tasks[0]?.task_id).toBe("first");
  expect(result.current.stale).toBe(true);
  rerender({ ...props, filter: "succeeded" });
  await waitFor(() => expect(result.current.tasks[0]?.task_id).toBe("newer"));
  await act(async () => old.resolve(page([task("obsolete", "failed")])));
  expect(result.current.tasks[0].task_id).toBe("newer");
});

test("selection discards late details and caches text without large MCP result data", async () => {
  const old = deferred<Task>();
  bridge.invoke.mockImplementation(async (_command, args) => {
    if (args.action === "task_list")
      return page([task("first"), task("second")]);
    if (args.args.task_id === "first") return old.promise;
    return {
      ...task("second"),
      result: {
        exit_code: 7,
        content: [{ type: "image", data: "large-image-fixture" }],
      },
      output: { text: "last line" },
    };
  });
  const props = { ...initial(), selected: "first" };
  const { result, rerender } = renderHook(useTaskHistory, {
    initialProps: props,
  });
  await waitFor(() => expect(result.current.detailLoading).toBe(true));
  rerender({ ...props, selected: "second" });
  await waitFor(() =>
    expect(result.current.selectedTask?.output_tail).toBe("last line"),
  );
  expect(result.current.selectedTask?.result).toEqual({ exit_code: 7 });
  expect(JSON.stringify(result.current.selectedTask)).not.toContain(
    "large-image-fixture",
  );
  await act(async () => old.resolve(task("first")));
  expect(result.current.selectedTask?.task_id).toBe("second");
  rerender({ ...props, enabled: false, selected: "second" });
  rerender({ ...props, selected: "second" });
  await waitFor(() => expect(result.current.detailLoading).toBe(false));
  expect(
    bridge.invoke.mock.calls.filter(
      ([, args]) =>
        args.action === "task_detail" && args.args.task_id === "second",
    ),
  ).toHaveLength(1);
});

test("detail errors are visible and retry reads the selected task without replaying it", async () => {
  let fail = true;
  bridge.invoke.mockImplementation(async (_command, args) => {
    if (args.action === "task_list") return page([task("first")]);
    if (fail) throw new Error("output unavailable");
    return { ...task("first"), output: { text: "recovered output" } };
  });
  const { result } = renderHook(() => useTaskHistory(initial()));
  await waitFor(() =>
    expect(result.current.detailError).toContain("output unavailable"),
  );
  expect(result.current.selectedTask?.task_id).toBe("first");
  fail = false;
  act(() => result.current.reload());
  await waitFor(() =>
    expect(result.current.selectedTask?.output_tail).toBe("recovered output"),
  );
  expect(result.current.detailError).toBe("");
  expect(
    bridge.invoke.mock.calls.every(([, args]) =>
      ["task_list", "task_detail"].includes(args.action),
    ),
  ).toBe(true);
});

test("live state updates preserve history scroll while moving a page resets it", async () => {
  bridge.invoke.mockImplementation(async (_command, args) =>
    args.action === "task_list"
      ? page(
          [task(args.args.cursor ? "second" : "first")],
          args.args.cursor ? null : { started_at: 10, task_id: "first" },
        )
      : task(args.args.task_id),
  );
  const props = initial();
  const { result, rerender } = renderHook(useTaskHistory, {
    initialProps: props,
  });
  await waitFor(() => expect(result.current.next).toBeDefined());
  const list = document.createElement("section");
  list.scrollTo = vi.fn();
  result.current.list.current = list;
  rerender({ ...props, snapshot: snapshot([task("first", "failed")]) });
  await waitFor(() => expect(result.current.loading).toBe(false));
  expect(list.scrollTo).not.toHaveBeenCalled();
  act(() => result.current.next!());
  await waitFor(() => expect(result.current.tasks[0]?.task_id).toBe("second"));
  expect(list.scrollTo).toHaveBeenCalledWith({ top: 0 });
});

test("replay discovers older MCP calls using history independently of the snapshot", async () => {
  bridge.invoke.mockResolvedValue(
    page([{ ...task("old-call", "unknown"), kind: "mcp.call" }]),
  );
  const { result } = renderHook(() =>
    useReplayHistory(true, true, snapshot([task("latest-exec")])),
  );
  await waitFor(() =>
    expect(result.current.tasks[0]?.task_id).toBe("old-call"),
  );
  expect(bridge.invoke).toHaveBeenCalledWith("control", {
    action: "task_list",
    args: { kind: "mcp.call", limit: 8 },
  });
});

test("MCP failures retain bounded UTF-8 text and an error notice without caching images", async () => {
  const oldTask = { ...task("old-mcp", "failed"), kind: "mcp.call" };
  bridge.invoke.mockImplementation(async (_command, args) =>
    args.action === "task_list"
      ? page([oldTask])
      : {
          ...oldTask,
          result: {
            server: "fixture",
            result: {
              isError: true,
              content: [
                { type: "text", text: "discarded-prefix" },
                { type: "image", data: "large-image-must-not-be-cached" },
                { type: "text", text: "失败".repeat(4000) + "：权限不足" },
              ],
            },
          },
        },
  );
  const { result } = renderHook(() => useTaskHistory(initial()));
  await waitFor(() =>
    expect(result.current.selectedTask?.error?.message).toContain(
      "工具返回执行错误",
    ),
  );
  const selected = result.current.selectedTask!;
  expect(selected.output_tail).toMatch(/：权限不足$/);
  expect(selected.output_tail).not.toContain("discarded-prefix");
  expect(selected.output_tail).not.toContain("�");
  expect(
    new TextEncoder().encode(selected.output_tail).length,
  ).toBeLessThanOrEqual(8192);
  expect(selected.result).toBeUndefined();
  expect(JSON.stringify(selected)).not.toContain(
    "large-image-must-not-be-cached",
  );
});

test("MCP errors without text still explain failure and successful text is preserved", async () => {
  const oldTask = { ...task("old-mcp", "failed"), kind: "mcp.call" };
  bridge.invoke.mockImplementation(async (_command, args) =>
    args.action === "task_list"
      ? page([oldTask])
      : args.args.task_id === "success"
        ? {
            ...oldTask,
            task_id: "success",
            status: "succeeded",
            result: {
              result: { content: [{ type: "text", text: "工具已完成" }] },
            },
          }
        : {
            ...oldTask,
            result: {
              result: {
                isError: true,
                content: [{ type: "image", data: "discard" }],
              },
            },
          },
  );
  const props = initial();
  const { result, rerender } = renderHook(useTaskHistory, {
    initialProps: props,
  });
  await waitFor(() =>
    expect(result.current.selectedTask?.error?.message).toContain(
      "工具返回执行错误",
    ),
  );
  rerender({ ...props, selected: "success" });
  await waitFor(() =>
    expect(result.current.selectedTask?.output_tail).toBe("工具已完成"),
  );
  expect(result.current.selectedTask?.error).toBeUndefined();
});

test("live output refresh keeps the confirmed detail while the next read is pending", async () => {
  let hold = false;
  const next = deferred<any>();
  bridge.invoke.mockImplementation(async (_command, args) => {
    if (args.action === "task_list") return page([task("live", "running")]);
    if (hold) return next.promise;
    return { ...task("live", "running"), output: { text: "confirmed output" } };
  });
  const props = {
    ...initial(),
    selected: "live",
    snapshot: snapshot([task("live", "running")]),
  };
  const { result, rerender } = renderHook(useTaskHistory, {
    initialProps: props,
  });
  await waitFor(() =>
    expect(result.current.selectedTask?.output_tail).toBe("confirmed output"),
  );
  hold = true;
  rerender({
    ...props,
    snapshot: snapshot([
      { ...task("live", "running"), output_tail: "incoming" },
    ]),
  });
  await waitFor(() => expect(result.current.detailLoading).toBe(true));
  expect(result.current.detailRefreshing).toBe(true);
  expect(result.current.selectedTask?.output_tail).toBe("confirmed output");
  await act(async () =>
    next.resolve({
      ...task("live", "running"),
      output: { text: "new output" },
    }),
  );
  expect(result.current.selectedTask?.output_tail).toBe("new output");
});
