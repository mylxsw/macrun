import { tr } from "./i18n.mjs";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { active, selectTasks } from "./model.mjs";
import type { Snapshot, Task } from "./types";

export type TaskCursor = { started_at: number; task_id: string };
export type TaskPage = {
  tasks: Task[];
  total: number;
  filtered_total: number;
  counts: Record<string, number>;
  next_cursor: TaskCursor | null;
};
export const TASK_PAGE_SIZE = 50;
type TaskDetail = Omit<Task, "result"> & {
  output?: { text?: string };
  result?: Task["result"] & {
    result?: {
      isError?: boolean;
      content?: { type?: unknown; text?: unknown }[];
    };
  };
};

// Slice before encoding so even a very large text block uses a bounded buffer.
function textTail(text: string): string {
  const bytes = new TextEncoder().encode(text.slice(-8192));
  let start = Math.max(0, bytes.length - 8192);
  while (start < bytes.length && (bytes[start] & 0xc0) === 0x80) start++;
  return new TextDecoder().decode(bytes.subarray(start));
}

export const readTaskPage = (args: {
  limit?: number;
  status?: string;
  query?: string;
  kind?: string;
  cursor?: TaskCursor | null;
}) =>
  invoke<TaskPage>("control", { action: "task_list", args }).then((page) => {
    const count = (value: unknown) =>
      typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
    if (
      !page ||
      !Array.isArray(page.tasks) ||
      !count(page.total) ||
      !count(page.filtered_total) ||
      !page.counts ||
      typeof page.counts !== "object" ||
      Object.values(page.counts).some((value) => !count(value)) ||
      page.tasks.some(
        (task) =>
          !task ||
          typeof task.task_id !== "string" ||
          typeof task.kind !== "string" ||
          typeof task.status !== "string" ||
          !count(task.started_at) ||
          !task.arguments ||
          typeof task.arguments !== "object",
      ) ||
      (page.next_cursor !== null &&
        (!page.next_cursor ||
          !count(page.next_cursor.started_at) ||
          typeof page.next_cursor.task_id !== "string"))
    ) {
      throw new Error(tr("任务历史返回格式无效，请重新读取。"));
    }
    return page;
  });

function fallbackPage(
  snapshot: Snapshot | null,
  status: string,
  query: string,
  index = 0,
  kind = "",
): TaskPage {
  const matches = (
    selectTasks(snapshot?.tasks || [], "all", query.trim()) as Task[]
  ).filter((t) => !kind || t.kind === kind);
  const filtered = selectTasks(matches, status, "") as Task[];
  const rows = filtered.slice(
    index * TASK_PAGE_SIZE,
    (index + 1) * TASK_PAGE_SIZE,
  );
  const last = rows.at(-1);
  return {
    tasks: rows,
    total: snapshot?.total_tasks || 0,
    filtered_total: filtered.length,
    counts: matches.reduce<Record<string, number>>((counts, t) => {
      counts[t.status] = (counts[t.status] || 0) + 1;
      return counts;
    }, {}),
    next_cursor:
      last && filtered.length > (index + 1) * TASK_PAGE_SIZE
        ? { started_at: last.started_at, task_id: last.task_id }
        : null,
  };
}

export function useTaskHistory({
  enabled,
  available,
  snapshot,
  filter,
  kind = "",
  connectionId = "",
  query,
  selected,
}: {
  enabled: boolean;
  available: boolean;
  snapshot: Snapshot | null;
  filter: string;
  kind?: string;
  connectionId?: string;
  query: string;
  selected: string;
}) {
  const [search, setSearch] = useState(query);
  const [cursors, setCursors] = useState<(TaskCursor | null)[]>([null]);
  const [index, setIndex] = useState(0);
  const [page, setPage] = useState<{ key: string; value: TaskPage } | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [detail, setDetail] = useState<Task | null>(null);
  const [detailError, setDetailError] = useState("");
  const [detailLoading, setDetailLoading] = useState(false);
  const [retry, setRetry] = useState(0);
  const terminal = useRef(new Map<string, Task>());
  const list = useRef<HTMLElement>(null);
  const signature = `${snapshot?.total_tasks}:${snapshot?.tasks.map((t) => `${t.task_id}:${t.status}`).join(",")}`;
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), 150);
    return () => clearTimeout(timer);
  }, [query]);
  useEffect(() => {
    setIndex(0);
    setCursors([null]);
  }, [filter, kind, search, available, connectionId]);
  const cursor = cursors[index];
  const requestKey = JSON.stringify([
    filter,
    kind,
    search,
    cursor,
    connectionId,
  ]);
  useEffect(() => {
    list.current?.scrollTo?.({ top: 0 });
  }, [requestKey]);
  useEffect(() => {
    if (!enabled || !available) return;
    let cancelled = false;
    setLoading(true);
    setError("");
    readTaskPage({
      limit: TASK_PAGE_SIZE,
      status: filter,
      query: search,
      ...(kind ? { kind } : {}),
      ...(connectionId ? { connection_id: connectionId } : {}),
      cursor,
    })
      .then((result) => {
        if (!cancelled) setPage({ key: requestKey, value: result });
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    enabled,
    available,
    filter,
    kind,
    search,
    cursor,
    signature,
    retry,
    connectionId,
  ]);
  const searchPending = search !== query.trim();
  const current = available
    ? page
      ? page.value
      : {
          tasks: [],
          total: snapshot?.total_tasks || 0,
          filtered_total: 0,
          counts: {},
          next_cursor: null,
        }
    : fallbackPage(
        connectionId && snapshot
          ? {
              ...snapshot,
              tasks: snapshot.tasks.filter(
                (t) => t.connection_id === connectionId,
              ),
            }
          : snapshot,
        filter,
        query,
        index,
        kind,
      );
  const selectedId = selected || current.tasks[0]?.task_id || "";
  const live = snapshot?.tasks.find((t) => t.task_id === selectedId);
  useEffect(() => {
    setDetailError("");
    if (!enabled || !selectedId) {
      setDetailLoading(false);
      return;
    }
    if (!available) {
      setDetailLoading(false);
      return;
    }
    const cached = terminal.current.get(selectedId);
    if (cached) {
      setDetail(cached);
      setDetailLoading(false);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    invoke<TaskDetail>("control", {
      action: "task_detail",
      args: { task_id: selectedId, tail_bytes: 8192 },
    })
      .then((record) => {
        if (cancelled) return;
        // History details need text and exit status, not potentially large MCP images.
        const toolResult =
          record.kind === "mcp.call" ? record.result?.result : undefined;
        const toolText = Array.isArray(toolResult?.content)
          ? toolResult.content.reduce(
              (tail, block) =>
                block.type === "text" && typeof block.text === "string"
                  ? textTail(tail + (tail ? "\n" : "") + textTail(block.text))
                  : tail,
              "",
            )
          : "";
        const task: Task = {
          task_id: record.task_id,
          connection_id: record.connection_id,
          connection_name: record.connection_name,
          kind: record.kind,
          status: record.status,
          arguments: record.arguments,
          started_at: record.started_at,
          ended_at: record.ended_at,
          error:
            record.error ||
            (toolResult?.isError
              ? { message: tr("工具返回执行错误，请查看输出。") }
              : undefined),
          progress: record.progress,
          metrics: record.metrics,
          server_metrics: record.server_metrics,
          result:
            record.result?.exit_code === undefined
              ? undefined
              : { exit_code: record.result.exit_code },
          output_tail: textTail(
            record.output?.text || record.output_tail || toolText,
          ),
        };
        setDetail(task);
        if (!active(task)) {
          if (terminal.current.size >= 100)
            terminal.current.delete(terminal.current.keys().next().value!);
          terminal.current.set(selectedId, task);
        }
      })
      .catch((e) => {
        if (!cancelled) setDetailError(String(e));
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, available, selectedId, live?.status, live?.output_tail, retry]);
  return {
    ...current,
    loading: loading || (available && searchPending),
    stale: available && !!page && (page.key !== requestKey || searchPending),
    error,
    detailError,
    detailLoading,
    detailRefreshing: detailLoading && detail?.task_id === selectedId,
    list,
    selectedTask:
      detail?.task_id === selectedId
        ? detail
        : current.tasks.find((t) => t.task_id === selectedId) || live,
    pageNumber: index + 1,
    previous:
      index > 0 && (!available || (!loading && page?.key === requestKey))
        ? () => setIndex((n) => n - 1)
        : undefined,
    next:
      (!available || (!loading && page?.key === requestKey)) &&
      current.next_cursor
        ? () => {
            setCursors((old) => [
              ...old.slice(0, index + 1),
              current.next_cursor,
            ]);
            setIndex(index + 1);
          }
        : undefined,
    reload: () => setRetry((n) => n + 1),
  };
}

export function useReplayHistory(
  enabled: boolean,
  available: boolean,
  snapshot: Snapshot | null,
) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [error, setError] = useState("");
  const signature = `${snapshot?.total_tasks}:${snapshot?.tasks
    .filter((t) => t.kind === "mcp.call")
    .map((t) => `${t.task_id}:${t.status}`)
    .join(",")}`;
  useEffect(() => {
    if (!enabled || !available) return;
    let cancelled = false;
    readTaskPage({ kind: "mcp.call", limit: 8 })
      .then((page) => {
        if (!cancelled) {
          setTasks(page.tasks);
          setError("");
        }
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, available, signature]);
  return { tasks: tasks || snapshot?.tasks || [], error };
}
