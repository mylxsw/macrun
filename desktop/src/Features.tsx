import { useEffect, useState } from "react";
import {
  Check,
  CircleAlert,
  Folder,
  Monitor,
  Plus,
  Terminal,
  X,
} from "lucide-react";
import type { Snapshot, Task, AppState } from "./types";
import { statuses } from "./model.mjs";
import "./features.css";
export type Act = (
  c: string,
  a?: Record<string, unknown>,
  s?: string,
) => Promise<any>;

export function Approvals({
  tasks,
  act,
  disabled,
}: {
  tasks: Task[];
  act: Act;
  disabled: boolean;
}) {
  return (
    <>
      {tasks
        .filter((t) => t.status === "awaiting_approval")
        .map((t) => (
          <section className="card approval" key={t.task_id}>
            <strong>等待你确认 · 60 秒内处理</strong>
            <pre className="command">
              {t.arguments.command || t.arguments.tool}
            </pre>
            <small>{t.arguments.cwd}</small>
            <div className="actions">
              <button
                disabled={disabled}
                className="primary"
                onClick={() =>
                  act("control", {
                    action: "approve",
                    args: { task_id: t.task_id, allow: true },
                  })
                }
              >
                允许一次
              </button>
              <button
                disabled={disabled}
                onClick={() =>
                  act("control", {
                    action: "approve",
                    args: { task_id: t.task_id, allow: false },
                  })
                }
              >
                拒绝
              </button>
            </div>
          </section>
        ))}
    </>
  );
}

export function Pairing({
  act,
  running,
  onComplete,
  onManual,
  onClose,
}: {
  act: Act;
  running: boolean;
  onComplete?: (route?: "desktop" | "main") => void;
  onManual?: () => void;
  onClose?: () => void;
}) {
  const [uri, setUri] = useState(""),
    [result, setResult] = useState<any>(null),
    [step, setStep] = useState(1),
    [pending, setPending] = useState(false),
    [error, setError] = useState("");
  const perform = async (fn: () => Promise<void>) => {
    setPending(true);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  };
  const checkConnection = async () => {
    const checks = await act("connection_check");
    if (checks) setResult((v: any) => ({ ...v, ...checks }));
    else setError("暂时无法完成检查，请稍后重试。");
  };
  const finish = async (route: "desktop" | "main") => {
    if (route === "main") {
      const saved = await act(
        "control",
        { action: "desktop", args: { enabled: false } },
        "仅启用命令与文件能力",
      );
      if (saved === undefined) return;
    }
    if (onComplete) onComplete(route);
    else await act("open_main", { route });
  };
  const passed =
    result?.checks?.length > 0 &&
    result.checks.every((c: any) => c.ok === true);
  return (
    <div className="pairing-screen">
      <div className="pairing-drag" data-tauri-drag-region aria-hidden="true" />
      <aside className="pairing-sidebar" aria-label="配对步骤">
        <div className="pairing-brand">
          <span className="logo">
            <Terminal size={17} />
          </span>
          <b>连接到服务器</b>
        </div>
        {["配对码", "检查连接", "桌面控制"].map((name, i) => (
          <div
            key={name}
            className={`pairing-step ${step === i + 1 ? "current" : step > i + 1 ? "done" : ""}`}
            aria-current={step === i + 1 ? "step" : undefined}
          >
            <span>{step > i + 1 ? <Check size={13} /> : i + 1}</span>
            {name}
          </div>
        ))}
        <p className="muted pairing-help">
          还没有服务器？先在 Linux 上按 README 第 1–2 步安装 Macrun 服务端。
        </p>
      </aside>
      <main className="pairing-content">
        {onClose && (
          <button
            className="pairing-close icon-button"
            aria-label="关闭配对"
            onClick={onClose}
            disabled={pending}
          >
            <X size={16} />
          </button>
        )}
        {step === 1 ? (
          <>
            <h1>粘贴配对码</h1>
            <p className="pairing-intro">
              在服务器上运行下面这条命令，它会生成一个 10
              分钟内有效、只能用一次的配对码。
            </p>
            <pre className="term pairing-command">
              {"$ macrun invite --data 服务端数据目录 --server 服务器地址:7443"}
            </pre>
            <label className="pairing-code">
              配对码
              <input
                className="mono"
                autoFocus
                value={uri}
                onChange={(e) => setUri(e.target.value)}
                spellCheck={false}
                placeholder="macrun://pair/…"
                disabled={pending}
              />
            </label>
            <p className="muted">
              配对码包含服务器地址和证书指纹。令牌会通过加密连接下发，并存入钥匙串，不需要再用
              scp 复制文件。
            </p>
            {running && (
              <p className="error-text" role="status">
                请先在设置中断开现有连接，再重新配对。
              </p>
            )}
            {error && (
              <p className="error-text" role="alert">
                {error}
              </p>
            )}
            <div className="pairing-actions">
              {onManual && (
                <button onClick={onManual} disabled={pending}>
                  手动填写地址和证书
                </button>
              )}
              <button
                className="primary"
                disabled={pending || running || !uri.trim()}
                onClick={() =>
                  perform(async () => {
                    const paired = await act("pair", { uri: uri.trim() });
                    if (!paired) {
                      setError("配对未完成，请检查配对码是否有效，然后重试。");
                      return;
                    }
                    setResult(paired);
                    setUri("");
                    setStep(2);
                    const started = await act("start_worker");
                    if (started === undefined) {
                      setError(
                        "配对已保存，但执行器尚未启动。请在设置中重新连接。",
                      );
                      return;
                    }
                    await checkConnection();
                  })
                }
              >
                {pending ? "连接中…" : "连接"}
              </button>
            </div>
          </>
        ) : step === 2 ? (
          <>
            <h1>检查连接</h1>
            <p className="pairing-intro">
              逐项检查，任何一步失败都会说明原因和下一步怎么做。
            </p>
            <div className="card pairing-checks">
              <div className="feature-line">
                <span className="check-icon ok">
                  <Check size={13} />
                </span>
                <span className="grow">令牌已存入钥匙串</span>
                <small>Keychain</small>
              </div>
              {result?.checks?.map((c: any) => (
                <div className="feature-line" key={c.name}>
                  <span className={`check-icon ${c.ok ? "ok" : "warn"}`}>
                    {c.ok ? <Check size={13} /> : <CircleAlert size={13} />}
                  </span>
                  <span className="grow">{c.name}</span>
                  <small>{c.ok ? "已通过" : "尚未通过"}</small>
                </div>
              ))}
              <div className="feature-line">
                <span className="grow">证书指纹已固定</span>
                <small className="mono wrap">{result?.fingerprint}</small>
              </div>
            </div>
            {(result?.error || error) && (
              <p className="error-text" role="alert">
                {result?.error || error}
              </p>
            )}
            <div className="actions pairing-check-actions">
              <button
                disabled={pending}
                onClick={() => perform(checkConnection)}
              >
                重新检查
              </button>
              <button
                disabled={pending || !passed}
                onClick={() =>
                  perform(async () => {
                    const r = await act("control", {
                      action: "self_test",
                      args: {},
                    });
                    if (r) setResult((v: any) => ({ ...v, testId: r.task_id }));
                  })
                }
              >
                试运行一条本机命令
              </button>
              {result?.testId && (
                <button
                  disabled={pending}
                  onClick={() =>
                    perform(async () => {
                      const r = await act("control", {
                        action: "task_detail",
                        args: { task_id: result.testId },
                      });
                      if (r)
                        setResult((v: any) => ({ ...v, testStatus: r.status }));
                    })
                  }
                >
                  查询试运行：
                  {statuses[result.testStatus as keyof typeof statuses] ||
                    "等待结果"}
                </button>
              )}
            </div>
            <p className="muted">
              连接通过后即可运行命令和同步文件。结果未知的命令不会自动重放。
            </p>
            <div className="pairing-actions">
              <button
                disabled={!passed || pending}
                className="primary"
                onClick={() => setStep(3)}
              >
                继续
              </button>
            </div>
          </>
        ) : (
          <>
            <h1>桌面控制（可选）</h1>
            <p className="pairing-intro">
              需要让 Agent 截图、点击应用时再开启。跳过不影响命令和文件。
            </p>
            <div className="card">
              <div className="feature-line">
                <Monitor size={17} />
                <div className="grow">
                  <b>computer-use 工具</b>
                  <small>在桌面控制页添加本机 MCP 后端，并实拍验证。</small>
                </div>
              </div>
              <div className="feature-line">
                <div className="grow">
                  <b>辅助功能</b>
                  <small>后端需要授权后才能点击和输入。</small>
                </div>
                <button
                  onClick={() =>
                    act("open_permission", { kind: "accessibility" })
                  }
                >
                  打开系统设置
                </button>
              </div>
              <div className="feature-line">
                <div className="grow">
                  <b>屏幕录制</b>
                  <small>通过后端实拍验证，不只看权限标记。</small>
                </div>
                <button
                  onClick={() => act("open_permission", { kind: "screen" })}
                >
                  打开系统设置
                </button>
              </div>
            </div>
            {error && (
              <p className="error-text" role="alert">
                {error}
              </p>
            )}
            <div className="pairing-actions">
              <button
                disabled={pending}
                onClick={() => perform(() => finish("main"))}
              >
                稍后再说
              </button>
              <button
                className="primary"
                disabled={pending}
                onClick={() => perform(() => finish("desktop"))}
              >
                完成
              </button>
            </div>
          </>
        )}
      </main>
    </div>
  );
}

const approvalModes = [
  {
    key: "direct",
    label: "直接执行",
    hint: "不做命令确认，适合专门给 Agent 使用的机器。",
  },
  {
    key: "risk",
    label: "风险命令先确认",
    hint: "已知只读命令直接执行，其余命令先请你确认。",
  },
  {
    key: "all",
    label: "每条都确认",
    hint: "每条命令和桌面调用都需要你点“允许”。",
  },
];
export function SafetyPanel({
  snapshot,
  act,
  disabled,
  includeRetention = true,
}: {
  snapshot: Snapshot | null;
  act: Act;
  disabled: boolean;
  includeRetention?: boolean;
}) {
  const [draft, setDraft] = useState<Snapshot["safety"] | null>(
      snapshot?.safety || null,
    ),
    [dirty, setDirty] = useState(false),
    [adding, setAdding] = useState(false),
    [path, setPath] = useState("");
  useEffect(() => {
    if (snapshot?.safety && !dirty) setDraft(snapshot.safety);
  }, [snapshot?.safety, dirty]);
  const update = (k: keyof Snapshot["safety"], v: unknown) => {
    if (draft) {
      setDirty(true);
      setDraft({ ...draft, [k]: v });
    }
  };
  const addPath = () => {
    const root = path.trim();
    if (!draft || !root) return;
    update("roots", [...new Set([...draft.roots, root])]);
    setPath("");
    setAdding(false);
  };
  return (
    <div className="card safety-card">
      <div className="feature-line settings-split">
        <div className="feature-copy">
          <div className="path-policy-title">
            <b>允许的工作目录</b>
            <label className="path-policy-toggle">
              <small>{draft?.restrict_paths ? "已限制" : "未限制"}</small>
              <input
                className="switch"
                type="checkbox"
                aria-label="限制工作目录"
                checked={draft?.restrict_paths || false}
                disabled={disabled || !draft}
                onChange={(e) => update("restrict_paths", e.target.checked)}
              />
            </label>
          </div>
          <small>
            {!draft
              ? "连接执行器后可设置目录限制。"
              : draft.restrict_paths
                ? "命令的工作目录、文件读写和同步目标必须位于这些目录之内。"
                : "目录限制未开启，以下目录不会约束命令、文件读写或同步目标。"}
          </small>
        </div>
        <div className="path-controls">
          {draft?.roots.map((root) => (
            <span className="pathchip mono" key={root}>
              {root}
              <button
                aria-label={`移除 ${root}`}
                className="icon-button"
                disabled={disabled}
                onClick={() =>
                  update(
                    "roots",
                    draft.roots.filter((p) => p !== root),
                  )
                }
              >
                <X size={12} />
              </button>
            </span>
          ))}
          {adding ? (
            <form
              className="path-entry"
              onSubmit={(e) => {
                e.preventDefault();
                addPath();
              }}
            >
              <input
                autoFocus
                aria-label="允许的绝对目录"
                placeholder="/absolute/path"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setAdding(false);
                }}
              />
              <button type="submit" disabled={!path.trim() || disabled}>
                添加
              </button>
              <button type="button" onClick={() => setAdding(false)}>
                取消
              </button>
            </form>
          ) : (
            <button
              className="ghost"
              disabled={disabled || !draft}
              onClick={() => setAdding(true)}
            >
              <Plus size={13} />
              添加目录
            </button>
          )}
        </div>
      </div>
      <div className="feature-line settings-split">
        <div className="feature-copy">
          <b>执行命令前</b>
          <small>
            {approvalModes.find((m) => m.key === draft?.approval)?.hint ||
              "连接执行器后可设置工作目录和命令确认。"}
          </small>
        </div>
        <div className="feature-seg" role="group" aria-label="命令确认方式">
          {approvalModes.map((mode) => (
            <button
              key={mode.key}
              className={draft?.approval === mode.key ? "on" : ""}
              aria-pressed={draft?.approval === mode.key}
              disabled={disabled || !draft}
              onClick={() => update("approval", mode.key)}
            >
              {mode.label}
            </button>
          ))}
        </div>
      </div>
      {draft?.approval === "risk" && (
        <div className="feature-line settings-split feature-shaded">
          <div className="feature-copy">
            <b>风险命令规则</b>
            <small>60 秒未处理自动拒绝。规则不能识别所有脚本行为。</small>
          </div>
          <div className="risk-rules">
            <span className="tag">未知程序或脚本</span>
            <span className="tag">重定向和管道</span>
            <span className="tag">组合命令</span>
            <small>执行器内置规则；需要逐条控制时请选择“每条都确认”。</small>
          </div>
        </div>
      )}
      <label className="feature-line">
        <div className="grow">
          <b>环境变量值不写入记录</b>
          <small>
            任务详情只显示变量名；命令自身输出的秘密仍可能出现在日志。
          </small>
        </div>
        <input
          type="checkbox"
          className="switch fixed-switch"
          checked
          disabled
          aria-label="环境变量保护始终开启"
        />
      </label>
      <details className="feature-disclosure safety-options">
        <summary>记录管理</summary>
        <div className="feature-disclosure-body">
          {includeRetention && (
            <label className="field">
              <span>任务记录保留</span>
              <select
                value={draft?.retention_days || 30}
                disabled={disabled || !draft}
                onChange={(e) =>
                  update("retention_days", Number(e.target.value))
                }
              >
                {[7, 30, 90].map((n) => (
                  <option key={n} value={n}>
                    {n} 天
                  </option>
                ))}
              </select>
            </label>
          )}
          <button
            disabled={disabled}
            onClick={() =>
              act(
                "control",
                { action: "prune", args: {} },
                "过期记录已清理，去重编号已保留",
              )
            }
          >
            清理过期记录
          </button>
        </div>
      </details>
      {dirty && (
        <div className="feature-line feature-shaded">
          <small className="grow">更改尚未保存</small>
          <button
            disabled={disabled}
            onClick={() => {
              setDraft(snapshot?.safety || null);
              setDirty(false);
            }}
          >
            取消更改
          </button>
          <button
            className="primary"
            disabled={disabled}
            onClick={async () => {
              const saved = await act(
                "control",
                {
                  action: "safety",
                  args: {
                    ...draft,
                    retention_days: includeRetention
                      ? draft?.retention_days
                      : snapshot?.safety.retention_days,
                    roots: draft?.roots.filter((r) => r.trim()),
                  },
                },
                "安全设置已保存",
              );
              if (saved !== undefined) setDirty(false);
            }}
          >
            保存安全设置
          </button>
        </div>
      )}
    </div>
  );
}

export function WorkspaceList({
  snapshot,
  act,
}: {
  snapshot: Snapshot | null;
  act: Act;
}) {
  return (
    <section className="workspace-section">
      <div className="row between">
        <h2>工作区</h2>
        <small>由服务器同步过来的目录</small>
      </div>
      <div className="card workspace-list">
        {snapshot?.workspaces?.length ? (
          snapshot.workspaces.map((w) => (
            <div className="feature-line workspace-row" key={w.root}>
              <Folder size={17} />
              <div className="grow">
                <b className="wrap">
                  {w.root.split("/").filter(Boolean).pop() || w.root}
                </b>
                <small className="mono ellipsis" title={w.root}>
                  {w.root}
                </small>
                <small>
                  {statuses[w.status as keyof typeof statuses] || w.status} ·{" "}
                  {new Date(w.time).toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                    hour12: false,
                  })}
                </small>
                {w.error?.message && (
                  <small className="error-text">{w.error.message}</small>
                )}
              </div>
              <details className="workspace-actions">
                <summary aria-label={`打开工作区 ${w.root}`} title="打开工作区">
                  <span className={`dot ${w.status}`} />
                </summary>
                <div>
                  <button
                    onClick={() =>
                      act("open_workspace", { root: w.root, terminal: false })
                    }
                  >
                    在访达中打开
                  </button>
                  <button
                    onClick={() =>
                      act("open_workspace", { root: w.root, terminal: true })
                    }
                  >
                    在终端中打开
                  </button>
                </div>
              </details>
            </div>
          ))
        ) : (
          <p className="muted">完成首次同步后在这里显示。</p>
        )}
      </div>
    </section>
  );
}

export function BackendPanel({
  snapshot,
  act,
  app,
  mode = "all",
}: {
  snapshot: Snapshot | null;
  act: Act;
  app: AppState | null;
  mode?: "permissions" | "advanced" | "all";
}) {
  const [permissions, setPermissions] = useState<any>(null),
    [input, setInput] = useState<any>(null),
    [text, setText] = useState<string | null>(null),
    [server, setServer] = useState(""),
    [tools, setTools] = useState<any>(null),
    [tool, setTool] = useState(""),
    [args, setArgs] = useState("{}"),
    [observation, setObservation] = useState<any>(null),
    [parseError, setParseError] = useState(""),
    [pending, setPending] = useState(false);
  const check = async () => {
    setPending(true);
    try {
      setPermissions(await act("permissions"));
      setInput(await act("input_status"));
    } finally {
      setPending(false);
    }
  };
  useEffect(() => {
    if (app && mode !== "advanced") void check();
  }, [!!app, mode]);
  const observe = async () => {
    let argumentsValue;
    try {
      argumentsValue = JSON.parse(args);
      setParseError("");
    } catch {
      setParseError("工具参数必须为有效 JSON");
      return;
    }
    setPending(true);
    try {
      const r = await act("control", {
        action: "observe",
        args: {
          server,
          session: tools?.session,
          tool,
          arguments: argumentsValue,
        },
      });
      if (r) setObservation({ task_id: r.task_id, status: r.status });
    } finally {
      setPending(false);
    }
  };
  useEffect(() => {
    if (!observation?.task_id || observation?.ended_at) return;
    let cancelled = false;
    const timer = setInterval(async () => {
      const r = await act("control", {
        action: "task_detail",
        args: { task_id: observation.task_id },
      });
      if (r && !cancelled) setObservation(r);
    }, 1000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [observation?.task_id, observation?.ended_at]);
  const checks = [
    {
      name: "辅助功能",
      ok: permissions?.accessibility,
      detail: permissions?.accessibility
        ? "Macrun 已授权；后端需单独授权"
        : "用于应用点击和输入",
      kind: "accessibility",
    },
    {
      name: "屏幕录制",
      ok: permissions?.screen_recording,
      detail: permissions?.screen_recording
        ? "应用已授权；请通过后端实拍验证"
        : "授权后仍需通过后端截图核对",
      kind: "screen",
    },
    {
      name: "图形登录会话",
      ok: permissions?.graphical_session,
      detail: permissions?.graphical_session
        ? "当前用户已登录"
        : "桌面操作需要图形登录会话",
    },
    {
      name: "防止自动休眠",
      ok: permissions?.keep_awake,
      detail: permissions?.keep_awake
        ? "桌面操作期间正在保持唤醒"
        : "睡眠后连接与桌面操作都会中断",
      awake: true,
    },
  ];
  return (
    <>
      {mode !== "advanced" && (
        <section className="requirements-section">
          <div className="row between">
            <h2>这台 Mac 是否具备条件</h2>
            <button className="link" disabled={pending} onClick={check}>
              检查系统权限
            </button>
          </div>
          <div className="card">
            {checks.map((c) => (
              <div className="feature-line requirement-row" key={c.name}>
                <span
                  className={`check-icon ${c.ok ? "ok" : permissions ? "warn" : "unchecked"}`}
                >
                  {c.ok ? <Check size={13} /> : <CircleAlert size={13} />}
                </span>
                <div className="grow">
                  <b>{c.name}</b>
                  <small>{permissions ? c.detail : "尚未检查"}</small>
                </div>
                {c.kind && permissions && !c.ok && (
                  <button
                    onClick={() => act("open_permission", { kind: c.kind })}
                  >
                    系统设置
                  </button>
                )}
                {c.awake && app && (
                  <button
                    onClick={() =>
                      act("save_preferences", {
                        preferences: {
                          ...app.preferences,
                          keep_awake: !app.preferences.keep_awake,
                        },
                      })
                    }
                  >
                    {app.preferences.keep_awake ? "关闭保持唤醒" : "保持唤醒"}
                  </button>
                )}
              </div>
            ))}
          </div>
          {input && !input.available && (
            <small className="input-permission-note">
              本机输入检测尚不可用。
              <button
                className="link"
                onClick={() => act("open_permission", { kind: "input" })}
              >
                授权输入监控
              </button>
              后重启应用。
            </small>
          )}
        </section>
      )}
      {mode !== "permissions" && (
        <div className="backend-advanced">
          <details className="feature-disclosure card">
            <summary>后端实拍与状态核对</summary>
            <div className="feature-disclosure-body">
              <p className="muted">
                选择后端公布的只读观察或截图工具。结果未知的操作不会自动重放。
              </p>
              <label className="field">
                <span>后端</span>
                <select
                  value={server}
                  onChange={(e) => {
                    setServer(e.target.value);
                    setTools(null);
                    setTool("");
                  }}
                >
                  <option value="">选择后端</option>
                  {snapshot?.backends.map((b) => (
                    <option key={b.name}>{b.name}</option>
                  ))}
                </select>
              </label>
              <div className="actions">
                <button
                  disabled={!server || pending}
                  onClick={async () =>
                    setTools(
                      await act("control", {
                        action: "tools",
                        args: { server },
                      }),
                    )
                  }
                >
                  读取工具
                </button>
                <button
                  disabled={!server || pending}
                  onClick={async () => {
                    await act(
                      "control",
                      { action: "restart_backend", args: { server } },
                      "后端会话已失效，请重新读取工具",
                    );
                    setTools(null);
                    setTool("");
                  }}
                >
                  重启后端
                </button>
              </div>
              {tools && (
                <>
                  <label className="field">
                    <span>工具（{tools.result?.tools?.length || 0}）</span>
                    <select
                      value={tool}
                      onChange={(e) => setTool(e.target.value)}
                    >
                      <option value="">选择只读截图工具</option>
                      {tools.result?.tools?.map((t: any) => (
                        <option key={t.name}>{t.name}</option>
                      ))}
                    </select>
                  </label>
                  {tool && (
                    <pre className="schema">
                      {JSON.stringify(
                        tools.result?.tools?.find((t: any) => t.name === tool)
                          ?.inputSchema,
                        null,
                        2,
                      )}
                    </pre>
                  )}
                  <label className="field">
                    <span>工具参数 JSON</span>
                    <textarea
                      value={args}
                      onChange={(e) => setArgs(e.target.value)}
                    />
                  </label>
                  {parseError && (
                    <p className="error-text" role="alert">
                      {parseError}
                    </p>
                  )}
                  <button disabled={!tool || pending} onClick={observe}>
                    执行观察并核对
                  </button>
                </>
              )}
              {observation && (
                <>
                  <p>
                    状态：
                    {statuses[observation.status as keyof typeof statuses] ||
                      observation.status}
                  </p>
                  {observation.error && (
                    <p className="error-text">{observation.error.message}</p>
                  )}
                  <ToolResult value={observation.result} />
                </>
              )}
            </div>
          </details>
          <details className="feature-disclosure backend-config">
            <summary>
              <Plus size={15} />
              添加 / 编辑 MCP 后端
            </summary>
            <div className="feature-disclosure-body card">
              <button
                onClick={async () => {
                  const config = await act("backend_config");
                  if (typeof config === "string") setText(config);
                }}
              >
                读取配置
              </button>
              {text !== null && (
                <>
                  <label className="field">
                    <span>worker.toml（修改前断开连接）</span>
                    <textarea
                      className="config-editor"
                      value={text}
                      onChange={(e) => setText(e.target.value)}
                      spellCheck={false}
                    />
                  </label>
                  <button
                    disabled={app?.worker_running}
                    onClick={() =>
                      act(
                        "save_backends",
                        { text },
                        "配置已保存并备份，重新连接后生效",
                      )
                    }
                  >
                    保存配置
                  </button>
                </>
              )}
            </div>
          </details>
        </div>
      )}
    </>
  );
}
export function ToolResult({ value }: { value: any }) {
  return (
    <>
      {value?.result?.content?.map((c: any, i: number) =>
        c.type === "image" &&
        ["image/png", "image/jpeg"].includes(c.mimeType) ? (
          <img
            className="observation"
            key={i}
            alt="后端观察截图"
            src={`data:${c.mimeType};base64,${c.data}`}
          />
        ) : c.type === "text" ? (
          <pre className="schema" key={i}>
            {c.text}
          </pre>
        ) : null,
      )}
    </>
  );
}
export function Replay({
  tasks,
  act,
  onTasks,
}: {
  tasks: Task[];
  act: Act;
  onTasks?: () => void;
}) {
  const [detail, setDetail] = useState<any>(null);
  const [records, setRecords] = useState<Record<string, any>>({});
  const [loading, setLoading] = useState(false);
  const loadRecord = async (taskId: string) => {
    const cached = records[taskId];
    if (
      cached &&
      [
        "succeeded",
        "failed",
        "cancelled",
        "timed_out",
        "unknown",
        "denied",
      ].includes(cached.status)
    )
      return cached;
    const record = await act("control", {
      action: "task_detail",
      args: { task_id: taskId },
    });
    if (record) setRecords((previous) => ({ ...previous, [taskId]: record }));
    return record;
  };
  const thumbnail = (taskId: string) => {
    const content = records[taskId]?.result?.result?.content;
    const picture = content?.find(
      (c: any) =>
        c.type === "image" && ["image/png", "image/jpeg"].includes(c.mimeType),
    );
    return picture
      ? `data:${picture.mimeType};base64,${picture.data}`
      : undefined;
  };
  const recent = tasks.filter((t) => t.kind === "mcp.call").slice(0, 8);
  return (
    <section className="replay-section">
      <div className="row between">
        <h2>最近操作</h2>
        <div className="actions">
          {recent.length > 0 && (
            <button
              className="link"
              disabled={loading}
              onClick={async () => {
                setLoading(true);
                try {
                  for (const task of recent) await loadRecord(task.task_id);
                } finally {
                  setLoading(false);
                }
              }}
            >
              {loading ? "读取中…" : "显示截图"}
            </button>
          )}
          {onTasks && (
            <button className="link" onClick={onTasks}>
              在任务中查看
            </button>
          )}
        </div>
      </div>
      {recent.length ? (
        <div className="replay-frames">
          {recent.map((t, i) => (
            <button
              className={`card replay-frame ${detail?.task_id === t.task_id ? "selected" : ""}`}
              key={t.task_id}
              onClick={async () => setDetail(await loadRecord(t.task_id))}
            >
              <div className="replay-preview">
                {thumbnail(t.task_id) ? (
                  <img
                    src={thumbnail(t.task_id)}
                    alt={`${t.arguments.tool || "桌面操作"}的记录截图`}
                  />
                ) : (
                  <>
                    <Monitor size={24} />
                    <small>
                      {records[t.task_id]
                        ? ["accepted", "running", "awaiting_approval"].includes(
                            records[t.task_id].status,
                          )
                          ? "操作进行中，点按刷新"
                          : "这次操作没有截图"
                        : "查看操作记录"}
                    </small>
                  </>
                )}
              </div>
              <div className="replay-caption">
                <span className="muted mono">
                  {String(i + 1).padStart(2, "0")}
                </span>
                <b className="ellipsis">{t.arguments.tool || "桌面调用"}</b>
                <small className="mono">
                  {new Date(t.started_at).toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}{" "}
                  · {statuses[t.status as keyof typeof statuses] || t.status}
                </small>
              </div>
            </button>
          ))}
        </div>
      ) : (
        <div className="card replay-empty muted">
          完成桌面操作后，可在这里查看调用记录和截图。
        </div>
      )}
      {detail && (
        <div className="card replay-detail">
          <div className="row between">
            <h3>操作详情</h3>
            <button
              className="icon-button"
              aria-label="关闭操作详情"
              onClick={() => setDetail(null)}
            >
              <X size={15} />
            </button>
          </div>
          <pre className="schema">
            {JSON.stringify(detail.arguments, null, 2)}
          </pre>
          {detail.arguments?.arguments?.x !== undefined &&
            detail.arguments?.arguments?.y !== undefined && (
              <p className="mono">
                点击位置：({detail.arguments.arguments.x},{" "}
                {detail.arguments.arguments.y})
              </p>
            )}
          <ToolResult value={detail.result} />
        </div>
      )}
    </section>
  );
}
