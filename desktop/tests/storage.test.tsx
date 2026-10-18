// @vitest-environment jsdom
import React from "react";
import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { SettingsPage } from "../src/SettingsPage";
import type { Snapshot } from "../src/types";

afterEach(cleanup);
const snapshot: Snapshot = {
  safety: {
    restrict_paths: false,
    roots: [],
    approval: "direct",
    retention_days: 30,
    yield_until: 0,
  },
  policy: { paused: false, desktop_enabled: true },
  connection: { state: "connected", server: "test:7443", since: 0 },
  tasks: [],
  workspaces: [],
  backends: [],
  today_summary: {},
  total_tasks: 0,
  active_count: 0,
  version: "test",
  protocol: 2,
};
function page(
  over: {
    snapshot?: Snapshot | null;
    busy?: boolean;
    available?: boolean;
  } = {},
) {
  const act = vi.fn().mockResolvedValue({});
  render(
    <SettingsPage
      app={null}
      snapshot={snapshot}
      busy={false}
      available
      act={act}
      refresh={vi.fn()}
      setError={vi.fn()}
      onPair={vi.fn()}
      {...over}
    />,
  );
  return act;
}

test("storage controls load safe legacy defaults and save numeric policies without changing other rules", () => {
  const act = page();
  const retention = screen.getByLabelText("执行快照保留") as HTMLSelectElement;
  const budget = screen.getByLabelText(
    "每台服务器的任务存储上限",
  ) as HTMLSelectElement;
  expect(retention.value).toBe("1");
  expect(budget.value).toBe("10");
  fireEvent.change(retention, { target: { value: "7" } });
  expect(act.mock.calls.at(-1)?.slice(0, 2)).toEqual([
    "control",
    {
      action: "safety",
      args: { ...snapshot.safety, snapshot_retention_days: 7 },
    },
  ]);
  fireEvent.change(budget, { target: { value: "5" } });
  expect(act.mock.calls.at(-1)?.[1]).toEqual({
    action: "safety",
    args: { ...snapshot.safety, storage_limit_gb: 5 },
  });
  fireEvent.click(screen.getByRole("button", { name: "立即清理" }));
  expect(act.mock.calls.at(-1)?.[1]).toEqual({ action: "prune", args: {} });
  expect(screen.getByText(/原项目和同步目录需自行管理/)).toBeTruthy();
});

test("storage usage shows sampled totals, reclaimed bytes, remaining pressure and partial failure", () => {
  page({
    snapshot: {
      ...snapshot,
      safety: {
        ...snapshot.safety,
        snapshot_retention_days: 7,
        storage_limit_gb: 5,
      },
      storage: {
        scanned_at: 1_700_000_000_000,
        task_bytes: 2 * 1024 ** 3,
        workspace_bytes: 1024 ** 3,
        limit_bytes: 1024 ** 3,
        over_limit: true,
        reclaimed_bytes: 512 * 1024 ** 2,
        errors: 2,
        removed: 1,
        workspaces_removed: 3,
      },
    },
  });
  expect(
    (screen.getByLabelText("执行快照保留") as HTMLSelectElement).value,
  ).toBe("7");
  expect(
    screen.getByText("任务数据 2.00 GiB，其中执行快照 1.00 GiB"),
  ).toBeTruthy();
  expect(screen.getByText(/上次回收 512.00 MiB/)).toBeTruthy();
  expect(screen.getByText(/仍超出容量上限/)).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("有 2 项未能清理");
});

test.each([{ busy: true }, { available: false }, { snapshot: null }])(
  "storage controls stay disabled when unavailable: %j",
  (over) => {
    page(over);
    expect(
      (screen.getByLabelText("执行快照保留") as HTMLSelectElement).disabled,
    ).toBe(true);
    expect(
      (screen.getByLabelText("每台服务器的任务存储上限") as HTMLSelectElement)
        .disabled,
    ).toBe(true);
    expect(screen.queryByText(/最近检查/)).toBeNull();
  },
);
