import { useEffect, useRef, useState } from "react";
import { CircleAlert } from "lucide-react";
import { SafetyPanel, type Act } from "./Features";
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
}: {
  app: AppState | null;
  snapshot: Snapshot | null;
  busy: boolean;
  available: boolean;
  act: Act;
  refresh: () => Promise<void>;
  setError: (s: string) => void;
  onPair: () => void;
  manualOpen?: boolean;
}) {
  const [settings, setSettings] = useState(app?.settings || emptySettings),
    [dirty, setDirty] = useState(false),
    [manual, setManual] = useState(manualOpen),
    [checks, setChecks] = useState<any>(null);
  const connection = useRef<HTMLElement>(null);
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
    setDirty(true);
    setSettings((s) => ({ ...s, [k]: v }));
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
        <h1>设置与安全</h1>
        <nav className="settings-subnav" aria-label="设置分区">
          {[
            ["safety", "安全边界"],
            ["conn", "连接"],
            ["general", "通用"],
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
      <section className="settings-section" id="safety">
        <div>
          <h2>安全边界</h2>
          <p className="muted">
            远程 Agent 拥有和你一样的用户权限。这些规则在执行器内生效，Shell
            仍可使用本机用户权限。
          </p>
        </div>
        <SafetyPanel
          snapshot={snapshot}
          act={act}
          disabled={busy || !available}
          includeRetention={false}
        />
      </section>
      <section className="settings-section" id="conn" ref={connection}>
        <h2>连接</h2>
        <div className="card">
          <div className="feature-line">
            <span className="setting-key">服务器</span>
            <span className="grow mono wrap">
              {snapshot?.connection.server ||
                app?.settings.server ||
                "尚未配对"}
            </span>
            <span className={`tag ${connected ? "succeeded" : ""}`}>
              {connected
                ? "已连接 · QUIC"
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
            <button disabled={busy || app?.worker_running} onClick={onPair}>
              {app?.settings.server ? "重新配对" : "配对服务器"}
            </button>
            {app?.worker_running ? (
              <button
                className="danger"
                disabled={busy || !available || !!snapshot?.active_count}
                onClick={() => act("stop_worker", {}, "已请求断开连接")}
              >
                断开连接
              </button>
            ) : (
              <button
                disabled={
                  busy || dirty || !app?.settings.server || app?.legacy_running
                }
                onClick={() => act("start_worker", {}, "已请求启动执行器")}
              >
                启动并连接
              </button>
            )}
          </div>
        </div>
        {manual && (
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
                  disabled={app?.worker_running}
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
                disabled={busy || app?.worker_running || !dirty}
              >
                保存配置
              </button>
            </div>
          </form>
        )}
      </section>
      <section className="settings-section" id="general">
        <h2>通用</h2>
        {app?.legacy_running && (
          <div className="legacy-alert">
            <CircleAlert size={16} />
            <span>
              检测到旧的 LaunchAgent{" "}
              <span className="mono">dev.macrun.worker</span> 仍在运行。迁移后由
              Macrun Desktop 统一管理，原服务配置会备份，旧数据会保留。
            </span>
            <button
              className="primary"
              disabled={busy}
              onClick={() => {
                if (
                  window.confirm(
                    "确认旧服务没有运行中的任务？迁移将备份并停止旧 LaunchAgent，旧数据会保留。",
                  )
                )
                  act("migrate_legacy", {}, "迁移完成，可以启动桌面连接");
              }}
            >
              迁移
            </button>
          </div>
        )}
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
              onChange={async (e) => {
                await act("autostart", { enabled: e.target.checked });
                refresh().catch((e) => setError(String(e)));
              }}
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
              <b>任务失败或结果未知时发送通知</b>
              <small>成功不打扰；失败、超时、未知各发一次。</small>
            </div>
            <input
              type="checkbox"
              className="switch"
              checked={app?.preferences.notifications || false}
              disabled={busy || !app}
              onChange={(e) => pref("notifications", e.target.checked)}
            />
          </label>
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
              诊断包包含版本、连通状态和脱敏日志，不包含命令输出、文件内容、截图和令牌。
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
                const result = await act("connection_check");
                if (result) setChecks(result);
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
    </div>
  );
}
