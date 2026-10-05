import { useRef, useState } from "react";
import {
  Check,
  ChevronRight,
  CircleAlert,
  Ellipsis,
  MousePointer2,
  Plus,
  Terminal,
} from "lucide-react";
import type { Act } from "./Features";
import { span, type ServerView } from "./format";
import type { DesktopCheck, Health } from "./health";
import type { AppState, Snapshot } from "./types";

const prefs = [
  [
    "show_overlay",
    "屏幕四周显示橙色边框和浮动提示",
    "Agent 操作期间显示正在做什么，浮条上有“停止”按钮。",
  ],
  [
    "yield_input",
    "我动鼠标或键盘时，Agent 让出 30 秒",
    "检测到本地输入后暂停新的桌面操作；已发出的点击不会撤回。需要“输入监控”权限。",
  ],
  ["keep_awake", "防止 Mac 自动休眠", "休眠会中断连接和桌面操作。"],
] as const;

function Mark({ ok }: { ok: boolean | null }) {
  return (
    <span className={`v4-mark-dot ${ok ? "ok" : ok === false ? "warn" : ""}`}>
      {ok ? <Check size={11} /> : ok === false ? "!" : "…"}
    </span>
  );
}

function ServerRow({
  server,
  app,
  busy,
  onPair,
  act,
  onSettings,
}: {
  server: ServerView;
  app: AppState | null;
  busy: boolean;
  onPair: (add?: boolean) => void;
  act: Act;
  onSettings: () => void;
}) {
  const [reveal, setReveal] = useState(false);
  const menu = useRef<HTMLDetailsElement>(null);
  const c = server.connection;
  const connected = c?.state === "connected";
  const close = () => menu.current?.removeAttribute("open");
  const detail = connected
    ? [
        `已连接 ${span(Date.now() - (c?.since || Date.now()))}`,
        c?.rtt_ms != null ? `${c.rtt_ms} ms` : "",
      ]
        .filter(Boolean)
        .join(" · ")
    : app?.worker_starting
      ? "正在启动执行器"
      : app?.worker_running
        ? c?.error || "正在重连"
        : "未连接";
  return (
    <div className="v4-server">
      <span className={`dot ${connected ? "succeeded" : "cancelled"}`} />
      <div className="grow">
        <b>{server.label}</b>
        <p className={!connected && c?.error ? "v4-err" : ""}>{detail}</p>
        {reveal && (
          <p className="mono v4-faint" aria-label="服务器地址">
            {server.address}
            {server.id === "primary" && app?.settings.certificate_fingerprint
              ? ` · 证书 ${app.settings.certificate_fingerprint}`
              : ""}
          </p>
        )}
      </div>
      <details className="v4-menu" ref={menu}>
        <summary aria-label={`${server.label} 的更多操作`}>
          <Ellipsis size={15} />
        </summary>
        <div role="menu">
          <button
            role="menuitem"
            onClick={() => {
              setReveal((v) => !v);
              close();
            }}
          >
            {reveal ? "隐藏地址与证书" : "显示地址与证书"}
          </button>
          <button
            role="menuitem"
            disabled={busy}
            onClick={() => {
              close();
              onPair(false);
            }}
          >
            重新配对
          </button>
          {server.id !== "primary" && (
            <button
              role="menuitem"
              disabled={busy || app?.worker_running}
              title={app?.worker_running ? "请先断开连接" : undefined}
              onClick={() => {
                close();
                act(
                  "remove_connection",
                  { id: server.id },
                  "连接已移除，历史数据仍保留",
                );
              }}
            >
              移除
            </button>
          )}
          <button
            role="menuitem"
            onClick={() => {
              close();
              onSettings();
            }}
          >
            连接详情与手动配置
          </button>
        </div>
      </details>
    </div>
  );
}

export function ThisMac({
  snapshot,
  app,
  available,
  act,
  busy,
  health,
  servers,
  onPair,
  onSettings,
  desktopToggle,
  desktopPending,
  technical,
}: {
  snapshot: Snapshot | null;
  app: AppState | null;
  available: boolean;
  act: Act;
  busy: boolean;
  health: Health;
  servers: ServerView[];
  onPair: (add?: boolean) => void;
  onSettings: () => void;
  desktopToggle: () => void;
  desktopPending: boolean;
  /** Backend sessions, tools, Macrun's own permissions and recent operations. */
  technical: React.ReactNode;
}) {
  const details = useRef<HTMLDetailsElement>(null);
  const openTechnical = () => {
    if (!details.current) return;
    details.current.open = true;
    details.current.scrollIntoView?.({ behavior: "smooth", block: "start" });
  };
  const connectionBusy = busy || !!app?.worker_starting;
  const missing = health.desktop.filter(
    (c) => c.ok === false && c.key !== "enabled",
  );
  const fix = (check: DesktopCheck) =>
    check.key === "backend" ? (
      <button onClick={openTechnical}>配置后端</button>
    ) : check.key === "verified" ? (
      <button onClick={openTechnical}>实拍核对</button>
    ) : null;
  const commands = health.commands
    ? {
        title: "命令与同步：可以使用",
        detail: `${health.online}/${health.servers} 台服务器在线 · 执行器 v${snapshot?.version} 运行中`,
        action: null,
      }
    : app?.worker_starting
      ? {
          title: "命令与同步：正在启动",
          detail: "如 macOS 请求访问钥匙串，请在系统窗口中允许。",
          action: null,
        }
      : !app?.settings.server
        ? {
            title: "命令与同步：尚未配对",
            detail: "在服务器上生成邀请，再粘贴到这里。",
            action: (
              <button className="primary" onClick={() => onPair()}>
                配对服务器
              </button>
            ),
          }
        : !app.worker_running
          ? {
              title: "命令与同步：执行器未运行",
              detail: "启动后连接所有已保存的服务器。",
              action: (
                <button
                  className="primary"
                  disabled={connectionBusy || app.legacy_running}
                  onClick={() => act("start_worker", {}, "已请求启动执行器")}
                >
                  启动并连接
                </button>
              ),
            }
          : {
              title: "命令与同步：正在连接",
              detail: snapshot?.connection.error || "正在连接服务器…",
              action: null,
            };
  const desktop = !health.desktopEnabled
    ? {
        title: "桌面：已关闭",
        detail: "命令和文件不受影响；Agent 的桌面请求会被拒绝。",
      }
    : missing.length
      ? {
          title: `桌面：还差 ${missing.length} 步`,
          detail: missing[0].detail,
        }
      : { title: "桌面：可以使用", detail: "后端已实测可用，图形会话正常。" };
  return (
    <div className="v4-mac">
      <div className="v4-ready">
        <div className={health.commands ? "ok" : "warn"}>
          <span className={`v4-ico ${health.commands ? "ok" : "warn"}`}>
            <Terminal size={15} />
          </span>
          <div className="grow">
            <b>{commands.title}</b>
            <p>{commands.detail}</p>
          </div>
          {commands.action}
        </div>
        <div
          className={
            !health.desktopEnabled ? "" : missing.length ? "warn" : "ok"
          }
        >
          <span
            className={`v4-ico ${!health.desktopEnabled ? "" : missing.length ? "warn" : "ok"}`}
          >
            <MousePointer2 size={15} />
          </span>
          <div className="grow">
            <b>{desktop.title}</b>
            <p>{desktop.detail}</p>
          </div>
          {health.desktopEnabled && missing[0] && fix(missing[0])}
        </div>
      </div>
      <div className="v4-cols even">
        <div>
          <div className="v4-sh">
            <h2>服务器</h2>
            <span className="grow" />
            <button
              disabled={connectionBusy || servers.length >= 16}
              onClick={() => onPair(true)}
            >
              <Plus size={13} />
              添加服务器
            </button>
          </div>
          <div className="v4-group" aria-label="服务器列表">
            {servers.map((s) => (
              <ServerRow
                key={s.id}
                server={s}
                app={app}
                busy={connectionBusy}
                onPair={onPair}
                act={act}
                onSettings={onSettings}
              />
            ))}
            {!servers.length && (
              <div className="v4-server">
                <span className="dot cancelled" />
                <div className="grow">
                  <b>尚未配对服务器</b>
                  <p>配对后，服务器上的 Agent 才能在这台 Mac 上执行任务。</p>
                </div>
                <button className="primary" onClick={() => onPair()}>
                  配对服务器
                </button>
              </div>
            )}
          </div>
          <div className="v4-sh">
            <h2>Agent 操作桌面时</h2>
          </div>
          <div className="v4-group">
            {prefs.map(([key, text, detail]) => (
              <label className="v4-perm" key={key}>
                <div className="grow">
                  <b>{text}</b>
                  <p>{detail}</p>
                </div>
                <input
                  className="switch"
                  type="checkbox"
                  aria-label={text}
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
          </div>
        </div>
        <div>
          <div className="v4-sh">
            <h2>桌面控制检查</h2>
            <small>按顺序完成即可</small>
          </div>
          <div className="v4-group" aria-label="桌面控制检查">
            {health.desktop.map((c) => (
              <div className="v4-check" key={c.key}>
                <Mark ok={c.key === "enabled" ? c.ok : c.ok} />
                <div className="grow">
                  <b>{c.title}</b>
                  <p>{c.detail}</p>
                </div>
                {c.key === "enabled" ? (
                  <input
                    className="switch"
                    type="checkbox"
                    aria-label="允许 Agent 操作桌面"
                    checked={!!snapshot?.policy.desktop_enabled}
                    disabled={!available || desktopPending}
                    aria-busy={desktopPending}
                    onChange={desktopToggle}
                  />
                ) : (
                  c.ok === false && fix(c)
                )}
              </div>
            ))}
            {!available && (
              <p className="v4-note pad">
                <CircleAlert size={13} /> 执行器未连接，以上状态可能已过期。
              </p>
            )}
          </div>
          <details className="v4-technical" ref={details}>
            <summary>
              <ChevronRight size={14} className="v4-disclosure" />
              技术详情：后端进程与工具、实拍核对、Macrun 自身权限、最近桌面操作
            </summary>
            <div className="v4-technical-body">{technical}</div>
          </details>
        </div>
      </div>
    </div>
  );
}
