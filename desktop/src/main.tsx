import {
  Approvals,
  Pairing,
  BackendPanel,
  Replay,
  usePermissionChecks,
} from "./Features";
import { Overview, StepMark } from "./Overview";
import { Performance, PerformanceDetail } from "./Performance";
import { Access } from "./Access";
import { ThisMac } from "./ThisMac";
import { CommandPreview } from "./CommandPreview";
import { health as readHealth } from "./health";
import { clock, dayLabel, lastLine, serverViews, taskDuration } from "./format";
import { SettingsPage } from "./SettingsPage";
import { useTaskHistory, useReplayHistory } from "./taskHistory";
import "./task-history.css";
import appIcon from "./assets/macrun-icon.png";
import React, { useEffect, useState, useRef, useLayoutEffect } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Folder,
  History,
  Laptop,
  MousePointer2,
  Settings as SettingsIcon,
  Shield,
  Terminal,
  Square,
  RefreshCw,
  Search,
  Copy,
  Monitor,
  Info,
} from "lucide-react";
import {
  statuses,
  active,
  title,
  todaySummary,
  tierLabels,
  agentExit,
  attentionCount,
  explainTask,
  groupSessions,
  needsAttention,
  problemLabel,
  projectOf,
  taskHeadline,
} from "./model.mjs";
import type { Task, Snapshot, AppState } from "./types";
import "./style.css";
import "./v3.css";
import "./v4.css";
import "./dark.css";
import "./interaction.css";
const isTauri = !!(window as any).__TAURI_INTERNALS__;
const border = new URLSearchParams(location.search).has("border");
const overlay = new URLSearchParams(location.search).has("overlay");
const tray = new URLSearchParams(location.search).has("tray");
const label = (s: string) => (statuses as Record<string, string>)[s] || s;
const kindLabel = (t: Task) =>
  ["exec", "exec.start"].includes(t.kind)
    ? "命令"
    : t.kind === "sync"
      ? "同步"
      : t.kind === "mcp.call"
        ? `桌面 · ${t.arguments.tool || "操作"}`
        : t.kind;
const duration = (t: Task) => {
  if (t.status === "unknown") return "—";
  const sec = Math.max(
    0,
    Math.floor(((t.ended_at || Date.now()) - t.started_at) / 1000),
  );
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;
};
const time = (t: Task) =>
  new Date(t.started_at).toLocaleTimeString("zh-CN", { hour12: false });
type ToolPage = {
  session: string;
  result: {
    tools: { name: string; [key: string]: unknown }[];
    nextCursor?: string;
  };
};
function isToolPage(value: unknown): value is ToolPage {
  if (!value || typeof value !== "object") return false;
  const page = value as Partial<ToolPage>;
  return (
    typeof page.session === "string" &&
    page.session.length > 0 &&
    Array.isArray(page.result?.tools) &&
    page.result.tools.every(
      (tool) => tool && typeof tool.name === "string" && tool.name.length > 0,
    ) &&
    (page.result.nextCursor === undefined ||
      typeof page.result.nextCursor === "string")
  );
}
function Status({ status }: { status: string }) {
  return (
    <span className={`tag ${status}`}>
      <i className="dot" />
      {label(status)}
    </span>
  );
}

const dialogControls =
  "button:not(:disabled),input:not(:disabled),textarea:not(:disabled),select:not(:disabled),a[href],summary,[tabindex='0']";
const topDialog = () =>
  Array.from(
    document.querySelectorAll<HTMLElement>(
      '[role="dialog"][aria-modal="true"]',
    ),
  ).at(-1);

function useAppDialog(
  open: boolean,
  ref: React.RefObject<HTMLElement | null>,
  returnFocus: React.RefObject<HTMLElement | null>,
  onClose: () => void,
) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const dialog = ref.current;
    if (!open || !dialog) return;
    if (topDialog() === dialog)
      (
        dialog.querySelector<HTMLElement>("button:not(:disabled)") ||
        dialog.querySelector<HTMLElement>(dialogControls)
      )?.focus();
    const handler = (event: KeyboardEvent) => {
      if (event.defaultPrevented || topDialog() !== dialog) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        close.current();
        return;
      }
      if (event.key !== "Tab") return;
      const nodes = Array.from(
        dialog.querySelectorAll<HTMLElement>(dialogControls),
      );
      const first = nodes[0],
        last = nodes[nodes.length - 1];
      if (!first) {
        event.preventDefault();
        dialog.focus();
      } else if (
        !dialog.contains(document.activeElement) ||
        document.activeElement === dialog ||
        (event.shiftKey && document.activeElement === first) ||
        (!event.shiftKey && document.activeElement === last)
      ) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      }
    };
    document.addEventListener("keydown", handler);
    return () => {
      document.removeEventListener("keydown", handler);
      const top = topDialog(),
        previous = returnFocus.current;
      if (
        previous?.isConnected &&
        !previous.matches(":disabled") &&
        (!top || top.contains(previous))
      )
        previous.focus();
      else top?.querySelector<HTMLElement>(dialogControls)?.focus();
    };
  }, [open, ref, returnFocus]);
}

// Keep a visited page mounted, but skip its React work while another page is shown.
const RetainedPage = React.memo(
  function RetainedPage({
    active,
    children,
  }: {
    active: boolean;
    children: React.ReactNode;
  }) {
    return (
      <section className="page-view" hidden={!active}>
        {children}
      </section>
    );
  },
  (previous, next) => !previous.active && !next.active,
);

function App() {
  const firstLoad = useRef(true);
  const pendingActions = useRef(new Set<string>());
  const [pending, setPending] = useState<string[]>([]);
  const isPending = (action: string, id = "") =>
    pending.includes(`control:${action}:${id}`);
  const [visited, setVisited] = useState(["live"]);
  const content = useRef<HTMLElement>(null);
  const positions = useRef(new Map<string, number>());
  const pageRef = useRef("live");
  const setPage = (next: string) => {
    if (
      ![
        "live",
        "tasks",
        "performance",
        "access",
        "desktop",
        "settings",
      ].includes(next)
    )
      return;
    if (content.current)
      positions.current.set(pageRef.current, content.current.scrollTop);
    pageRef.current = next;
    setVisited((old) => (old.includes(next) ? old : [...old, next]));
    updatePage(next);
  };
  const quitReturnFocus = useRef<HTMLElement | null>(null);
  const toolsReturnFocus = useRef<HTMLElement | null>(null);
  const quitDialog = useRef<HTMLElement | null>(null);
  const toolsDialog = useRef<HTMLElement | null>(null);
  const pairDialog = useRef<HTMLElement | null>(null);
  const pairReturnFocus = useRef<HTMLElement | null>(null);
  const pairPending = useRef(false);
  const toolsRequest = useRef(0);
  const trayContent = useRef<HTMLDivElement>(null);
  const [page, updatePage] = useState("live"),
    [pairing, setPairing] = useState(false),
    [addingServer, setAddingServer] = useState(false),
    [pairRequest, setPairRequest] = useState<boolean | null>(null),
    [disconnectingForPair, setDisconnectingForPair] = useState(false),
    [connectionFilter, setConnectionFilter] = useState(""),
    [manualOpen, setManualOpen] = useState(false),
    [app, setApp] = useState<AppState | null>(null),
    [snapshot, setSnapshot] = useState<Snapshot | null>(null),
    [available, setAvailable] = useState(false),
    [error, setError] = useState(""),
    [refreshWarning, setRefreshWarning] = useState(""),
    [filter, setFilter] = useState("all"),
    [kind, setKind] = useState(""),
    [query, setQuery] = useState(""),
    [selected, setSelected] = useState(""),
    [quit, setQuit] = useState(false),
    [tools, setTools] = useState<ToolPage | null>(null),
    [toolsServer, setToolsServer] = useState(""),
    [notice, setNotice] = useState(""),
    [openSessions, setOpenSessions] = useState<Record<string, boolean>>({});
  const busy = pending.some((key) => !key.startsWith("control:"));
  useLayoutEffect(() => {
    if (content.current)
      content.current.scrollTop = positions.current.get(page) || 0;
  }, [page]);
  const tasks = snapshot?.tasks || [],
    running = tasks.filter(active),
    connected = available && snapshot?.connection.state === "connected",
    paused = snapshot?.policy.paused || false;
  const connectionSummary =
    (snapshot?.connections?.length || 0) > 1
      ? `${available ? snapshot!.connections!.filter((c) => c.connection.state === "connected").length : 0}/${snapshot!.connections!.length} 台服务器在线`
      : "";
  const refresh = async () => {
    if (!isTauri) return;
    const a = await invoke<AppState>("app_state");
    setRefreshWarning("");
    setApp(a);
    if (firstLoad.current) {
      firstLoad.current = false;
      if (!a.settings.server && !tray && !overlay && !border) {
        if (a.legacy_detected || a.legacy_running) setPage("settings");
        else setPairing(true);
      }
    }
    if (a.snapshot) setSnapshot(a.snapshot);
  };
  useEffect(() => {
    if (isTauri && !tray && !overlay && !border)
      invoke("set_main_mode", { pairing }).catch((e) => setError(String(e)));
  }, [pairing]);
  useEffect(() => {
    document.documentElement.classList.toggle(
      "transparent-surface",
      overlay || border || tray,
    );
  }, []);
  const act = async (
    command: string,
    args: Record<string, unknown> = {},
    success?: string,
  ) => {
    setError("");
    setRefreshWarning("");
    const payload = args.args as Record<string, unknown> | undefined;
    const key =
      command === "control"
        ? `control:${args.action}:${payload?.task_id || payload?.server || ""}`
        : command;
    if (pendingActions.current.has(key)) return;
    pendingActions.current.add(key);
    setPending([...pendingActions.current]);
    try {
      const r = await invoke(command, args);
      if (success) setNotice(success);
      try {
        await refresh();
      } catch (e) {
        setRefreshWarning(
          `操作已完成，但界面状态刷新失败，请勿重复操作。${String(e)}`,
        );
      }
      return r ?? true;
    } catch (e) {
      setError(String(e));
    } finally {
      pendingActions.current.delete(key);
      setPending([...pendingActions.current]);
    }
  };
  const control = (action: string, args: Record<string, unknown> = {}) =>
    act(
      "control",
      { action, args },
      {
        cancel: "已请求取消任务，等待执行器返回结果",
        stop_all: "已请求停止所有任务，暂停接收并关闭桌面控制",
        pause: args.paused ? "已暂停接收新任务" : "已恢复接收新任务",
        desktop: args.enabled
          ? "已允许桌面控制，实际能力请通过后端验证"
          : "已关闭桌面控制",
      }[action],
    );
  useEffect(() => {
    if (!isTauri || border) return;
    let disposed = false;
    const unsub: (() => void)[] = [];
    const on = <T,>(event: string, fn: (v: T) => void) =>
      listen<T>(event, (e) => {
        if (!disposed) fn(e.payload);
      }).then((f) => (disposed ? f() : unsub.push(f)));
    refresh().catch((e) => setError(String(e)));
    on<Snapshot>("worker-state", (s) => {
      setSnapshot(s);
      setAvailable(true);
      setApp((a) =>
        a && (!a.worker_running || a.worker_starting)
          ? { ...a, worker_running: true, worker_starting: false }
          : a,
      );
    });
    on<boolean>("worker-starting", (starting) => {
      setApp((a) => (a ? { ...a, worker_starting: starting } : a));
    });
    on("worker-unavailable", () => {
      setAvailable(false);
      refresh().catch(() => {});
    });
    on<string>("navigate", (route) => {
      setPairing(false);
      const [target, taskId] = route.split(":");
      setPage(target);
      if (taskId) {
        setSelected(taskId);
        setFilter("all");
        setKind("");
        setQuery("");
      }
    });
    on("exit-requested", () => {
      if (!quitDialog.current)
        quitReturnFocus.current = document.activeElement as HTMLElement;
      setQuit(true);
    });
    on<string>("control-error", setError);
    const foregroundRefresh = () => {
      if (document.visibilityState !== "hidden") refresh().catch(() => {});
    };
    window.addEventListener("focus", foregroundRefresh);
    document.addEventListener("visibilitychange", foregroundRefresh);
    const timer = setInterval(foregroundRefresh, 30000);
    return () => {
      disposed = true;
      unsub.forEach((f) => f());
      clearInterval(timer);
      window.removeEventListener("focus", foregroundRefresh);
      document.removeEventListener("visibilitychange", foregroundRefresh);
    };
  }, []);
  useEffect(() => {
    if (!tray || !isTauri || !trayContent.current) return;
    const observer = new ResizeObserver(() => {
      const height = trayContent.current?.scrollHeight || 0;
      if (height)
        invoke("resize_panel", { height: Math.ceil(height + 14) }).catch(
          () => {},
        );
    });
    observer.observe(trayContent.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(""), 4000);
    return () => clearTimeout(t);
  }, [notice]);
  useEffect(() => {
    if (
      !snapshot ||
      tray ||
      overlay ||
      border ||
      !app?.preferences.notifications
    )
      return;
    const key = "macrun-seen-notifications";
    let seen: string[] = [];
    try {
      seen = JSON.parse(sessionStorage.getItem(key) || "[]");
    } catch {}
    // Approvals notify too: the panel is often closed while a request waits.
    const current = tasks.filter((t) =>
      ["failed", "unknown", "timed_out", "awaiting_approval"].includes(
        t.status,
      ),
    );
    if (sessionStorage.getItem(key))
      current
        .filter((t) => !seen.includes(t.task_id))
        .forEach((t) => {
          invoke("notify_task", { status: t.status }).catch(() => {});
        });
    sessionStorage.setItem(key, JSON.stringify(current.map((t) => t.task_id)));
  }, [snapshot]);
  useAppDialog(quit, quitDialog, quitReturnFocus, () => setQuit(false));
  useAppDialog(pairRequest !== null, pairDialog, pairReturnFocus, () => {
    if (!pairPending.current) setPairRequest(null);
  });
  const beginPairing = (add = false) => {
    setError("");
    if (app?.worker_running) {
      pairReturnFocus.current = document.activeElement as HTMLElement;
      setPairRequest(add);
    } else {
      setAddingServer(add);
      setPairing(true);
    }
  };
  const disconnectAndPair = async () => {
    if (pairPending.current || pairRequest === null) return;
    pairPending.current = true;
    setDisconnectingForPair(true);
    try {
      const stopped = await act("stop_worker", { onlyIfIdle: true });
      if (!stopped) return;
      setAddingServer(pairRequest);
      setPairRequest(null);
      setPairing(true);
    } finally {
      pairPending.current = false;
      setDisconnectingForPair(false);
    }
  };
  const closeTools = () => {
    toolsRequest.current += 1;
    setTools(null);
  };
  useEffect(() => {
    toolsRequest.current += 1;
    setTools(null);
  }, [page, pairing]);
  useAppDialog(!!tools, toolsDialog, toolsReturnFocus, closeTools);
  const jump = (p: string) =>
    tray ? act("open_main", { route: p }) : setPage(p);
  const stop = () => control("stop_all");
  const pause = () => control("pause", { paused: !paused });
  const desktopToggle = () =>
    control("desktop", { enabled: !snapshot?.policy.desktop_enabled });
  const choose = (t: Task) => {
    setSelected(t.task_id);
    setFilter("all");
    setKind("");
    setQuery("");
    jump(tray ? `tasks:${t.task_id}` : "tasks");
  };
  const copyText = async (text: string, done: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setNotice(done);
    } catch {
      setError("复制失败，请手动选择文本");
    }
  };
  const copy = (id: string) => copyText(id, "已复制任务编号");
  const stateText = !available
    ? "执行器未连接"
    : !connected
      ? "与服务器断开"
      : paused
        ? "已暂停接收"
        : running.length
          ? "Agent 正在工作"
          : "已连接，空闲";
  const stateClass = !available
    ? "cancelled"
    : !connected
      ? "failed"
      : paused
        ? "cancelled"
        : running.length
          ? "running"
          : "succeeded";
  // Waiting requests appear once, in the approval area, not in activity lists.
  const working = running.filter((t) => t.status !== "awaiting_approval");
  const recent = tasks.filter((t) => !active(t)).slice(0, 6);
  const approvals = tasks.filter((t) => t.status === "awaiting_approval");
  const desktopBusy = working.some((t) => t.kind === "mcp.call");
  const history = useTaskHistory({
    enabled: page === "tasks" && !tray,
    available,
    snapshot,
    filter,
    kind,
    query,
    selected,
    connectionId: connectionFilter,
  });
  const replayHistory = useReplayHistory(
    page === "desktop" && !tray,
    available,
    snapshot,
  );
  const filtered = history.tasks;
  const sel = history.selectedTask;
  const summary =
    snapshot?.today_summary || (todaySummary(tasks) as Record<string, number>);
  const mainWindow = isTauri && !tray && !overlay && !border;
  // Shared by the overview's health line, the This Mac checklist and the
  // Macrun permission details, so they read macOS once rather than three times.
  const permissionState = usePermissionChecks(
    undefined,
    mainWindow && !!app && (page === "desktop" || page === "live"),
  );
  const status = readHealth(
    snapshot,
    app,
    available,
    permissionState.permissions,
  );
  const servers = serverViews(
    app?.settings,
    snapshot,
    available && !!app?.worker_running,
  );
  const serverLabel = (t: Task) =>
    servers.length > 1
      ? servers.find((s) => s.id === t.connection_id)?.label ||
        t.connection_name ||
        ""
      : "";
  const nav = [
    ["live", "概览", Activity],
    ["tasks", "活动", History],
    ["performance", "性能", Activity],
    ["access", "权限", Shield],
    ["desktop", "本机", Laptop],
  ] as const;
  const feedback = (
    <>
      {error && (
        <div className="alert error action-feedback" role="alert">
          <Info size={16} />
          <span>{error}</span>
          <button onClick={() => setError("")} aria-label="关闭错误提示">
            ×
          </button>
        </div>
      )}
      {refreshWarning ? (
        <div className="toast" role="alert">
          {refreshWarning}
        </div>
      ) : (
        notice && (
          <div className="toast" role="status">
            {notice}
          </div>
        )
      )}
      {app?.worker_starting && !overlay && !border && (
        <div className="alert startup-feedback" role="status">
          <RefreshCw size={16} className="loading-icon" />
          <span>
            正在启动执行器。如 macOS 弹出钥匙串授权，请在系统窗口中完成允许。
            等待期间仍可查看其他页面。
          </span>
        </div>
      )}
    </>
  );
  const exitDialog = quit && (
    <div className="scrim">
      <section
        className="dialog"
        ref={quitDialog}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="exit-title"
      >
        <h2 id="exit-title">退出 Macrun？</h2>
        <p>
          {running.length
            ? `仍有 ${running.length} 个任务进行中。退出会停止任务及本机执行程序。`
            : "退出后，本机将不再接收任务。"}
        </p>
        <p className="muted">尚未确认的桌面操作可能已经生效，不会自动重放。</p>
        <div className="actions">
          <button autoFocus onClick={() => setQuit(false)}>
            继续运行
          </button>
          <button
            className="danger"
            disabled={busy}
            onClick={() => act("exit_app", { force: true })}
          >
            停止并退出
          </button>
        </div>
        {feedback}
      </section>
    </div>
  );
  if (border)
    return (
      <div style={{ position: "fixed", inset: 0, background: "#e8590c" }} />
    );
  if (overlay)
    return (
      <div className="overlay-shell">
        <div className="overlay-bar">
          <span className="dot" />
          <span className="overlay-action">
            <b>Agent 正在操作这台 Mac</b>
            <span>
              {" "}
              ·{" "}
              {running.find((t) => t.kind === "mcp.call")?.arguments.tool ||
                "桌面操作"}
            </span>
          </span>
          <span className="mono overlay-key">⌃⌥⌘.</span>
          <button disabled={busy || !available} onClick={stop}>
            停止
          </button>
        </div>
        {feedback}
      </div>
    );
  if (tray) {
    const trayProjects = groupSessions(
      working,
      snapshot?.workspaces || [],
    ) as ReturnType<typeof groupSessions>;
    const codeProjects = trayProjects.filter((p: any) => !p.project.desktop);
    const headline = approvals.length
      ? `需要你确认 ${approvals.length} 个请求`
      : !available
        ? "执行器未运行"
        : !connected
          ? "未连接服务器"
          : paused
            ? "已暂停接收新任务"
            : codeProjects.length > 1
              ? `正在 ${codeProjects.length} 个项目上工作`
              : codeProjects.length === 1
                ? `正在处理 ${codeProjects[0].project.name}`
                : desktopBusy
                  ? "Agent 正在操作桌面"
                  : "就绪，等待 Agent";
    const problems = attentionCount(summary);
    const sub =
      !available || !connected
        ? snapshot?.connection.error || ""
        : [
            approvals.length && working.length
              ? `${trayProjects.length} 个项目进行中`
              : codeProjects.length && desktopBusy
                ? "另有桌面操作"
                : "",
            status.servers > 1
              ? `${status.online}/${status.servers} 台服务器在线`
              : snapshot?.connection.rtt_ms != null
                ? `已连接 · ${snapshot.connection.rtt_ms} ms`
                : "已连接",
          ]
            .filter(Boolean)
            .join(" · ");
    const dotClass = approvals.length
      ? "awaiting_approval"
      : desktopBusy
        ? "desktop-active"
        : stateClass;
    const recentProjects =
      !working.length && !approvals.length
        ? (
            groupSessions(
              tasks.filter((t) => !active(t)),
              snapshot?.workspaces || [],
            ) as ReturnType<typeof groupSessions>
          ).slice(0, 2)
        : [];
    return (
      <div className="tray">
        <div ref={trayContent} className="tray-content">
          <header className="tray-head">
            <span className={`dot ${dotClass}`} />
            <div className="grow">
              <h2>{headline}</h2>
              {sub && <p>{sub}</p>}
            </div>
            {(running.length > 0 || approvals.length > 0) && (
              <button
                className="stop-all"
                disabled={!available || busy}
                onClick={stop}
                title="取消所有任务、暂停接收并关闭桌面控制（⌃⌥⌘.）"
              >
                <Square size={11} />
                停止
              </button>
            )}
            {!available && (
              <button className="primary" onClick={() => jump("desktop")}>
                检查连接
              </button>
            )}
            {paused && available && !running.length && (
              <button disabled={busy} onClick={pause}>
                恢复
              </button>
            )}
          </header>
          <Approvals
            tasks={tasks}
            act={act}
            disabled={busy || !available}
            workspaces={snapshot?.workspaces}
            serverLabel={serverLabel}
            compact
          />
          {working.length > 0 && (
            <section className="tray-section" aria-label="正在进行">
              {trayProjects.slice(0, 3).map((p: any) => {
                const t: Task = p.tasks.find(
                  (x: Task) => active(x) && x.status !== "awaiting_approval",
                );
                const desktop = p.project.desktop;
                return (
                  <button
                    className="tray-task"
                    key={p.id}
                    onClick={() => choose(t)}
                  >
                    <span className={`tray-kind ${desktop ? "desktop" : ""}`}>
                      {desktop ? (
                        <MousePointer2 size={13} />
                      ) : t.kind === "sync" ? (
                        <RefreshCw size={13} />
                      ) : (
                        <Terminal size={13} />
                      )}
                    </span>
                    <span className="grow">
                      <span className="row">
                        <b className="ellipsis">
                          {desktop ? "桌面操作" : p.project.name}
                        </b>
                        {p.project.tag && <small>{p.project.tag}</small>}
                        <span className="grow" />
                        <small className="mono">{taskDuration(t)}</small>
                      </span>
                      <span className="mono ellipsis tray-task-title">
                        {taskHeadline(t)}
                      </span>
                      {!desktop && lastLine(t.output_tail) && (
                        <small className="mono ellipsis tray-tail">
                          {t.progress
                            ? `已收到 ${t.progress.received} / ${t.progress.total} 个文件`
                            : lastLine(t.output_tail)}
                        </small>
                      )}
                    </span>
                  </button>
                );
              })}
              {trayProjects.length > 3 && (
                <button className="link tray-more" onClick={() => jump("live")}>
                  还有 {trayProjects.length - 3} 个项目
                </button>
              )}
            </section>
          )}
          {recentProjects.length > 0 && (
            <section className="tray-section" aria-label="最近">
              <small className="tray-label">最近</small>
              {recentProjects.map((p: any) => (
                <button
                  className="tray-task recent"
                  key={p.id}
                  onClick={() => choose(p.tasks[0])}
                >
                  <span className="grow ellipsis">
                    <b>{p.project.name}</b>{" "}
                    <small>{p.tasks.length} 个任务</small>
                  </span>
                  <small>{clock(p.end)}</small>
                </button>
              ))}
            </section>
          )}
          {available && connected && !approvals.length && (
            <button className="tray-today" onClick={() => jump("tasks")}>
              今天 {summary.total || 0} 个任务 ·{" "}
              {problems ? `${problems} 个问题` : "没有问题"}
              <span className="push">›</span>
            </button>
          )}
          <section className="tray-section tray-switches">
            <label className="tray-switch">
              <span className="grow">接收新任务</span>
              <input
                className="switch"
                type="checkbox"
                aria-label="接收新任务"
                checked={!paused}
                disabled={!available || isPending("pause")}
                aria-busy={isPending("pause")}
                onChange={pause}
              />
            </label>
            <label className="tray-switch">
              <span className="grow">允许 Agent 操作桌面</span>
              <input
                className="switch"
                type="checkbox"
                aria-label="允许 Agent 操作桌面"
                checked={snapshot?.policy.desktop_enabled || false}
                disabled={!available || isPending("desktop")}
                aria-busy={isPending("desktop")}
                onChange={desktopToggle}
              />
            </label>
          </section>
          <footer className="tray-menu">
            <button onClick={() => jump("live")}>
              打开 Macrun<small>⌘O</small>
            </button>
            <button onClick={() => jump("settings")}>
              设置…<small>⌘,</small>
            </button>
            <button onClick={() => act("request_quit")}>
              退出 Macrun<small>⌘Q</small>
            </button>
          </footer>
          {feedback}
          {exitDialog}
        </div>
      </div>
    );
  }
  if (pairing)
    return (
      <div className="pairing-window">
        <div className="pairing-feedback">{feedback}</div>
        {exitDialog}
        <Pairing
          act={(command, args = {}, success) =>
            act(
              command,
              command === "pair" ? { ...args, add: addingServer } : args,
              success,
            )
          }
          running={!!app?.worker_running}
          adding={addingServer}
          onClose={() => {
            setPairing(false);
            setPage("settings");
          }}
          onManual={
            addingServer
              ? undefined
              : () => {
                  setPairing(false);
                  setManualOpen(true);
                  setPage("settings");
                }
          }
          onComplete={(route) => {
            setPairing(false);
            setPage(route === "desktop" ? "desktop" : "live");
          }}
        />
      </div>
    );
  return (
    <div className="layout">
      <aside className="sidebar">
        <div className="drag" data-tauri-drag-region />
        <div className="brand" data-tauri-drag-region>
          <div className="logo" data-tauri-drag-region>
            <img src={appIcon} alt="" className="brand-icon" />
          </div>
          <div>
            <b data-tauri-drag-region>Macrun</b>
            <small data-tauri-drag-region>
              {snapshot ? `v${snapshot.version}` : "执行器未启动"}
            </small>
          </div>
        </div>
        <nav aria-label="主导航">
          {nav.map(([id, text, Icon]) => {
            // Badges only count things that need the person.
            const badge =
              id === "live" && approvals.length
                ? { text: String(approvals.length), warn: true }
                : id === "live" && working.length
                  ? { text: String(working.length), warn: false }
                  : id === "desktop" &&
                      status.desktopEnabled &&
                      status.desktopMissing
                    ? { text: `${status.desktopMissing} 项`, warn: false }
                    : null;
            return (
              <button
                aria-current={page === id ? "page" : undefined}
                className={page === id ? "on" : ""}
                key={id}
                onClick={() => setPage(id)}
              >
                <Icon size={17} />
                {text}
                {badge && (
                  <span
                    className={badge.warn ? "warn" : ""}
                    aria-label={
                      id === "live" && badge.warn
                        ? `${badge.text} 个请求等你确认`
                        : undefined
                    }
                  >
                    {badge.text}
                  </span>
                )}
              </button>
            );
          })}
        </nav>
        <div className="v4-side-foot" aria-label="服务器状态">
          {servers.map((s) => (
            <button
              key={s.id}
              className="v4-side-server"
              title={`${s.label} · 在“本机”中查看`}
              onClick={() => setPage("desktop")}
            >
              <span
                className={`dot ${s.connection?.state === "connected" ? "succeeded" : app?.worker_running ? "failed" : "cancelled"}`}
              />
              <span className="grow ellipsis">{s.label}</span>
              <span className="mono">
                {s.connection?.state === "connected" &&
                s.connection.rtt_ms != null
                  ? `${s.connection.rtt_ms} ms`
                  : s.connection?.state === "connected"
                    ? ""
                    : app?.worker_starting
                      ? "启动中"
                      : app?.worker_running
                        ? "重连中"
                        : "未连接"}
              </span>
            </button>
          ))}
          {!servers.length && (
            <button
              className="v4-side-server"
              onClick={() => setPage("desktop")}
            >
              <span className="dot cancelled" />
              <span className="grow">尚未配对服务器</span>
            </button>
          )}
          <button
            className={`v4-side-server settings ${page === "settings" ? "on" : ""}`}
            aria-current={page === "settings" ? "page" : undefined}
            onClick={() => setPage("settings")}
          >
            <SettingsIcon size={15} />
            <span className="grow">设置</span>
          </button>
        </div>
      </aside>
      <main ref={content} className={`content page-${page}`}>
        <div className="drag title-drag" data-tauri-drag-region />
        {!isTauri && (
          <div className="alert">
            请在 Tauri
            桌面应用中打开。浏览器仅显示空状态布局，不连接本机执行器。
          </div>
        )}
        {feedback}
        {snapshot && !available && !app?.worker_starting && (
          <div className="alert" role="status">
            <Info size={16} />
            <span>
              执行器暂时不可用。以下保留上次收到的记录，任务状态可能已变化。
            </span>
          </div>
        )}
        {visited.includes("live") && (
          <RetainedPage active={page === "live"}>
            <header className="page-header v4-toolbar">
              <h1>概览</h1>
              <span className="grow" />
              <label className="v4-receive">
                接收新任务
                <input
                  className="switch"
                  type="checkbox"
                  aria-label="接收新任务"
                  checked={!paused}
                  disabled={!available || isPending("pause")}
                  aria-busy={isPending("pause")}
                  onChange={pause}
                />
              </label>
              <button
                className="v4-stop"
                disabled={!available || isPending("stop_all")}
                aria-busy={isPending("stop_all")}
                title="取消所有任务、暂停接收并关闭桌面控制"
                onClick={stop}
              >
                <Square size={13} />
                停止全部 <small>⌃⌥⌘.</small>
              </button>
            </header>
            <Overview
              snapshot={snapshot}
              app={app}
              available={available}
              act={act}
              isPending={isPending}
              control={control}
              onTask={choose}
              onPage={setPage}
              health={status}
              servers={servers}
            />
          </RetainedPage>
        )}
        {visited.includes("performance") && (
          <RetainedPage active={page === "performance"}>
            <Performance active={page === "performance"} app={app} />
          </RetainedPage>
        )}
        {visited.includes("tasks") && (
          <RetainedPage active={page === "tasks"}>
            <header className="page-header v4-toolbar">
              <h1>活动</h1>
              <div className="v4-seg" role="group" aria-label="快速筛选">
                {(
                  [
                    ["all", "全部", ""],
                    ["attention", "需要关注", ""],
                    ["all", "桌面", "mcp.call"],
                  ] as const
                ).map(([f, text, k]) => {
                  const on = filter === f && kind === k;
                  const count =
                    text === "全部"
                      ? Object.entries(history.counts)
                          .filter(([key]) => key !== "exited")
                          .reduce((sum, [, n]) => sum + n, 0)
                      : text === "需要关注"
                        ? attentionCount(history.counts)
                        : null;
                  return (
                    <button
                      key={text}
                      aria-pressed={on}
                      className={`${on ? "on" : ""} ${f === "attention" ? "attention" : ""}`}
                      onClick={() => {
                        setFilter(f);
                        setKind(k);
                        setSelected("");
                      }}
                    >
                      {text}
                      {count !== null && <small>{count}</small>}
                    </button>
                  );
                })}
              </div>
              {servers.length > 1 && (
                <div className="v4-seg" role="group" aria-label="按服务器筛选">
                  {[{ id: "", label: "所有服务器" }, ...servers].map((c) => (
                    <button
                      key={c.id || "all-servers"}
                      aria-pressed={connectionFilter === c.id}
                      className={connectionFilter === c.id ? "on" : ""}
                      onClick={() => {
                        setConnectionFilter(c.id);
                        setSelected("");
                      }}
                    >
                      {c.label}
                    </button>
                  ))}
                </div>
              )}
              <label className="v4-more">
                <span className="sr-only">更多筛选</span>
                <select
                  aria-label="更多筛选"
                  value={
                    !["all", "attention"].includes(filter)
                      ? filter
                      : kind && kind !== "mcp.call"
                        ? `kind:${kind}`
                        : ""
                  }
                  onChange={(e) => {
                    const v = e.target.value;
                    if (v.startsWith("kind:")) {
                      setFilter("all");
                      setKind(v.slice(5));
                    } else {
                      setFilter(v || "all");
                      setKind("");
                    }
                    setSelected("");
                  }}
                >
                  <option value="">更多筛选</option>
                  <optgroup label="状态">
                    {Object.entries(statuses).map(([k, v]) => (
                      <option key={k} value={k}>
                        {v} {history.counts[k] || 0}
                      </option>
                    ))}
                  </optgroup>
                  <optgroup label="类型">
                    <option value="kind:exec.start">命令</option>
                    <option value="kind:sync">同步</option>
                  </optgroup>
                </select>
              </label>
              <span className="grow" />
              <label className="search">
                <Search size={15} />
                <input
                  aria-label="搜索任务"
                  placeholder="项目、命令、目录或任务编号"
                  value={query}
                  onChange={(e) => {
                    setQuery(e.target.value);
                    setSelected("");
                  }}
                />
              </label>
            </header>
            <div className="v4-activity">
              <div className="task-history-list">
                {!available && (
                  <p className="task-history-status">
                    执行器未连接，仅显示已缓存的近期任务。
                  </p>
                )}
                {history.error && (
                  <div className="task-history-error" role="alert">
                    历史记录读取失败：{history.error}{" "}
                    <button onClick={history.reload}>重新加载历史</button>
                  </div>
                )}
                <section
                  className="v4-task-list"
                  aria-label="任务列表"
                  ref={history.list}
                  aria-busy={history.loading}
                  data-updating={history.stale || undefined}
                  onKeyDown={(event) => {
                    if (
                      !["ArrowUp", "ArrowDown", "Home", "End"].includes(
                        event.key,
                      ) ||
                      event.altKey ||
                      event.metaKey ||
                      event.ctrlKey
                    )
                      return;
                    const rows = Array.from(
                      event.currentTarget.querySelectorAll<HTMLButtonElement>(
                        "button[data-task-row]",
                      ),
                    );
                    const at = rows.indexOf(
                      document.activeElement as HTMLButtonElement,
                    );
                    if (at < 0) return;
                    event.preventDefault();
                    const next =
                      event.key === "Home"
                        ? 0
                        : event.key === "End"
                          ? rows.length - 1
                          : Math.max(
                              0,
                              Math.min(
                                rows.length - 1,
                                at + (event.key === "ArrowDown" ? 1 : -1),
                              ),
                            );
                    rows[next]?.focus();
                    rows[next]?.click();
                  }}
                >
                  {(() => {
                    const sessions = groupSessions(
                      filtered,
                      snapshot?.workspaces || [],
                    );
                    let day = "";
                    return sessions.map((session: any, index: number) => {
                      const label = dayLabel(session.end);
                      const heading = label !== day;
                      day = label;
                      const multi = session.tasks.length > 1;
                      const contains = session.tasks.some(
                        (t: Task) => t.task_id === sel?.task_id,
                      );
                      const open =
                        !multi ||
                        (openSessions[session.id] ?? (index < 2 || contains));
                      const row = (t: Task) => (
                        <button
                          data-task-row
                          className={`v4-row ${sel?.task_id === t.task_id ? "selected" : ""}`}
                          key={t.task_id}
                          aria-label={`${title(t)} · ${label}`}
                          onClick={() => {
                            setSelected(t.task_id);
                            if (window.innerWidth <= 1000)
                              document
                                .querySelector<HTMLElement>(
                                  '[aria-label="任务详情"]',
                                )
                                ?.scrollIntoView?.({
                                  block: "nearest",
                                  behavior: "smooth",
                                });
                          }}
                        >
                          <span className="v4-time">{clock(t.started_at)}</span>
                          <span className="v4-mark">
                            <StepMark task={t} />
                          </span>
                          <span className="grow v4-row-text">
                            <span
                              className={`ellipsis ${t.kind === "sync" ? "" : "mono"}`}
                            >
                              {taskHeadline(t)}
                            </span>
                            {!multi && (
                              <small className="ellipsis">
                                {session.project.name}
                                {session.project.tag
                                  ? ` · ${session.project.tag}`
                                  : ""}
                                {serverLabel(t) ? ` · ${serverLabel(t)}` : ""}
                              </small>
                            )}
                          </span>
                          <span className="v4-time right">
                            {taskDuration(t)}
                          </span>
                        </button>
                      );
                      return (
                        <React.Fragment key={session.id}>
                          {heading && <div className="v4-day">{label}</div>}
                          {multi ? (
                            <div className={`v4-session ${open ? "open" : ""}`}>
                              <div
                                className="v4-session-head"
                                role="button"
                                tabIndex={0}
                                aria-expanded={open}
                                onClick={() =>
                                  setOpenSessions((o) => ({
                                    ...o,
                                    [session.id]: !open,
                                  }))
                                }
                                onKeyDown={(e) => {
                                  if (e.key !== "Enter" && e.key !== " ")
                                    return;
                                  e.preventDefault();
                                  setOpenSessions((o) => ({
                                    ...o,
                                    [session.id]: !open,
                                  }));
                                }}
                              >
                                {open ? (
                                  <ChevronDown size={14} />
                                ) : (
                                  <ChevronRight size={14} />
                                )}
                                {session.project.desktop ? (
                                  <MousePointer2 size={14} className="v4-act" />
                                ) : (
                                  <Folder size={14} className="v4-faint" />
                                )}
                                <span className="grow">
                                  <b>{session.project.name}</b>
                                  {session.project.tag && (
                                    <span className="v4-faint">
                                      {" "}
                                      {session.project.tag}
                                    </span>
                                  )}
                                  {serverLabel(session.tasks[0]) && (
                                    <span className="v4-faint">
                                      {" "}
                                      · {serverLabel(session.tasks[0])}
                                    </span>
                                  )}
                                  <small>
                                    {clock(session.start)} –{" "}
                                    {session.active
                                      ? "现在"
                                      : clock(session.end)}{" "}
                                    · {session.tasks.length} 个任务
                                    {session.exited
                                      ? ` · ${session.exited} 个非零退出`
                                      : ""}
                                  </small>
                                </span>
                                {session.active ? (
                                  <span
                                    className={`v4-chip ${session.project.desktop ? "act" : "run"}`}
                                  >
                                    {session.project.desktop
                                      ? "操作中"
                                      : "进行中"}
                                  </span>
                                ) : session.problems ? (
                                  <span className="v4-chip warn">
                                    {problemLabel(
                                      session.tasks.find(needsAttention),
                                    )}
                                  </span>
                                ) : (
                                  <span className="v4-faint small">完成</span>
                                )}
                              </div>
                              {open && session.tasks.map(row)}
                            </div>
                          ) : (
                            row(session.tasks[0])
                          )}
                        </React.Fragment>
                      );
                    });
                  })()}
                  {!filtered.length && (
                    <div className="empty">
                      {history.loading ? "正在读取任务…" : "没有匹配的任务"}
                    </div>
                  )}
                </section>
                <nav className="task-pagination" aria-label="任务分页">
                  <span role="status">
                    {history.loading
                      ? "正在读取…"
                      : `共 ${history.filtered_total} 条 · 第 ${history.pageNumber} 页`}
                  </span>
                  <button
                    disabled={!history.previous || history.loading}
                    onClick={() => {
                      setSelected("");
                      history.previous?.();
                    }}
                  >
                    较新
                  </button>
                  <button
                    disabled={!history.next || history.loading}
                    onClick={() => {
                      setSelected("");
                      history.next?.();
                    }}
                  >
                    更早
                  </button>
                </nav>
              </div>
              <section className="v4-detail" aria-label="任务详情">
                {history.detailError && (
                  <div className="task-history-error" role="alert">
                    任务详情读取失败：{history.detailError}{" "}
                    <button onClick={history.reload}>重试读取详情</button>
                  </div>
                )}
                {sel ? (
                  (() => {
                    const explanation = explainTask(sel);
                    const project = projectOf(sel, snapshot?.workspaces || []);
                    const command = sel.arguments.command as string | undefined;
                    const directory =
                      sel.arguments.cwd || sel.arguments.remote_root;
                    return (
                      <>
                        <div className="row v4-detail-meta">
                          <span className="v4-chip">{kindLabel(sel)}</span>
                          <span
                            className={`v4-result ${needsAttention(sel) ? "warn" : agentExit(sel) ? "" : sel.status}`}
                          >
                            {agentExit(sel)
                              ? `退出码 ${sel.result?.exit_code}`
                              : sel.status === "succeeded"
                                ? `✓ 成功${sel.result?.exit_code != null ? ` · 退出码 ${sel.result.exit_code}` : ""}`
                                : problemLabel(sel)}
                          </span>
                          <span className="grow" />
                          <small>
                            {dayLabel(sel.started_at)} {time(sel)} · 用时{" "}
                            {taskDuration(sel)}
                          </small>
                        </div>
                        {command ? (
                          <CommandPreview key={sel.task_id} command={command} />
                        ) : (
                          <h3 className="mono command">
                            {sel.kind === "mcp.call"
                              ? `${sel.arguments.server} · ${sel.arguments.tool}`
                              : title(sel)}
                          </h3>
                        )}
                        <dl>
                          <dt>项目</dt>
                          <dd>
                            {project.name}
                            {project.tag && (
                              <span className="v4-faint"> · {project.tag}</span>
                            )}
                          </dd>
                          {directory && (
                            <>
                              <dt>目录</dt>
                              <dd className="mono">{directory}</dd>
                            </>
                          )}
                          {sel.arguments.path && (
                            <>
                              <dt>文件</dt>
                              <dd className="mono">{sel.arguments.path}</dd>
                            </>
                          )}
                          {serverLabel(sel) && (
                            <>
                              <dt>来自</dt>
                              <dd className="task-source">
                                {serverLabel(sel)}
                              </dd>
                            </>
                          )}
                          {sel.desktop_tier && (
                            <>
                              <dt>桌面分级</dt>
                              <dd>{tierLabels[sel.desktop_tier]}</dd>
                            </>
                          )}
                          {sel.approved_by_rule && (
                            <>
                              <dt>确认方式</dt>
                              <dd>按临时允许规则自动放行</dd>
                            </>
                          )}
                          {sel.arguments.env && (
                            <>
                              <dt>环境变量</dt>
                              <dd className="mono">
                                {Object.keys(sel.arguments.env)
                                  .map((k) => `${k}=••••`)
                                  .join(" · ")}
                              </dd>
                            </>
                          )}
                          <dt>任务编号</dt>
                          <dd className="mono">
                            {sel.task_id}{" "}
                            <button
                              className="v4-link"
                              onClick={() => copy(sel.task_id)}
                            >
                              <Copy size={12} />
                              复制任务编号
                            </button>
                          </dd>
                        </dl>
                        <PerformanceDetail
                          metrics={sel.metrics}
                          server={sel.server_metrics}
                        />
                        {explanation && !explanation.agentExit && (
                          <div className="v4-explain" role="note">
                            <div>
                              <b>发生了什么</b>
                              {explanation.what}
                            </div>
                            <div>
                              <b>Agent 收到了什么</b>
                              {explanation.agent}
                            </div>
                            <div>
                              <b>你可以做什么</b>
                              {explanation.you}
                            </div>
                          </div>
                        )}
                        {explanation?.agentExit && (
                          <p className="v4-note">
                            {explanation.what}
                            {explanation.agent}这是命令自己的结果，不计入 Macrun
                            问题。
                          </p>
                        )}
                        <div className="v4-sh">
                          <h2>输出</h2>
                          <span className="grow" />
                          <small>最后 8 KB</small>
                        </div>
                        <pre
                          className="term"
                          tabIndex={0}
                          onKeyDown={selectLogText}
                        >
                          {history.detailLoading && !history.detailRefreshing
                            ? "正在读取输出…"
                            : sel.output_tail ||
                              sel.error?.message ||
                              "暂无文本输出"}
                        </pre>
                        <div className="actions">
                          {command && (
                            <button
                              onClick={() => copyText(command, "已复制命令")}
                            >
                              <Copy size={14} />
                              复制命令
                            </button>
                          )}
                          <button
                            onClick={() =>
                              act("open_log", { taskId: sel.task_id })
                            }
                          >
                            打开完整日志
                          </button>
                          {directory && (
                            <button
                              onClick={() =>
                                act("open_workspace", {
                                  root: directory,
                                  terminal: true,
                                  taskId: sel.task_id,
                                })
                              }
                            >
                              在终端打开目录
                            </button>
                          )}
                          {sel.status === "unknown" && (
                            <button onClick={() => setPage("desktop")}>
                              前往本机核对
                            </button>
                          )}
                          {active(sel) && (
                            <button
                              disabled={
                                !available || isPending("cancel", sel.task_id)
                              }
                              aria-busy={isPending("cancel", sel.task_id)}
                              className="danger"
                              onClick={() =>
                                control("cancel", { task_id: sel.task_id })
                              }
                            >
                              取消任务
                            </button>
                          )}
                        </div>
                      </>
                    );
                  })()
                ) : (
                  <div className="empty">选择任务查看结果和输出。</div>
                )}
              </section>
            </div>
          </RetainedPage>
        )}
        {visited.includes("access") && (
          <RetainedPage active={page === "access"}>
            <header className="page-header v4-toolbar">
              <h1>权限</h1>
              <span className="v4-muted">
                远程 Agent 在这台 Mac 上能做什么。对所有服务器生效。
              </span>
            </header>
            <Access
              snapshot={snapshot}
              act={act}
              available={available}
              pending={isPending("safety")}
            />
          </RetainedPage>
        )}
        {visited.includes("desktop") && (
          <RetainedPage active={page === "desktop"}>
            <header className="page-header v4-toolbar">
              <h1>本机</h1>
              <span className="v4-muted">这台 Mac 是否准备好替 Agent 干活</span>
            </header>
            <ThisMac
              snapshot={snapshot}
              app={app}
              available={available}
              act={act}
              busy={busy}
              health={status}
              servers={servers}
              onPair={beginPairing}
              onSettings={() => setPage("settings")}
              desktopToggle={desktopToggle}
              desktopPending={isPending("desktop")}
              technical={
                <>
                  <h2 className="v4-sub">后端</h2>
                  {snapshot?.backends.map((b) => (
                    <article className="card backend" key={b.name}>
                      <div className="row">
                        <Monitor size={22} />
                        <h3>{b.display_name || b.name}</h3>
                        <span className="tag push">
                          {{
                            ready: "已启动",
                            running: "运行中",
                            busy: "正在使用",
                            not_started: "未启动",
                          }[b.state] || b.state}
                        </span>
                      </div>
                      <p className="mono wrap">{b.command}</p>
                      <div className="backend-stats">
                        <div className="soft">
                          <small>会话</small>
                          <div className="mono ellipsis" title={b.session}>
                            {b.session || "尚未生成"}
                          </div>
                        </div>
                        <div className="soft">
                          <small>工具</small>
                          <div>{b.tool_count ?? "未读取"}</div>
                        </div>
                        <div className="soft">
                          <small>近期调用</small>
                          <div>
                            {
                              tasks.filter(
                                (t) =>
                                  t.kind === "mcp.call" &&
                                  t.arguments.server ===
                                    (b.backend_name || b.name) &&
                                  (!b.connection_id ||
                                    t.connection_id === b.connection_id),
                              ).length
                            }
                          </div>
                        </div>
                      </div>
                      <button
                        disabled={!available || isPending("tools", b.name)}
                        aria-busy={isPending("tools", b.name)}
                        onClick={async () => {
                          const request = ++toolsRequest.current;
                          const trigger = document.activeElement as HTMLElement;
                          const r = await control("tools", { server: b.name });
                          if (!r || request !== toolsRequest.current) return;
                          if (!isToolPage(r)) {
                            setError("后端未返回有效工具列表，请重试。");
                            return;
                          }
                          toolsReturnFocus.current = trigger;
                          setToolsServer(b.name);
                          setTools(r);
                        }}
                      >
                        查看工具列表
                      </button>
                      <button
                        className="ghost"
                        disabled={
                          !available || isPending("restart_backend", b.name)
                        }
                        aria-busy={isPending("restart_backend", b.name)}
                        onClick={() =>
                          control("restart_backend", { server: b.name })
                        }
                      >
                        重启后端
                      </button>
                      <small className="backend-note">
                        重启会更换会话编号，Agent 需要重新发现工具并重新截图。
                      </small>
                    </article>
                  ))}
                  {!snapshot?.backends.length && (
                    <div className="card empty">
                      <Monitor />
                      <h3>尚未配置桌面后端</h3>
                      <p>
                        在设置中选择已有的
                        worker.toml。仅声明配置不代表权限已获授权。
                      </p>
                      <button onClick={() => setPage("settings")}>
                        前往设置
                      </button>
                    </div>
                  )}
                  <BackendPanel
                    snapshot={snapshot}
                    act={act}
                    app={app}
                    mode="advanced"
                  />
                  <BackendPanel
                    snapshot={snapshot}
                    act={act}
                    app={app}
                    mode="permissions"
                    active={page === "desktop"}
                    shared={permissionState}
                  />
                  {replayHistory.error && (
                    <p className="task-history-error" role="alert">
                      最近操作读取失败：{replayHistory.error}
                    </p>
                  )}
                  <Replay
                    tasks={replayHistory.tasks}
                    act={act}
                    onTasks={() => {
                      setFilter("all");
                      setKind("mcp.call");
                      setPage("tasks");
                    }}
                  />
                </>
              }
            />
          </RetainedPage>
        )}
        {visited.includes("settings") && (
          <RetainedPage active={page === "settings"}>
            <SettingsPage
              app={app}
              snapshot={snapshot}
              busy={busy}
              actionError={error}
              available={available}
              act={act}
              refresh={refresh}
              setError={setError}
              onPair={beginPairing}
              manualOpen={manualOpen}
            />
          </RetainedPage>
        )}
      </main>
      {tools && (
        <div className="scrim">
          <section
            className="dialog tools"
            ref={toolsDialog}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-label="后端工具列表"
          >
            <h2>后端工具列表</h2>
            <pre className="term" tabIndex={0} onKeyDown={selectLogText}>
              {JSON.stringify(tools, null, 2)}
            </pre>
            <div className="row between">
              <small>已读取 {tools.result?.tools?.length || 0} 个工具</small>
              <div className="actions">
                {tools.result?.nextCursor && (
                  <button
                    disabled={isPending("tools", toolsServer)}
                    aria-busy={isPending("tools", toolsServer)}
                    onClick={async () => {
                      const request = ++toolsRequest.current;
                      const next = await control("tools", {
                        server: toolsServer,
                        cursor: tools.result.nextCursor,
                      });
                      if (!next || request !== toolsRequest.current) return;
                      if (!isToolPage(next)) {
                        setError("后端未返回有效工具列表，请重试。");
                        return;
                      }
                      if (next.session !== tools.session) {
                        closeTools();
                        setError("后端会话已变化，请重新打开工具列表。");
                        return;
                      }
                      const merged = new Map<
                        string,
                        ToolPage["result"]["tools"][number]
                      >();
                      for (const item of [
                        ...(tools.result?.tools || []),
                        ...(next.result?.tools || []),
                      ])
                        merged.set(item.name, item);
                      setTools({
                        ...next,
                        result: { ...next.result, tools: [...merged.values()] },
                      });
                    }}
                  >
                    {isPending("tools", toolsServer)
                      ? "读取中…"
                      : "读取更多工具"}
                  </button>
                )}
                <button autoFocus onClick={closeTools}>
                  关闭
                </button>
              </div>
            </div>
            {error && (
              <p className="error-text" role="alert">
                {error}
              </p>
            )}
          </section>
        </div>
      )}
      {pairRequest !== null && (
        <div className="scrim">
          <section
            className="dialog"
            ref={pairDialog}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-labelledby="pair-request-title"
          >
            <h2 id="pair-request-title">
              {pairRequest ? "添加服务器" : "重新配对"}
            </h2>
            <p>
              需要先断开当前所有服务器，才能修改连接配置。已有连接和任务记录会保留。
            </p>
            <p className="muted" role="status">
              {snapshot?.active_count
                ? `还有 ${snapshot.active_count} 个任务正在运行，请等待任务结束后继续。`
                : available
                  ? "当前没有任务运行，断开后会进入配对页面。配对完成后可重新连接全部服务器。"
                  : "正在确认任务状态，请稍候；暂时无法确认时请取消并检查连接。"}
            </p>
            <div className="actions">
              <button
                disabled={disconnectingForPair}
                onClick={() => setPairRequest(null)}
              >
                取消
              </button>
              <button
                disabled={
                  busy ||
                  disconnectingForPair ||
                  !available ||
                  !!snapshot?.active_count
                }
                onClick={disconnectAndPair}
              >
                {disconnectingForPair
                  ? "正在断开…"
                  : pairRequest
                    ? "断开并添加"
                    : "断开并配对"}
              </button>
            </div>
            {error && (
              <p className="error-text" role="alert">
                {error}
              </p>
            )}
          </section>
        </div>
      )}
      {exitDialog}
    </div>
  );
}
function selectLogText(event: React.KeyboardEvent<HTMLPreElement>) {
  if (event.key.toLowerCase() !== "a" || (!event.metaKey && !event.ctrlKey))
    return;
  event.preventDefault();
  const range = document.createRange();
  range.selectNodeContents(event.currentTarget);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}
createRoot(document.getElementById("root")!).render(<App />);
