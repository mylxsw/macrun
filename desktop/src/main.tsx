import {
  Approvals,
  Pairing,
  WorkspaceList,
  BackendPanel,
  Replay,
} from "./Features";
import { SettingsPage } from "./SettingsPage";
import { useTaskHistory, useReplayHistory } from "./taskHistory";
import "./task-history.css";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import React, { useEffect, useState, useRef } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  List,
  MousePointer2,
  Shield,
  Terminal,
  Square,
  Pause,
  Play,
  ArrowUpRight,
  Search,
  Copy,
  RefreshCw,
  Monitor,
  Info,
  LogOut,
} from "lucide-react";
import { statuses, active, title, todaySummary } from "./model.mjs";
import type { Task, Snapshot, AppState } from "./types";
import "./style.css";
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
      dialog.querySelector<HTMLElement>(dialogControls)?.focus();
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

function App() {
  const firstLoad = useRef(true);
  const pendingActions = useRef(0);
  const quitReturnFocus = useRef<HTMLElement | null>(null);
  const toolsReturnFocus = useRef<HTMLElement | null>(null);
  const quitDialog = useRef<HTMLElement | null>(null);
  const toolsDialog = useRef<HTMLElement | null>(null);
  const toolsRequest = useRef(0);
  const trayContent = useRef<HTMLDivElement>(null);
  const [page, setPage] = useState("live"),
    [pairing, setPairing] = useState(false),
    [manualOpen, setManualOpen] = useState(false),
    [app, setApp] = useState<AppState | null>(null),
    [snapshot, setSnapshot] = useState<Snapshot | null>(null),
    [available, setAvailable] = useState(false),
    [error, setError] = useState(""),
    [refreshWarning, setRefreshWarning] = useState(""),
    [busy, setBusy] = useState(false),
    [filter, setFilter] = useState("all"),
    [query, setQuery] = useState(""),
    [selected, setSelected] = useState(""),
    [quit, setQuit] = useState(false),
    [tools, setTools] = useState<ToolPage | null>(null),
    [toolsServer, setToolsServer] = useState(""),
    [notice, setNotice] = useState("");
  const tasks = snapshot?.tasks || [],
    running = tasks.filter(active),
    connected = available && snapshot?.connection.state === "connected",
    paused = snapshot?.policy.paused || false;
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
    pendingActions.current += 1;
    setBusy(true);
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
      pendingActions.current -= 1;
      setBusy(pendingActions.current > 0);
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
    if (!isTauri) return;
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
        a ? { ...a, worker_running: true, worker_starting: false } : a,
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
        setQuery("");
      }
    });
    on("exit-requested", () => {
      if (!quitDialog.current)
        quitReturnFocus.current = document.activeElement as HTMLElement;
      setQuit(true);
    });
    on<string>("control-error", setError);
    const timer = setInterval(() => {
      refresh().catch(() => {});
    }, 4000);
    return () => {
      disposed = true;
      unsub.forEach((f) => f());
      clearInterval(timer);
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
    const current = tasks.filter((t) =>
      ["failed", "unknown", "timed_out"].includes(t.status),
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
    setQuery("");
    jump(tray ? `tasks:${t.task_id}` : "tasks");
  };
  const copy = async (s: string) => {
    try {
      await navigator.clipboard.writeText(s);
      setNotice("已复制任务编号");
    } catch {
      setError("复制失败，请手动选择任务编号");
    }
  };
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
  const recent = tasks.slice(0, 6);
  const approvals = tasks.filter((t) => t.status === "awaiting_approval");
  const history = useTaskHistory({
    enabled: page === "tasks" && !tray,
    available,
    snapshot,
    filter,
    query,
    selected,
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
  const nav = [
    ["live", "现场", Activity],
    ["tasks", "任务", List],
    ["desktop", "桌面控制", MousePointer2],
    ["settings", "设置与安全", Shield],
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
  if (tray)
    return (
      <div className="tray">
        <div ref={trayContent} className="tray-content">
          <header>
            <h2>
              <span
                className={`dot ${approvals.length ? "awaiting_approval" : stateClass}`}
              />
              {approvals.length ? "需要你确认" : stateText}
            </h2>
            <p>
              {running.length} 个任务进行中 ·{" "}
              {snapshot?.connection.server || "尚未配置服务器"}
            </p>
          </header>
          <Approvals tasks={tasks} act={act} disabled={busy || !available} />
          {!!running.filter((t) => t.status !== "awaiting_approval").length && (
            <div className="tray-current card">
              {running
                .filter((t) => t.status !== "awaiting_approval")
                .slice(0, 2)
                .map((t, i) => (
                  <button
                    className={`tray-task ${i ? "compact" : ""}`}
                    key={t.task_id}
                    onClick={() => choose(t)}
                  >
                    <div className="row between">
                      <small>
                        {kindLabel(t)} · {label(t.status)}
                      </small>
                      <span className="mono">
                        {t.progress
                          ? `${t.progress.received} / ${t.progress.total} 个文件`
                          : duration(t)}
                      </span>
                    </div>
                    {!i && <div className="mono ellipsis">{title(t)}</div>}
                  </button>
                ))}
            </div>
          )}
          {!connected && (
            <div className="alert error tray-connection">
              {snapshot?.connection.error ||
                "尚未连接服务器，请打开设置检查连接。"}
            </div>
          )}
          <button
            className="menuitem"
            disabled={!available || busy}
            onClick={pause}
          >
            {paused ? <Play /> : <Pause />}
            {paused ? "恢复接收任务" : "暂停接收新任务"}
          </button>
          <button
            className="menuitem"
            disabled={!available || busy}
            onClick={stop}
          >
            <Square />
            全部停止<small>⌃⌥⌘.</small>
          </button>
          <button
            className="menuitem"
            disabled={!available || busy}
            onClick={desktopToggle}
          >
            <MousePointer2 />
            允许桌面控制
            <small>{snapshot?.policy.desktop_enabled ? "✓" : ""}</small>
          </button>
          <hr />
          <small>最近</small>
          {tasks
            .filter((t) => !active(t))
            .slice(0, 3)
            .map((t) => (
              <button
                className="menuitem"
                onClick={() => choose(t)}
                key={t.task_id}
              >
                <span className={`dot ${t.status}`} />
                <span className="ellipsis">{title(t)}</span>
                <small>{time(t).slice(0, 5)}</small>
              </button>
            ))}
          {!recent.length && <p className="muted">暂无任务记录</p>}
          <hr />
          <button className="menuitem" onClick={() => jump("live")}>
            打开 Macrun<small>⌘O</small>
          </button>
          <button className="menuitem" onClick={() => jump("settings")}>
            设置…<small>⌘,</small>
          </button>
          <button
            className="menuitem"
            onClick={() => {
              act("request_quit");
            }}
          >
            退出 Macrun<small>⌘Q</small>
          </button>
          {feedback}
          {exitDialog}
        </div>
      </div>
    );
  if (pairing)
    return (
      <div className="pairing-window">
        <div className="pairing-feedback">{feedback}</div>
        {exitDialog}
        <Pairing
          act={act}
          running={!!app?.worker_running}
          onClose={() => {
            setPairing(false);
            setPage("settings");
          }}
          onManual={() => {
            setPairing(false);
            setManualOpen(true);
            setPage("settings");
          }}
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
        <div className="brand">
          <div className="logo">
            <Terminal size={19} />
          </div>
          <div>
            <b>Macrun</b>
            <small>
              执行器 · {snapshot ? `v${snapshot.version}` : "未启动"}
            </small>
          </div>
        </div>
        <nav aria-label="主导航">
          {nav.map(([id, text, Icon]) => (
            <button
              aria-current={page === id ? "page" : undefined}
              className={page === id ? "on" : ""}
              key={id}
              onClick={() => setPage(id)}
            >
              <Icon size={17} />
              {text}
              {id === "live" && running.length > 0 && (
                <span>{running.length}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-status">
          <div className="row">
            <span className={`dot ${connected ? "succeeded" : "failed"}`} />
            <b>{connected ? "已连接" : "尚未连接"}</b>
            <span className="mono muted push">
              {snapshot?.connection.rtt_ms != null
                ? `${snapshot.connection.rtt_ms} ms`
                : ""}
            </span>
          </div>
          <small className="mono">
            {snapshot?.connection.server || app?.settings.server || "尚未连接"}
          </small>
        </div>
      </aside>
      <main className={`content page-${page}`} key={page}>
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
        {page === "live" && (
          <>
            <header className="page-header">
              <div>
                <h1>现场</h1>
                <p>服务器上的 Agent 正通过 Macrun 在这台 Mac 上做的事。</p>
              </div>
              <div className="actions">
                <button disabled={!available || busy} onClick={pause}>
                  {paused ? <Play size={15} /> : <Pause size={15} />}{" "}
                  {paused ? "恢复接收" : "暂停接收新任务"}
                </button>
                <button
                  className="danger"
                  disabled={!available || busy}
                  onClick={stop}
                >
                  <Square size={15} />
                  全部停止 <small>⌃⌥⌘.</small>
                </button>
              </div>
            </header>
            <Approvals tasks={tasks} act={act} disabled={busy || !available} />
            {paused && (
              <div className="alert">
                已暂停接收新任务。正在运行的任务继续完成；新请求返回 busy。
              </div>
            )}
            <div className="status-grid">
              <StatusCard
                title="连接"
                status={connected ? "succeeded" : "failed"}
                value={connected ? "已连接服务器" : "尚未连接服务器"}
                detail={
                  snapshot?.connection.error ||
                  (connected
                    ? `QUIC · ${snapshot?.connection.rtt_ms ?? "—"} ms · 已连接 ${Math.floor((Date.now() - (snapshot?.connection.since || Date.now())) / 60000)} 分钟`
                    : "配置连接后启动执行器")
                }
              />
              <StatusCard
                title="执行器"
                status={
                  available ? (paused ? "cancelled" : "succeeded") : "cancelled"
                }
                value={
                  available
                    ? paused
                      ? "已暂停接收"
                      : "运行中 · 接收任务"
                    : app?.worker_starting
                      ? "正在读取凭据并启动"
                      : app?.worker_running
                        ? "正在启动／连接本机控制"
                        : "未运行"
                }
                detail={
                  snapshot
                    ? `v${snapshot.version} · 协议 ${snapshot.protocol} · 桌面应用托管`
                    : "由 Macrun Desktop 托管"
                }
              />
              <StatusCard
                title="桌面控制"
                status={
                  snapshot?.policy.desktop_enabled ? "succeeded" : "cancelled"
                }
                value={
                  snapshot?.policy.desktop_enabled
                    ? `${snapshot?.backends[0]?.name || "桌面控制"} · 已允许`
                    : "桌面控制已关闭"
                }
                detail={
                  snapshot?.backends.length
                    ? `${snapshot.backends.length} 个后端已配置，权限须实测`
                    : "尚未配置后端；命令和文件能力独立可用"
                }
              />
            </div>
            <div className="live-grid">
              <section>
                <h2>
                  正在进行 <span className="tag">{running.length}</span>
                </h2>
                {running.map((t) => (
                  <TaskCard
                    key={t.task_id}
                    task={t}
                    view={() => choose(t)}
                    cancel={() => control("cancel", { task_id: t.task_id })}
                    disabled={!available || busy}
                    openDirectory={() =>
                      act("open_workspace", {
                        root: t.arguments.cwd || t.arguments.remote_root,
                        terminal: true,
                        taskId: t.task_id,
                      })
                    }
                  />
                ))}
                {!running.length && (
                  <div className="card empty">
                    <Activity size={28} />
                    <h3>
                      {available
                        ? "等待 Agent 发起任务"
                        : app?.worker_starting
                          ? "正在启动执行器"
                          : "连接你的服务器"}
                    </h3>
                    <p>
                      {available
                        ? "新任务、命令输出和同步进度会实时显示在这里。"
                        : app?.worker_starting
                          ? "如有钥匙串授权窗口，请完成系统确认。连接成功后，任务会自动显示。"
                          : "启动执行器后，在这里查看这台电脑上的执行情况。"}
                    </p>
                    {!available && (
                      <button
                        className="primary"
                        onClick={() => setPage("settings")}
                      >
                        配置连接
                      </button>
                    )}
                  </div>
                )}
              </section>
              <aside>
                <WorkspaceList snapshot={snapshot} act={act} />
                <div className="row between section-heading">
                  <h2>刚刚</h2>
                  <button className="link" onClick={() => setPage("tasks")}>
                    全部任务
                  </button>
                </div>
                <div className="card recent">
                  {recent.map((t) => (
                    <button key={t.task_id} onClick={() => choose(t)}>
                      <small className="mono">{time(t)}</small>
                      <span className={`dot ${t.status}`} />
                      <span className="ellipsis">{title(t)}</span>
                      <small className="mono">{duration(t)}</small>
                    </button>
                  ))}
                  {!recent.length && (
                    <p className="muted">任务结束后显示在这里。</p>
                  )}
                </div>
                <p className="muted foot">
                  今天 {summary.total} 个任务：成功 {summary.succeeded || 0} ·
                  失败 {summary.failed || 0} · 未知 {summary.unknown || 0}
                </p>
                {(snapshot?.total_tasks || 0) > 200 && (
                  <p className="muted">
                    在任务页查看完整历史记录，支持按状态、命令和目录查找。
                  </p>
                )}
              </aside>
            </div>
          </>
        )}
        {page === "tasks" && (
          <>
            <header className="page-header">
              <div>
                <h1>任务</h1>
                <p>每个任务按编号只记一次。查询、心跳和重复请求不计数。</p>
              </div>
              <label className="search">
                <Search size={17} />
                <input
                  aria-label="搜索任务"
                  placeholder="命令、目录或任务编号"
                  value={query}
                  onChange={(e) => {
                    setQuery(e.target.value);
                    setSelected("");
                  }}
                />
              </label>
            </header>
            <div className="filters">
              {[["all", "全部"], ...Object.entries(statuses)].map(([k, v]) => (
                <button
                  aria-pressed={filter === k}
                  key={k}
                  className={filter === k ? "on" : ""}
                  onClick={() => {
                    setFilter(k);
                    setSelected("");
                  }}
                >
                  <span className={`dot ${k}`} />
                  {v}
                  <small>
                    {k === "all"
                      ? Object.values(history.counts).reduce(
                          (sum, count) => sum + count,
                          0,
                        )
                      : history.counts[k] || 0}
                  </small>
                </button>
              ))}
            </div>
            <div className="task-grid">
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
                  className="card task-list"
                  aria-label="任务列表"
                  ref={history.list}
                  aria-busy={history.loading}
                >
                  {filtered.map((t) => (
                    <button
                      className={sel?.task_id === t.task_id ? "selected" : ""}
                      key={t.task_id}
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
                      <span className={`dot ${t.status}`} />
                      <div className="grow">
                        <div className="mono ellipsis">{title(t)}</div>
                        <small>
                          {kindLabel(t)} · {time(t)}
                        </small>
                      </div>
                      <div className="right">
                        <span className={`text-${t.status}`}>
                          {label(t.status)}
                        </span>
                        <small className="mono">{duration(t)}</small>
                      </div>
                    </button>
                  ))}
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
                      if (window.innerWidth <= 1000)
                        document
                          .querySelector<HTMLElement>('[aria-label="任务详情"]')
                          ?.scrollIntoView?.({
                            block: "nearest",
                            behavior: "smooth",
                          });
                    }}
                  >
                    上一页
                  </button>
                  <button
                    disabled={!history.next || history.loading}
                    onClick={() => {
                      setSelected("");
                      history.next?.();
                      if (window.innerWidth <= 1000)
                        document
                          .querySelector<HTMLElement>('[aria-label="任务详情"]')
                          ?.scrollIntoView?.({
                            block: "nearest",
                            behavior: "smooth",
                          });
                    }}
                  >
                    下一页
                  </button>
                </nav>
              </div>
              <section className="card task-detail" aria-label="任务详情">
                {history.detailError && (
                  <div className="task-history-error" role="alert">
                    任务详情读取失败：{history.detailError}{" "}
                    <button onClick={history.reload}>重试读取详情</button>
                  </div>
                )}
                {sel ? (
                  <>
                    <div className="row between">
                      <div className="row">
                        <Status status={sel.status} />
                        <span className="tag">{kindLabel(sel)}</span>
                      </div>
                      <small>{time(sel)}</small>
                    </div>
                    <h3 className="mono command">{title(sel)}</h3>
                    {sel.status === "unknown" && (
                      <div className="alert task-explanation">
                        <Info size={16} />
                        <span>
                          操作可能已经生效，也可能没有。Macrun
                          不会自动重放。请核对本机状态后再决定下一步。
                        </span>
                      </div>
                    )}
                    {sel.error && sel.status !== "unknown" && (
                      <p className="error-text wrap">{sel.error.message}</p>
                    )}
                    <dl>
                      <dt>工作目录</dt>
                      <dd className="mono">
                        {sel.arguments.cwd || sel.arguments.remote_root || "—"}
                      </dd>
                      <dt>耗时</dt>
                      <dd className="mono">{duration(sel)}</dd>
                      <dt>退出码</dt>
                      <dd className="mono">{sel.result?.exit_code ?? "—"}</dd>
                      <dt>任务编号</dt>
                      <dd className="mono">{sel.task_id}</dd>
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
                    </dl>
                    <div className="row between">
                      <small>输出</small>
                      <small>最后 8 KB</small>
                    </div>
                    <pre className="term">
                      {history.detailLoading
                        ? "正在读取输出…"
                        : sel.output_tail ||
                          sel.error?.message ||
                          "暂无文本输出"}
                    </pre>
                    <div className="actions">
                      <button onClick={() => copy(sel.task_id)}>
                        <Copy size={14} />
                        复制任务编号
                      </button>
                      <button
                        onClick={() => act("open_log", { taskId: sel.task_id })}
                      >
                        打开完整日志
                      </button>
                      {sel.status === "unknown" && (
                        <button onClick={() => setPage("desktop")}>
                          前往桌面控制核对
                        </button>
                      )}
                      {active(sel) && (
                        <button
                          disabled={!available || busy}
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
                ) : (
                  <div className="empty">选择任务查看结果和输出。</div>
                )}
              </section>
            </div>
          </>
        )}
        {page === "desktop" && (
          <>
            <header className="page-header">
              <div>
                <h1>桌面控制</h1>
                <p>通过本机 MCP 后端操作桌面。命令与文件不受这里的开关影响。</p>
              </div>
              <label className="card switch-label">
                允许 Agent 操作桌面
                <input
                  className="switch"
                  type="checkbox"
                  checked={snapshot?.policy.desktop_enabled || false}
                  disabled={!available || busy}
                  onChange={desktopToggle}
                />
              </label>
            </header>

            <div className="desktop-grid">
              <section>
                <h2>后端</h2>
                {snapshot?.backends.map((b) => (
                  <article className="card backend" key={b.name}>
                    <div className="row">
                      <Monitor size={22} />
                      <h3>{b.name}</h3>
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
                                t.arguments.server === b.name,
                            ).length
                          }
                        </div>
                      </div>
                    </div>
                    <button
                      disabled={!available || busy}
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
                      disabled={!available || busy}
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
              </section>
              <BackendPanel
                snapshot={snapshot}
                act={act}
                app={app}
                mode="permissions"
              />
            </div>
            <h2 className="section-heading">操作时</h2>
            <div className="card behavior-card">
              {(
                [
                  [
                    "show_overlay",
                    "显示屏幕边框和悬浮提示",
                    "Agent 操作期间，屏幕四周显示橙色描边，顶部显示正在做什么和停止按钮。",
                  ],
                  [
                    "yield_input",
                    "我动鼠标或键盘时让出",
                    "检测到本地输入后暂停新的桌面操作 30 秒；已发出的点击不会撤回。",
                  ],
                ] as const
              ).map(([key, text, detail]) => (
                <label className="line" key={key}>
                  <div className="grow">
                    <b>{text}</b>
                    <small>{detail}</small>
                  </div>
                  <input
                    className="switch"
                    type="checkbox"
                    checked={app?.preferences[key] || false}
                    disabled={!app || busy}
                    onChange={(e) =>
                      act("save_preferences", {
                        preferences: {
                          ...app!.preferences,
                          [key]: e.target.checked,
                        },
                      })
                    }
                  />
                </label>
              ))}
              <div className="line">
                <div className="grow">
                  <b>紧急停止快捷键</b>
                  <small>
                    取消所有运行中的任务，暂停接收，并关闭桌面控制。
                  </small>
                </div>
                <kbd>⌃ ⌥ ⌘ .</kbd>
              </div>
            </div>
            {replayHistory.error && (
              <p className="task-history-error" role="alert">
                最近操作读取失败：{replayHistory.error}
              </p>
            )}
            <Replay
              tasks={replayHistory.tasks}
              act={act}
              onTasks={() => setPage("tasks")}
            />
          </>
        )}
        {page === "settings" && (
          <SettingsPage
            app={app}
            snapshot={snapshot}
            busy={busy}
            actionError={error}
            available={available}
            act={act}
            refresh={refresh}
            setError={setError}
            onPair={() => setPairing(true)}
            manualOpen={manualOpen}
          />
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
            <pre className="term">{JSON.stringify(tools, null, 2)}</pre>
            <div className="row between">
              <small>已读取 {tools.result?.tools?.length || 0} 个工具</small>
              <div className="actions">
                {tools.result?.nextCursor && (
                  <button
                    disabled={busy}
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
                    {busy ? "读取中…" : "读取更多工具"}
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
      {exitDialog}
    </div>
  );
}
function TerminalTail({ text }: { text: string }) {
  const ref = useRef<HTMLPreElement>(null),
    follow = useRef(true);
  useEffect(() => {
    if (ref.current && follow.current)
      ref.current.scrollTop = ref.current.scrollHeight;
  }, [text]);
  return (
    <pre
      className="term"
      ref={ref}
      onScroll={() => {
        const el = ref.current;
        if (el)
          follow.current =
            el.scrollHeight - el.scrollTop - el.clientHeight < 30;
      }}
    >
      {text}
    </pre>
  );
}
function StatusCard({
  title,
  status,
  value,
  detail,
}: {
  title: string;
  status: string;
  value: string;
  detail: string;
}) {
  return (
    <article className="card status-card">
      <small>
        <span className={`dot ${status}`} />
        {title}
      </small>
      <h3>{value}</h3>
      <small className="wrap">{detail}</small>
    </article>
  );
}
function TaskCard({
  task: t,
  view,
  cancel,
  disabled,
  openDirectory,
}: {
  task: Task;
  view: () => void;
  cancel: () => void;
  disabled: boolean;
  openDirectory: () => void;
}) {
  const sync = t.kind === "sync";
  return (
    <article className={`card active-task ${sync ? "sync-task" : ""}`}>
      <div className="row task-meta">
        <span className={`dot ${t.status}`} />
        <span className={`tag ${sync ? "succeeded" : "running"}`}>
          {kindLabel(t)}
        </span>
        <small>
          {label(t.status)} · 已用时 <span className="mono">{duration(t)}</span>
        </small>
        {t.progress && (
          <small className="push">
            {t.progress.total} 个文件中已收到 {t.progress.received} 个 ·{" "}
            {Math.round(t.progress.bytes / 1024)} KB
          </small>
        )}
      </div>
      <h3 className="mono command">
        {sync ? (
          <button
            className="sync-title-button"
            onClick={view}
            aria-label={`查看同步详情：${title(t)}`}
          >
            {title(t)}
          </button>
        ) : (
          title(t)
        )}
      </h3>
      {sync ? (
        <small>
          同步只更新文件，不会自动触发构建。完成前请勿依赖这个目录的内容。
        </small>
      ) : (
        <>
          <small className="mono wrap">
            {t.arguments.cwd || t.arguments.remote_root || ""} · task{" "}
            {t.task_id.slice(0, 8)}…
          </small>
          <TerminalTail text={t.output_tail || "等待任务输出…"} />
        </>
      )}
      {!sync && (
        <div className="actions">
          <button onClick={view}>查看任务详情</button>
          {(t.arguments.cwd || t.arguments.remote_root) && (
            <button className="ghost" onClick={openDirectory}>
              在终端打开目录
            </button>
          )}
          <button disabled={disabled} className="danger push" onClick={cancel}>
            取消任务
          </button>
        </div>
      )}
    </article>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
