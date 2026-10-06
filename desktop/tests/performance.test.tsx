// @vitest-environment jsdom
import React from "react";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
  bytes,
  millis,
  Performance,
  PerformanceDetail,
  Trend,
  type Dashboard,
} from "../src/Performance";
import type { Metrics } from "../src/types";
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
const metric: Metrics = {
  schema_version: 1,
  origin: "worker",
  wall_ms: 100,
  complete: true,
  phases: [{ name: "process_run", wall_ms: 50, count: 1, max_ms: 50 }],
  bytes: { payload: 1024 },
  files: {},
  outputs: [{ name: "build/app", bytes: 4096, status: "complete" }],
  samples: [
    { offset_ms: 1, payload_bytes: 512 },
    { offset_ms: 100, payload_bytes: 1024 },
  ],
  first_payload_offset_ms: 1,
  last_payload_offset_ms: 100,
};
const task = {
  task_id: "abc",
  kind: "exec.start",
  status: "succeeded",
  started_at: 10,
  ended_at: 110,
  connection_id: "primary",
  connection_name: "Example",
  arguments: {},
  metrics: metric,
};
const dashboard = (patch: Partial<Dashboard> = {}): Dashboard => ({
  as_of: 100,
  total: 1,
  measured: 1,
  counts: { succeeded: 1 },
  success_denominator: 1,
  latency: { n: 1, p50_ms: 100, p95_ms: 100, p99_ms: 100 },
  bytes: { payload: 1024 },
  phases_ms: {},
  operations: [task],
  next_cursor: null,
  series: [
    { time: 10, count: 1, p50_ms: 100, p95_ms: 100, payload_bytes: 1024 },
  ],
  projects: { id: "example" },
  errors: [],
  ...patch,
});
beforeEach(() => {
  bridge.invoke.mockReset();
  bridge.invoke.mockImplementation(async (command, args) =>
    command === "export_metrics"
      ? { count: 1200, cancelled: false }
      : args.action === "metrics_detail"
        ? task
        : dashboard(),
  );
});
afterEach(cleanup);

test("90-day filter fits the native maximum half-open window exactly", async () => {
  render(<Performance active app={null} />);
  await screen.findByText(/1\/1 个任务有性能指标/);
  fireEvent.change(screen.getByLabelText("时间"), { target: { value: "90" } });
  await waitFor(() =>
    expect(
      bridge.invoke.mock.calls.at(-1)?.[1].args.to_ms -
        bridge.invoke.mock.calls.at(-1)?.[1].args.from_ms,
    ).toBe(90 * 86400000),
  );
});

test("missing values, zero byte outputs and precise counter strings remain distinct", () => {
  expect(bytes(undefined)).toBe("未采集");
  expect(bytes(0)).toBe("0 B");
  expect(bytes("9007199254740993")).toContain("TiB");
  expect(bytes(-1)).toBe("未采集");
  expect(millis(null)).toBe("未采集");
  expect(millis(0)).toBe("0.0 ms");
  render(<PerformanceDetail />);
  expect(screen.getByText(/旧任务不会补造/)).toBeTruthy();
});
test("origins have independent phase tracks and declared outputs and speeds are visible", () => {
  render(
    <PerformanceDetail
      metrics={metric}
      server={{
        ...metric,
        origin: "server",
        wall_ms: 200,
        phases: [{ name: "scan", wall_ms: 120, count: 1, max_ms: 120 }],
      }}
    />,
  );
  expect(screen.getByText("Mac 执行器")).toBeTruthy();
  expect(screen.getByText("服务器")).toBeTruthy();
  expect(screen.getByText(/不能直接相加/)).toBeTruthy();
  expect(screen.getByText(/build\/app/)).toBeTruthy();
  expect(screen.getByText(/KiB\/s/)).toBeTruthy();
  expect(screen.getAllByRole("meter")).toHaveLength(2);
});
test("trend breaks across missing samples instead of drawing zero latency", () => {
  const { container } = render(
    <Trend title="latency" values={[10, null, 20]} />,
  );
  expect(container.querySelectorAll("polyline")).toHaveLength(2);
  expect(screen.getByText(/无样本区间留空/)).toBeTruthy();
});
test("offline history supports native full-filter export and cancellation", async () => {
  render(<Performance active app={null} />);
  await screen.findByText(/1\/1 个任务有性能指标/);
  expect(screen.getByText(/执行器已停止/)).toBeTruthy();
  fireEvent.click(screen.getByText("导出 JSON"));
  await screen.findByText("已导出 1200 条记录");
  expect(bridge.invoke).toHaveBeenCalledWith(
    "export_metrics",
    expect.objectContaining({
      format: "json",
      args: expect.objectContaining({ from_ms: expect.any(Number) }),
    }),
  );
  bridge.invoke.mockResolvedValueOnce({ cancelled: true });
  fireEvent.click(screen.getByText("导出 CSV"));
  await waitFor(() => expect(screen.queryByText(/已导出/)).toBeNull());
});
test("filters discard late responses and clear stale statistics", async () => {
  let resolve!: (v: Dashboard) => void;
  bridge.invoke.mockImplementationOnce(
    () => new Promise<Dashboard>((done) => (resolve = done)),
  );
  render(<Performance active app={null} />);
  fireEvent.change(screen.getByLabelText("操作"), {
    target: { value: "sync" },
  });
  await screen.findByText(/1\/1 个任务有性能指标/);
  await act(async () => resolve(dashboard({ total: 9999 })));
  expect(screen.queryByText(/9999/)).toBeNull();
  expect(bridge.invoke.mock.calls.at(-1)?.[1].args.kind).toBe("sync");
});
test("pagination uses stable range and details ignore responses after close", async () => {
  const cursor = { started_at: 10, connection_id: "primary", task_id: "abc" };
  bridge.invoke.mockResolvedValueOnce(dashboard({ next_cursor: cursor }));
  render(<Performance active app={null} />);
  await screen.findByText(/1\/1 个任务有性能指标/);
  const first = bridge.invoke.mock.calls[0][1].args;
  fireEvent.click(screen.getByText("下一页"));
  await screen.findByText("第 2 页");
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledTimes(2));
  const second = bridge.invoke.mock.calls[1][1].args;
  expect(second.cursor).toEqual(cursor);
  expect(second.from_ms).toBe(first.from_ms);
  expect(second.to_ms).toBe(first.to_ms);
  let resolve!: (v: typeof task) => void;
  bridge.invoke.mockImplementationOnce(
    () => new Promise<typeof task>((done) => (resolve = done)),
  );
  fireEvent.click(screen.getByRole("button", { name: "查看 abc 性能" }));
  fireEvent.click(screen.getByText("关闭详情"));
  await act(async () => resolve(task));
  expect(screen.queryByText("性能详情")).toBeNull();
});
test("malformed and degraded histories offer retry and missing success ratios", async () => {
  bridge.invoke.mockResolvedValueOnce({});
  render(<Performance active app={null} />);
  await screen.findByRole("alert");
  fireEvent.click(screen.getByText("重试"));
  await screen.findByText(/1\/1 个任务有性能指标/);
  bridge.invoke.mockResolvedValueOnce(
    dashboard({
      total: 0,
      measured: 0,
      operations: [],
      latency: { n: 0, p50_ms: null, p95_ms: null, p99_ms: null },
      success_denominator: 0,
      errors: [{ connection_id: "broken", message: "索引不可读" }],
    }),
  );
  fireEvent.click(screen.getByText("刷新"));
  await screen.findByText(/索引不可读/);
  expect(screen.getByText("0 / —")).toBeTruthy();
  expect(screen.getByText("暂无匹配的性能历史。")).toBeTruthy();
});
