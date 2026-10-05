import { useEffect, useRef, useState } from "react";
import { CircleAlert } from "lucide-react";
import { PruneButton, type Act } from "./Features";
import type { AppState, Settings, Snapshot } from "./types";
const emptySettings: Settings = {
  server: "",
  cert: "",
  token_file: "",
  backend_config: "",
};

export function SettingsPage({
  app,
  snapshot,
  busy,
  available,
  act,
  refresh,
  setError,
  onPair,
  manualOpen = false,
  actionError,
}: {
  app: AppState | null;
  snapshot: Snapshot | null;
  busy: boolean;
  available: boolean;
  act: Act;
  refresh: () => Promise<void>;
  setError: (s: string) => void;
  onPair: (add?: boolean) => void;
  manualOpen?: boolean;
  actionError?: string;
}) {
  const [settings, setSettings] = useState(app?.settings || emptySettings),
    [dirty, setDirty] = useState(false),
    [manual, setManual] = useState(manualOpen),
    [savingSettings, setSavingSettings] = useState(false),
    [checks, setChecks] = useState<any>(null),
    [migrationOpen, setMigrationOpen] = useState(false);
  const migrationTrigger = useRef<HTMLButtonElement>(null);
  const connectionAction = useRef<HTMLButtonElement>(null);
  const connectionHeading = useRef<HTMLHeadingElement>(null);
  const connection = useRef<HTMLElement>(null);
  const savePending = useRef(false);
  const editVersion = useRef(0);
  const connectionIdentity = useRef("");
  connectionIdentity.current = JSON.stringify([
    app?.settings.server,
    app?.settings.cert,
    app?.settings.certificate_fingerprint,
    app?.settings.keychain_account,
    app?.settings.token_file,
    available,
  ]);
  const connectionBusy = busy || savingSettings || !!app?.worker_starting;
  useEffect(
    () => setChecks(null),
    [
      app?.settings.server,
      app?.settings.cert,
      app?.settings.certificate_fingerprint,
      app?.settings.keychain_account,
      app?.settings.token_file,
      available,
    ],
  );
  useEffect(() => {
    if (app && !dirty) setSettings(app.settings);
  }, [app?.settings, dirty]);
  useEffect(() => {
    if (manualOpen) {
      setManual(true);
      connection.current?.scrollIntoView?.({ block: "start" });
    }
  }, [manualOpen]);
  const update = (k: keyof Settings, v: string) => {
    editVersion.current += 1;
    setDirty(true);
    setSettings((s) => ({
      ...s,
      [k]: v,
      ...(k === "token_file" ? { keychain_account: "" } : {}),
    }));
  };
  const pref = (key: keyof AppState["preferences"], enabled: boolean) =>
    app &&
    act("save_preferences", {
      preferences: { ...app.preferences, [key]: enabled },
    });
  const connected = available && snapshot?.connection.state === "connected";
  return (
    <div className="settings-page">
      <header>
        <h1>设置</h1>
        <nav className="settings-subnav" aria-label="设置分区">
          {[
            ["conn", "连接"],
            ["general", "通用"],
            ["records", "记录"],
            ["diag", "诊断"],
          ].map(([id, label]) => (
            <a
              key={id}
              href={`#${id}`}
              onClick={(e) => {
                e.preventDefault();
                document
                  .getElementById(id)
                  ?.scrollIntoView({ behavior: "smooth", block: "start" });
              }}
            >
              {label}
            </a>
          ))}
        </nav>
      </header>
      {(app?.legacy_running || app?.legacy_detected) && (
        <section
          className="legacy-alert legacy-migration-banner"
          aria-label="旧版 Macrun 迁移"
        >
          <CircleAlert size={16} />
          <div className="legacy-migration-copy">
            <b>检测到旧版 Macrun</b>
            <p>
              已有配置，无需重新配对。迁移后保留任务记录和同步状态，由桌面应用统一管理。
            </p>
            <small>
              {app.legacy_running
                ? "旧执行器正在运行；迁移前请确认任务已经结束。"
                : "已找到旧版配置，旧执行器当前未运行。"}
            </small>
          </div>
          <button
            ref={migrationTrigger}
            className="primary"
            disabled={busy}
            onClick={() => {
              setError("");
              setMigrationOpen(true);
            }}
          >
            迁移
          </button>
        </section>
      )}
      <section className="settings-section" id="conn" ref={connection}>
        <h2 ref={connectionHeading} tabIndex={-1}>
          连接
        </h2>
        <div className="card">
          <div className="feature-line">
            <span className="setting-key">服务器</span>
            <span className="grow mono wrap">
              {app?.settings.server ||
                snapshot?.connection.server ||
                "尚未配对"}
            </span>
            <span className={`tag ${connected ? "succeeded" : ""}`}>
              {connected
                ? "已连接 · QUIC"
                : app?.worker_starting
                  ? "正在启动"
                  : app?.worker_running
                    ? "连接中"
                    : "未连接"}
            </span>
          </div>
          <div className="feature-line">
            <span className="setting-key">服务器证书</span>
            <span className="grow mono wrap">
              {app?.settings.certificate_fingerprint
                ? app.settings.certificate_fingerprint
                : app?.settings.cert || "尚未配置"}
            </span>
            <small>
              {app?.settings.keychain_account ? "配对时已固定" : "证书文件"}
            </small>
          </div>
          <div className="feature-line">
            <span className="setting-key">访问令牌</span>
            <span className="grow">
              {app?.settings.keychain_account
                ? "保存在 macOS 钥匙串"
                : app?.settings.token_file
                  ? "使用本机令牌文件"
                  : "尚未配置"}
            </span>
            <small>凭据不在页面显示</small>
          </div>
          <div className="feature-line">
            <span className="setting-key">本机平台</span>
            <span className="grow">
              {app?.platform || "仅在桌面应用中可用"}
            </span>
            <button
              className="ghost"
              onClick={() => setManual((v) => !v)}
              aria-expanded={manual}
            >
              手动连接配置
            </button>
          </div>
          <div className="feature-line feature-shaded">
            <small className="grow">
              {snapshot?.active_count
                ? "有任务运行时，请先全部停止，等待任务结束后再断开。"
                : "更换服务器或令牌失效时重新配对。现有任务记录会保留。"}
            </small>
            <button
              disabled={connectionBusy || app?.worker_running}
              onClick={() => onPair()}
            >
              {app?.settings.server ? "重新配对" : "配对服务器"}
            </button>
            {app?.worker_running ? (
              <button
                ref={connectionAction}
                className="danger"
                disabled={busy || !available || !!snapshot?.active_count}
                onClick={() => act("stop_worker", {}, "已请求断开连接")}
              >
                断开连接
              </button>
            ) : (
              <button
                ref={connectionAction}
                disabled={
                  connectionBusy ||
                  dirty ||
                  !app?.settings.server ||
                  app?.legacy_running
                }
                onClick={() => act("start_worker", {}, "已请求启动执行器")}
              >
                {app?.worker_starting ? "正在启动…" : "启动并连接"}
              </button>
            )}
          </div>
        </div>
        <div className="server-connections" aria-label="服务器连接列表">
          <div className="row between">
            <h3>服务器连接</h3>
            <button
              disabled={
                connectionBusy || (app?.settings.connections?.length || 0) >= 15
              }
              onClick={() => onPair(true)}
            >
              添加服务器
            </button>
          </div>
          <p className="muted">
            保存的服务器会同时连接。点击添加后会引导你安全断开；已有连接和任务记录会保留。
          </p>
          {[
            ...(app?.settings.server
              ? [
                  {
                    id: "primary",
                    name: app.settings.server,
                    server: app.settings.server,
                  },
                ]
              : []),
            ...(app?.settings.connections || []),
          ].map((c) => {
            const live =
              available && app?.worker_running
                ? snapshot?.connections?.find((v) => v.id === c.id)
                    ?.connection ||
                  (c.id === "primary" ? snapshot?.connection : undefined)
                : undefined;
            return (
              <div className="feature-line" key={c.id}>
                <span
                  className={`dot ${live?.state === "connected" ? "succeeded" : "cancelled"}`}
                />
                <div className="grow">
                  <b>{c.name}</b>
                  <small className="mono wrap">
                    {c.id === "primary" ? "主连接 · " : ""}
                    {c.server}
                  </small>
                  {live?.error && (
                    <small className="error-text">{live.error}</small>
                  )}
                </div>
                <span
                  className={`tag ${live?.state === "connected" ? "succeeded" : ""}`}
                >
                  {live?.state === "connected"
                    ? "已连接"
                    : app?.worker_running
                      ? "连接中"
                      : "未连接"}
                </span>
                {c.id !== "primary" && (
                  <button
                    className="ghost"
                    disabled={connectionBusy || app?.worker_running}
                    onClick={() =>
                      act(
                        "remove_connection",
                        { id: c.id },
                        "连接已移除，历史数据仍保留",
                      )
                    }
                  >
                    移除
                  </button>
                )}
              </div>
            );
          })}
        </div>
        {app?.worker_starting && (
          <p className="muted" role="status">
            正在启动执行器。如果 macOS
            请求访问钥匙串，请完成系统授权后等待连接；无需重复点击。
          </p>
        )}
        {manual && (
          <form
            className="card connection-form"
            onSubmit={async (e) => {
              e.preventDefault();
              if (connectionBusy || app?.worker_running || savePending.current)
                return;
              savePending.current = true;
              setSavingSettings(true);
              const revision = editVersion.current;
              try {
                const saved = await act(
                  "save_settings",
                  { settings },
                  "连接配置已保存",
                );
                if (
                  saved !== undefined &&
                  saved !== false &&
                  editVersion.current === revision
                )
                  setDirty(false);
              } catch (error) {
                setError(String(error));
              } finally {
                savePending.current = false;
                setSavingSettings(false);
              }
            }}
          >
            <div className="row between">
              <h3>手动连接（兼容已有部署）</h3>
              <button
                type="button"
                className="ghost"
                onClick={() => setManual(false)}
              >
                收起
              </button>
            </div>
            <p className="muted">
              已配对连接使用系统钥匙串；手动连接支持现有证书与令牌文件。修改前请断开连接。
            </p>
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
            ).map(([k, label, placeholder]) => (
              <label className="field" key={k}>
                <span>{label}</span>
                <input
                  required={
                    k !== "backend_config" &&
                    !(k === "token_file" && settings.keychain_account)
                  }
                  value={settings[k]}
                  placeholder={placeholder}
                  onChange={(e) => update(k, e.target.value)}
                  disabled={connectionBusy || app?.worker_running}
                />
              </label>
            ))}
            <div className="actions">
              <small className="grow">
                {dirty ? "更改尚未保存。" : "凭据内容不会显示在页面中。"}
              </small>
              <button
                type="submit"
                className="primary"
                disabled={connectionBusy || app?.worker_running || !dirty}
              >
                保存配置
              </button>
            </div>
          </form>
        )}
      </section>
      <section className="settings-section" id="general">
        <h2>通用</h2>

        <div className="card">
          <label className="feature-line">
            <div className="grow">
              <b>登录时启动</b>
              <small>在图形登录后启动；注销或重启到登录界面前不会运行。</small>
            </div>
            <input
              type="checkbox"
              className="switch"
              disabled={busy || !app}
              checked={app?.autostart || false}
              onChange={(e) => act("autostart", { enabled: e.target.checked })}
            />
          </label>
          <div className="feature-line">
            <div className="grow">
              <b>关闭窗口后继续在菜单栏运行</b>
              <small>
                只有“退出 Macrun”才会停止执行器；有任务在跑时会先询问。
              </small>
            </div>
            <input
              type="checkbox"
              className="switch fixed-switch"
              checked
              disabled
              aria-label="关闭窗口后保持运行，始终开启"
            />
          </div>
          <label className="feature-line">
            <div className="grow">
              <b>需要确认、失败或结果未知时发送通知</b>
              <small>成功不打扰；待确认、失败、超时、未知各发一次。</small>
            </div>
            <input
              type="checkbox"
              className="switch"
              checked={app?.preferences.notifications || false}
              disabled={busy || !app}
              onChange={(e) => pref("notifications", e.target.checked)}
            />
          </label>
          <label className="feature-line">
            <div className="grow">
              <b>应用启动时自动连接</b>
              <small>使用已保存的连接配置启动执行器。</small>
            </div>
            <input
              type="checkbox"
              className="switch"
              checked={app?.preferences.auto_connect || false}
              disabled={busy || !app}
              onChange={(e) => pref("auto_connect", e.target.checked)}
            />
          </label>
        </div>
      </section>
      <section className="settings-section" id="records">
        <h2>记录</h2>
        <div className="card">
          <div className="feature-line">
            <div className="grow">
              <b>任务记录保留</b>
              <small>
                超过期限的输出和截图会被清理，去重编号另保留 90 天。
              </small>
            </div>
            <div className="feature-seg" role="group" aria-label="保留时长">
              {[7, 30, 90].map((n) => (
                <button
                  key={n}
                  className={snapshot?.safety.retention_days === n ? "on" : ""}
                  aria-pressed={snapshot?.safety.retention_days === n}
                  disabled={busy || !available || !snapshot?.safety}
                  onClick={() =>
                    act(
                      "control",
                      {
                        action: "safety",
                        args: { ...snapshot?.safety, retention_days: n },
                      },
                      "记录保留时长已保存",
                    )
                  }
                >
                  {n} 天
                </button>
              ))}
            </div>
          </div>
          <div className="feature-line feature-shaded">
            <small className="grow">
              启动时和每小时自动清理过期记录；也可以现在清理。去重编号会保留，过期任务不会再次执行。
            </small>
            <PruneButton act={act} disabled={busy || !available} />
          </div>
        </div>
      </section>
      <section className="settings-section" id="diag">
        <h2>诊断</h2>
        <div className="card">
          <div className="feature-line">
            <span className="setting-key">版本</span>
            <span className="grow mono wrap">
              {snapshot
                ? `macrun ${snapshot.version} · 协议 ${snapshot.protocol}`
                : "执行器未启动"}
              {app?.platform && ` · ${app.platform}`}
            </span>
          </div>
          <div className="feature-line">
            <span className="setting-key">数据目录</span>
            <span className="grow mono wrap">
              {app?.data_dir || "仅在桌面应用中可用"}
            </span>
            {snapshot && <small>{snapshot.total_tasks} 条任务</small>}
          </div>
          <div className="feature-line feature-shaded">
            <small className="grow">
              诊断包包含版本、连接状态、运行任务数量和权限状态，不包含日志、命令输出、文件内容、截图和令牌。
            </small>
            <button
              disabled={busy || !app}
              onClick={() => act("open_log", { taskId: null })}
            >
              查看日志
            </button>
            <button
              disabled={busy || !available}
              onClick={async () => {
                setChecks(null);
                const identity = connectionIdentity.current;
                const result = await act("connection_check");
                if (result && identity === connectionIdentity.current)
                  setChecks(result);
              }}
            >
              测试连通性
            </button>
            <button
              className="primary"
              disabled={busy || !app}
              onClick={() =>
                act("diagnostics", {}, "诊断包已导出并在访达中定位")
              }
            >
              导出诊断包
            </button>
          </div>
          {checks && (
            <div className="feature-line diagnostic-checks" role="status">
              {checks.checks?.map((c: any) => (
                <span
                  className={`tag ${c.ok ? "succeeded" : "failed"}`}
                  key={c.name}
                >
                  {c.name} · {c.ok ? "通过" : "未通过"}
                </span>
              ))}
              {checks.error && <p className="error-text">{checks.error}</p>}
            </div>
          )}
        </div>
      </section>
      {migrationOpen && (
        <MigrationDialog
          act={act}
          actionError={actionError}
          onClose={() => setMigrationOpen(false)}
          returnFocus={() =>
            [migrationTrigger.current, connectionAction.current].find(
              (target) => target?.isConnected && !target.disabled,
            ) || connectionHeading.current
          }
        />
      )}
    </div>
  );
}

function MigrationDialog({
  act,
  actionError,
  onClose,
  returnFocus,
}: {
  act: Act;
  actionError?: string;
  onClose: () => void;
  returnFocus: () => HTMLElement | null;
}) {
  const dialog = useRef<HTMLDivElement>(null);
  const pendingRef = useRef(false);
  const closeRef = useRef(onClose);
  const returnFocusRef = useRef(returnFocus);
  closeRef.current = onClose;
  returnFocusRef.current = returnFocus;
  const [pending, setPending] = useState(false),
    [failure, setFailure] = useState(""),
    [result, setResult] = useState<any>(null),
    [started, setStarted] = useState(false);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const element = dialog.current;
    element?.querySelector<HTMLButtonElement>("button")?.focus();
    const isTopDialog = () => {
      const dialogs = document.querySelectorAll('[role="dialog"]');
      return dialogs.item(dialogs.length - 1) === element;
    };
    const handleKey = (event: KeyboardEvent) => {
      if (!isTopDialog()) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        if (!pendingRef.current) closeRef.current();
      }
      if (event.key !== "Tab") return;
      const controls = Array.from(
        element?.querySelectorAll<HTMLElement>(
          "button:not(:disabled), a[href], input:not(:disabled), [tabindex='0']",
        ) || [],
      );
      const first = controls[0],
        last = controls[controls.length - 1];
      if (!first) {
        event.preventDefault();
        element?.focus();
        return;
      }
      if (
        event.shiftKey &&
        (document.activeElement === first ||
          document.activeElement === element ||
          !element?.contains(document.activeElement))
      ) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (document.activeElement === last ||
          document.activeElement === element ||
          !element?.contains(document.activeElement))
      ) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKey, true);
    return () => {
      document.removeEventListener("keydown", handleKey, true);
      const remainingDialogs = document.querySelectorAll('[role="dialog"]');
      if (remainingDialogs.length && !isTopDialog()) return;
      const target =
        previous?.isConnected && !previous.matches(":disabled")
          ? previous
          : returnFocusRef.current();
      target?.focus();
    };
  }, []);
  const perform = async (operation: "migrate" | "start") => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(true);
    setFailure("");
    try {
      const response = await act(
        operation === "migrate" ? "migrate_legacy" : "start_worker",
        {},
        operation === "migrate"
          ? "旧服务已迁移，可以启动桌面连接"
          : "已请求启动执行器",
      );
      if (response === undefined || response === false) {
        setFailure(
          operation === "migrate"
            ? "迁移未完成，请检查错误信息后重试。"
            : "迁移已完成，但执行器未能启动。请检查错误信息后重试。",
        );
      } else if (operation === "migrate") setResult(response);
      else setStarted(true);
    } catch (error) {
      setFailure(String(error));
    } finally {
      pendingRef.current = false;
      setPending(false);
      const dialogs = document.querySelectorAll('[role="dialog"]');
      if (dialogs.item(dialogs.length - 1) === dialog.current)
        dialog.current?.focus();
    }
  };
  return (
    <div
      className="scrim migration-scrim"
      onClick={(event) => {
        if (event.target === event.currentTarget && !pendingRef.current)
          onClose();
      }}
    >
      <div
        className="dialog migration-dialog"
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="migration-title"
        aria-describedby="migration-description"
        aria-busy={pending}
        tabIndex={-1}
      >
        <h2 id="migration-title">{result ? "迁移完成" : "迁移旧执行器"}</h2>
        <div id="migration-description">
          {result ? (
            <>
              <p>
                旧服务配置已备份并停用，连接配置已交给 Macrun
                Desktop。任务记录和同步状态已复制到桌面版，原目录的旧副本仍保留。
              </p>
              {typeof result.backup === "string" && (
                <p className="muted wrap">
                  备份位置：<span className="mono">{result.backup}</span>
                </p>
              )}
              <p>
                {started
                  ? "已请求启动执行器，可在连接区域查看连接状态。"
                  : "下一步启动执行器，连接已保存的服务器。"}
              </p>
            </>
          ) : (
            <>
              <p>
                迁移会备份原 LaunchAgent
                配置、停止并停用旧服务，再把连接配置交给 Macrun Desktop。
              </p>
              <p>
                任务记录和同步状态会复制到桌面版，原目录的旧副本仍保留。请确认旧服务没有正在运行的任务；执行器也会在迁移前检查。
              </p>
            </>
          )}
        </div>
        {failure && (
          <div className="migration-error" role="alert">
            <b>{failure}</b>
            {actionError && actionError !== failure && <p>{actionError}</p>}
          </div>
        )}
        {pending && (
          <p className="muted" role="status">
            {result
              ? "正在启动执行器，请稍候…"
              : "正在备份和迁移旧服务，请稍候…"}
          </p>
        )}
        <div className="actions">
          {result ? (
            <>
              {!started && (
                <button disabled={pending} onClick={() => perform("start")}>
                  {pending ? "正在启动…" : "启动并连接"}
                </button>
              )}
              <button className="primary" disabled={pending} onClick={onClose}>
                完成
              </button>
            </>
          ) : (
            <>
              <button disabled={pending} onClick={onClose}>
                取消
              </button>
              <button
                className="primary"
                disabled={pending}
                onClick={() => perform("migrate")}
              >
                {pending ? "正在迁移…" : failure ? "重试迁移" : "确认迁移"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
