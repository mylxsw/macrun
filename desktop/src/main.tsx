import React, { useEffect, useState } from "react";
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
  Folder,
  ArrowUpRight,
  Search,
  Copy,
  RefreshCw,
  Monitor,
  Info,
  LogOut,
} from "lucide-react";
import {
  statuses,
  active,
  title,
  selectTasks,
  todaySummary,
} from "./model.mjs";
import type { Task, Snapshot, AppState, Settings } from "./types";
import "./style.css";
const isTauri = !!(window as any).__TAURI_INTERNALS__;
const tray = new URLSearchParams(location.search).has("tray");
const emptySettings: Settings = {
  server: "",
  cert: "",
  token_file: "",
  backend_config: "",
};
const label = (s: string) => (statuses as Record<string, string>)[s] || s;
const duration = (t: Task) => {
  const sec = Math.max(
    0,
    Math.floor(((t.ended_at || Date.now()) - t.started_at) / 1000),
  );
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;
};
const time = (t: Task) =>
  new Date(t.started_at).toLocaleTimeString("zh-CN", { hour12: false });
function Status({ status }: { status: string }) {
  return (
    <span className={`tag ${status}`}>
      <i className="dot" />
      {label(status)}
    </span>
  );
}
function App() {
  const [page, setPage] = useState("live"),
    [app, setApp] = useState<AppState | null>(null),
    [snapshot, setSnapshot] = useState<Snapshot | null>(null),
    [available, setAvailable] = useState(false),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [filter, setFilter] = useState("all"),
    [query, setQuery] = useState(""),
    [selected, setSelected] = useState(""),
    [quit, setQuit] = useState(false),
    [tools, setTools] = useState<any>(null),
    [notice, setNotice] = useState("");
  const tasks = snapshot?.tasks || [],
    running = tasks.filter(active),
    connected = available && snapshot?.connection.state === "connected",
    paused = snapshot?.policy.paused || false;
  const refresh = async () => {
    if (!isTauri) return;
    const a = await invoke<AppState>("app_state");
    setApp(a);
    if (a.snapshot) setSnapshot(a.snapshot);
  };
  const act = async (
    command: string,
    args: Record<string, unknown> = {},
    success?: string,
  ) => {
    setError("");
    setBusy(true);
    try {
      const r = await invoke(command, args);
      if (success) setNotice(success);
      await refresh();
      return r ?? true;
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const control = (action: string, args: Record<string, unknown> = {}) =>
    act("control", { action, args });
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
      setApp((a) => (a ? { ...a, worker_running: true } : a));
    });
    on("worker-unavailable", () => {
      setAvailable(false);
      refresh().catch(() => {});
    });
    on<string>("navigate", setPage);
    on("exit-requested", () => setQuit(true));
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
    if (!notice) return;
    const t = setTimeout(() => setNotice(""), 4000);
    return () => clearTimeout(t);
  }, [notice]);
  const jump = (p: string) =>
    tray ? act("open_main", { route: p }) : setPage(p);
  const stop = () => control("stop_all");
  const pause = () => control("pause", { paused: !paused });
  const desktopToggle = () =>
    control("desktop", { enabled: !snapshot?.policy.desktop_enabled });
  const choose = (t: Task) => {
    setSelected(t.task_id);
    jump("tasks");
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
  const recent = tasks.filter((t) => !active(t)).slice(0, 6);
  const filtered = selectTasks(tasks, filter, query) as Task[];
  const sel = filtered.find((t) => t.task_id === selected) || filtered[0];
  const summary = todaySummary(tasks) as Record<string, number>;
  const nav = [
    ["live", "现场", Activity],
    ["tasks", "任务", List],
    ["desktop", "桌面控制", MousePointer2],
    ["settings", "设置与安全", Shield],
  ] as const;
  const feedback = (
    <>
      {error && (
        <div className="alert error" role="alert">
          <Info size={16} />
          <span>{error}</span>
          <button onClick={() => setError("")} aria-label="关闭错误提示">
            ×
          </button>
        </div>
      )}
      {notice && (
        <div className="toast" role="status">
          {notice}
        </div>
      )}
    </>
  );
  const exitDialog = quit && (
    <div className="scrim">
      <section
        className="dialog"
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
  if (tray)
    return (
      <div className="tray">
        <header>
          <h2>
            <span className={`dot ${stateClass}`} />
            {stateText}
          </h2>
          <p>
            {running.length} 个任务进行中 ·{" "}
            {snapshot?.connection.server || "尚未配置服务器"}
          </p>
        </header>
        {running.slice(0, 2).map((t) => (
          <button
            className="tray-task"
            key={t.task_id}
            onClick={() => choose(t)}
          >
            <div className="row between">
              <small>
                {t.kind === "sync" ? "同步" : "任务"} · {label(t.status)}
              </small>
              <small className="mono">{duration(t)}</small>
            </div>
            <div className="mono ellipsis">{title(t)}</div>
            {t.progress && (
              <small>
                {t.progress.received} / {t.progress.total} 个文件
              </small>
            )}
          </button>
        ))}
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
          {snapshot?.policy.desktop_enabled ? "关闭桌面控制" : "允许桌面控制"}
        </button>
        <hr />
        <small>最近</small>
        {recent.slice(0, 3).map((t) => (
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
          打开 Macrun<small>↗</small>
        </button>
        <button className="menuitem" onClick={() => jump("settings")}>
          设置…
        </button>
        <button
          className="menuitem"
          onClick={() => {
            act("request_quit");
          }}
        >
          退出 Macrun
        </button>
        {feedback}
        {exitDialog}
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
            <small>执行器 · {snapshot?.version || "未启动"}</small>
          </div>
        </div>
        <nav aria-label="主导航">
          {nav.map(([id, text, Icon]) => (
            <button
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
          <b>
            <span className={`dot ${stateClass}`} />
            {stateText}
          </b>
          <small className="mono">
            {snapshot?.connection.server || app?.settings.server || "尚未连接"}
          </small>
        </div>
      </aside>
      <main className="content">
        <div className="drag title-drag" data-tauri-drag-region />
        {!isTauri && (
          <div className="alert">
            请在 Tauri
            桌面应用中打开。浏览器仅显示空状态布局，不连接本机执行器。
          </div>
        )}
        {feedback}
        {page === "live" && (
          <>
            <header className="page-header">
              <div>
                <h1>现场</h1>
                <p>服务器上的 Agent 正通过 Macrun 在这台电脑上做的事。</p>
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
                  snapshot?.policy.desktop_enabled ? "unknown" : "cancelled"
                }
                value={
                  snapshot?.policy.desktop_enabled
                    ? "已允许桌面请求"
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
                  />
                ))}
                {!running.length && (
                  <div className="card empty">
                    <Activity size={28} />
                    <h3>
                      {available ? "等待 Agent 发起任务" : "连接你的服务器"}
                    </h3>
                    <p>
                      {available
                        ? "新任务、命令输出和同步进度会实时显示在这里。"
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
                <h2>工作区</h2>
                <div className="card workspace-list">
                  {[
                    ...new Set(
                      tasks
                        .filter((t) => t.kind === "sync")
                        .map((t) => t.arguments.remote_root),
                    ),
                  ].map((path) => (
                    <div className="line" key={path}>
                      <Folder size={17} />
                      <div className="mono wrap">{path}</div>
                    </div>
                  ))}
                  {!tasks.some((t) => t.kind === "sync") && (
                    <p className="muted">尚无同步记录</p>
                  )}
                </div>
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
                    仅展示最近 200 条，今日汇总可能不完整。
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
                  onChange={(e) => setQuery(e.target.value)}
                />
              </label>
            </header>
            <div className="filters">
              {[["all", "全部"], ...Object.entries(statuses)].map(([k, v]) => (
                <button
                  aria-pressed={filter === k}
                  key={k}
                  className={filter === k ? "on" : ""}
                  onClick={() => setFilter(k)}
                >
                  <span className={`dot ${k}`} />
                  {v}
                  <small>
                    {k === "all"
                      ? tasks.length
                      : tasks.filter((t) => t.status === k).length}
                  </small>
                </button>
              ))}
            </div>
            <div className="task-grid">
              <section className="card task-list" aria-label="任务列表">
                {filtered.map((t) => (
                  <button
                    className={sel?.task_id === t.task_id ? "selected" : ""}
                    key={t.task_id}
                    onClick={() => setSelected(t.task_id)}
                  >
                    <span className={`dot ${t.status}`} />
                    <div className="grow">
                      <div className="mono ellipsis">{title(t)}</div>
                      <small>
                        {t.kind} · {time(t)}
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
                  <div className="empty">没有匹配的任务</div>
                )}
              </section>
              <section className="card task-detail" aria-label="任务详情">
                {sel ? (
                  <>
                    <div className="row between">
                      <div className="row">
                        <Status status={sel.status} />
                        <span className="tag">{sel.kind}</span>
                      </div>
                      <small>{time(sel)}</small>
                    </div>
                    <h3 className="mono command">{title(sel)}</h3>
                    {sel.status === "unknown" && (
                      <div className="alert">
                        操作可能已经生效，也可能没有。Macrun
                        不会自动重放。请核对本机状态后再决定下一步。
                      </div>
                    )}
                    {sel.error && (
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
                      {sel.output_tail || sel.error?.message || "暂无文本输出"}
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
                允许桌面控制
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
                          busy: "正在使用",
                          not_started: "未启动",
                        }[b.state] || b.state}
                      </span>
                    </div>
                    <p className="mono wrap">{b.command}</p>
                    <div className="soft">
                      <small>会话</small>
                      <div className="mono wrap">
                        {b.session || "工具发现后生成"}
                      </div>
                    </div>
                    <button
                      disabled={!available || busy}
                      onClick={async () => {
                        const r = await control("tools", { server: b.name });
                        if (r) setTools(r);
                      }}
                    >
                      查看工具列表
                    </button>
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
              </section>
              <section>
                <h2>这台电脑是否具备条件</h2>
                <div className="card">
                  {[
                    ["辅助功能", "尚未通过桌面后端验证"],
                    ["屏幕录制", "需要实际截图验证，不能仅凭工具已连接判断"],
                    ["图形登录会话", "请确保本机用户已登录且屏幕未锁定"],
                    ["自动休眠", "休眠期间连接与任务可能中断"],
                  ].map(([t, d]) => (
                    <div className="line" key={t}>
                      <Info size={18} className="amber" />
                      <div>
                        <b>{t}</b>
                        <small>{d}</small>
                      </div>
                      <span className="tag push">未验证</span>
                    </div>
                  ))}
                </div>
              </section>
            </div>
            <h2 className="section-heading">操作时</h2>
            <div className="card">
              <div className="line">
                <div>
                  <b>紧急停止快捷键</b>
                  <small>取消所有运行任务、暂停接收，并关闭桌面控制。</small>
                </div>
                <kbd className="push">⌃ ⌥ ⌘ .</kbd>
              </div>
              <div className="line">
                <div>
                  <b>屏幕提示与本地输入让出</b>
                  <small>按设计规划在下一阶段接入；当前版本未启用。</small>
                </div>
                <span className="tag push">尚未实现</span>
              </div>
            </div>
          </>
        )}
        {page === "settings" && (
          <SettingsPage
            app={app}
            snapshot={snapshot}
            busy={busy}
            available={available}
            act={act}
            refresh={refresh}
            setError={setError}
          />
        )}
        <footer>
          Macrun Desktop · 开发版本 ·{" "}
          {isTauri ? "真实本机状态" : "浏览器空状态预览"}
        </footer>
      </main>
      {exitDialog}
      {tools && (
        <div className="scrim">
          <section
            className="dialog tools"
            role="dialog"
            aria-modal="true"
            aria-label="后端工具列表"
          >
            <h2>后端工具列表</h2>
            <pre className="term">{JSON.stringify(tools, null, 2)}</pre>
            <button autoFocus onClick={() => setTools(null)}>
              关闭
            </button>
          </section>
        </div>
      )}
    </div>
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
}: {
  task: Task;
  view: () => void;
  cancel: () => void;
  disabled: boolean;
}) {
  return (
    <article className="card active-task">
      <div className="row">
        <Status status={t.status} />
        <span className="tag">{t.kind}</span>
        <small>
          已用时 <span className="mono">{duration(t)}</span>
        </small>
      </div>
      <h3 className="mono command">{title(t)}</h3>
      <small className="mono wrap">
        {t.arguments.cwd || t.arguments.remote_root || ""} ·{" "}
        {t.task_id.slice(0, 8)}…
      </small>
      {t.progress ? (
        <p className="muted">
          已收到 {t.progress.received} / {t.progress.total} 个文件 ·{" "}
          {t.progress.bytes} 字节
        </p>
      ) : (
        <pre className="term">{t.output_tail || "等待任务输出…"}</pre>
      )}
      <div className="actions">
        <button onClick={view}>
          查看完整输出 <ArrowUpRight size={14} />
        </button>
        <button disabled={disabled} className="danger push" onClick={cancel}>
          取消任务
        </button>
      </div>
    </article>
  );
}
function SettingsPage({
  app,
  snapshot,
  busy,
  available,
  act,
  refresh,
  setError,
}: {
  app: AppState | null;
  snapshot: Snapshot | null;
  busy: boolean;
  available: boolean;
  act: (c: string, a?: Record<string, unknown>, s?: string) => Promise<unknown>;
  refresh: () => Promise<void>;
  setError: (s: string) => void;
}) {
  const [settings, setSettings] = useState(app?.settings || emptySettings),
    [dirty, setDirty] = useState(false);
  useEffect(() => {
    if (app && !dirty) setSettings(app.settings);
  }, [app, dirty]);
  const update = (k: keyof Settings, v: string) => {
    setDirty(true);
    setSettings((s) => ({ ...s, [k]: v }));
  };
  return (
    <>
      <header className="page-header">
        <div>
          <h1>设置与安全</h1>
          <p>连接 · 安全边界 · 通用 · 诊断</p>
        </div>
      </header>
      <h2>连接</h2>
      <form
        className="card connection-form"
        onSubmit={async (e) => {
          e.preventDefault();
          const saved = await act(
            "save_settings",
            { settings },
            "连接配置已保存",
          );
          if (saved !== undefined) setDirty(false);
        }}
      >
        <div className="alert">
          当前为 M0
          开发版本，使用已有证书与令牌文件连接。一次性配对和钥匙串存储将在 M1
          接入。
        </div>
        {(
          [
            ["server", "服务器地址", "server.example:7443"],
            ["cert", "服务器证书路径", "/absolute/path/cert.der"],
            ["token_file", "令牌文件路径", "/absolute/path/token"],
            [
              "backend_config",
              "桌面后端配置（可选）",
              "/absolute/path/worker.toml",
            ],
          ] as const
        ).map(([k, l, p]) => (
          <label className="field" key={k}>
            <span>{l}</span>
            <input
              required={k !== "backend_config"}
              value={settings[k]}
              placeholder={p}
              onChange={(e) => update(k, e.target.value)}
              disabled={app?.worker_running}
            />
          </label>
        ))}
        <small>
          只保存文件路径，不在页面显示令牌内容。执行器使用独立的数据目录。
        </small>
        <div className="actions">
          <button type="submit" disabled={busy || app?.worker_running}>
            保存配置
          </button>
          <button
            className="primary"
            type="button"
            disabled={
              busy ||
              dirty ||
              !app?.settings.server ||
              app?.worker_running ||
              app?.legacy_running
            }
            onClick={() => act("start_worker", {}, "已请求启动执行器")}
          >
            启动并连接
          </button>
          <button
            className="danger"
            type="button"
            disabled={busy || !available || !!snapshot?.active_count}
            onClick={() => act("stop_worker", {}, "已请求断开连接")}
          >
            断开连接
          </button>
        </div>
        {!!snapshot?.active_count && (
          <small>有任务运行时，请先“全部停止”，等待任务结束后再断开。</small>
        )}
      </form>
      <h2 className="section-heading">安全边界</h2>
      <div className="card">
        <div className="line">
          <div>
            <b>执行器内的控制</b>
            <small>
              暂停接收、全部停止、桌面控制开关已接入。控制接口不作为远程工具暴露。
            </small>
          </div>
        </div>
        <div className="line">
          <div>
            <b>环境变量值不写入任务参数记录</b>
            <small>
              参数只保留变量名及遮盖值；命令自身输出的秘密仍可能出现在日志。
            </small>
          </div>
        </div>
        <div className="line">
          <div>
            <b>目录白名单与命令确认</b>
            <small>
              尚未实现。当前命令以本机用户权限直接执行，同用户命令仍能访问本机资源，当前不是安全隔离环境。
            </small>
          </div>
          <span className="tag push">下一阶段</span>
        </div>
      </div>
      <h2 className="section-heading">通用</h2>
      {app?.legacy_running && (
        <div className="alert">
          检测到旧 LaunchAgent
          dev.macrun.worker。为避免重复运行，已禁用新执行器启动。本版不会自动迁移或停止现有服务。
        </div>
      )}
      <div className="card">
        <label className="line">
          <div className="grow">
            <b>登录时启动</b>
            <small>图形登录后启动桌面应用，不安装后台系统服务。</small>
          </div>
          <input
            type="checkbox"
            className="switch"
            disabled={busy}
            checked={app?.autostart || false}
            onChange={async (e) => {
              await act("autostart", { enabled: e.target.checked });
              refresh().catch((e) => setError(String(e)));
            }}
          />
        </label>
        <div className="line">
          <div>
            <b>关闭窗口后继续在菜单栏运行</b>
            <small>
              通过“退出 Macrun”停止执行器；应用异常退出后 worker
              会检测到父进程管道关闭。
            </small>
          </div>
          <span className="tag push">已启用</span>
        </div>
      </div>
      <h2 className="section-heading">诊断</h2>
      <div className="card">
        <div className="line">
          <span className="muted">执行器版本</span>
          <span className="mono">
            {snapshot?.version || "未启动"} · 协议 {snapshot?.protocol || "—"}
          </span>
        </div>
        <div className="line">
          <span className="muted">数据目录</span>
          <span className="mono wrap">
            {app?.data_dir || "仅在桌面应用中可用"}
          </span>
        </div>
        <div className="line">
          <button
            disabled={busy}
            onClick={() => act("open_log", { taskId: null })}
          >
            <Terminal size={15} />
            查看运行日志
          </button>
          <button
            disabled={busy || !available}
            onClick={() =>
              act(
                "control",
                { action: "snapshot", args: {} },
                "本机控制连接正常",
              )
            }
          >
            <RefreshCw size={15} />
            检查本机连接
          </button>
        </div>
      </div>
    </>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
