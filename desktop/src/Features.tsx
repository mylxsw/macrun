import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  CircleAlert,
  Info,
  Folder,
  Monitor,
  Plus,
  Terminal,
  X,
} from "lucide-react";
import type {
  Snapshot,
  Task,
  AppState,
  AllowRule,
  DesktopTier,
  TierPolicy,
} from "./types";
import {
  statuses,
  riskReasons,
  tierLabels,
  tierPolicyLabels,
} from "./model.mjs";
import "./features.css";
import appIcon from "./assets/macrun-icon.png";
export type Act = (
  c: string,
  a?: Record<string, unknown>,
  s?: string,
) => Promise<any>;
const readNative: Act = (command, args) => invoke(command, args);
const terminalStatuses = new Set([
  "succeeded",
  "failed",
  "cancelled",
  "timed_out",
  "unknown",
  "denied",
]);
const failureText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

/** Re-renders every second while `enabled`, for countdowns. */
export function useNow(enabled: boolean) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!enabled) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [enabled]);
  return now;
}
const APPROVAL_MS = 60_000;
export function Approvals({
  tasks,
  act,
  disabled,
  compact = false,
}: {
  tasks: Task[];
  act: Act;
  disabled: boolean;
  compact?: boolean;
}) {
  const waiting = tasks.filter((t) => t.status === "awaiting_approval");
  const [index, setIndex] = useState(0);
  const now = useNow(waiting.length > 0);
  if (!waiting.length) return null;
  const shown = compact
    ? [waiting[Math.min(index, waiting.length - 1)]]
    : waiting;
  const decide = (t: Task, allow: boolean, scope = "once") =>
    act(
      "control",
      { action: "approve", args: { task_id: t.task_id, allow, scope } },
      allow
        ? {
            once: "已允许这一次",
            similar: "已允许，15 分钟内同类请求不再询问",
            session: "已允许，执行器重启前不再询问",
          }[scope]
        : "已拒绝，Agent 会收到 approval_rejected",
    );
  return (
    <>
      {shown.map((t) => {
        const desktop = t.kind === "mcp.call";
        const deadline = t.approval_deadline || t.started_at + APPROVAL_MS;
        const left = Math.max(0, Math.ceil((deadline - now) / 1000));
        const reasons = desktop
          ? [`${tierLabels[t.desktop_tier || "control"]}类桌面操作`]
          : riskReasons(t.arguments.command);
        return (
          <section
            className={`card approval ${compact ? "compact" : ""}`}
            key={t.task_id}
            aria-label="等待你确认"
          >
            <div className="row approval-head">
              <span
                className="approval-ring"
                style={{
                  ["--left" as string]: `${(left / (APPROVAL_MS / 1000)) * 100}%`,
                }}
                aria-hidden
              />
              <strong>{desktop ? "桌面操作需要你确认" : "命令需要你确认"}</strong>
              <small className="approval-left" role="timer">
                {left} 秒后过期，Agent 会收到 approval_expired
              </small>
              {compact && waiting.length > 1 && (
                <span className="approval-pager push">
                  <button
                    className="icon-button"
                    aria-label="上一条待确认"
                    onClick={() =>
                      setIndex((i) => (i + waiting.length - 1) % waiting.length)
                    }
                  >
                    ‹
                  </button>
                  {Math.min(index, waiting.length - 1) + 1} / {waiting.length}
                  <button
                    className="icon-button"
                    aria-label="下一条待确认"
                    onClick={() => setIndex((i) => (i + 1) % waiting.length)}
                  >
                    ›
                  </button>
                </span>
              )}
            </div>
            <pre className="command">
              {t.arguments.command ||
                `${t.arguments.server} · ${t.arguments.tool}`}
            </pre>
            <div className="row approval-meta">
              {reasons.map((r) => (
                <span className="tag approval-reason" key={r}>
                  {r}
                </span>
              ))}
              {t.arguments.cwd && <small className="mono">{t.arguments.cwd}</small>}
            </div>
            <div className="actions">
              <button disabled={disabled} onClick={() => decide(t, false)}>
                拒绝
              </button>
              <span className="push" />
              <button
                disabled={disabled}
                title={
                  desktop
                    ? "15 分钟内，同一后端的同一工具不再询问"
                    : "15 分钟内，同一程序在这个目录及子目录中不再询问"
                }
                onClick={() => decide(t, true, "similar")}
              >
                15 分钟内允许同类
              </button>
              {!compact && (
                <button
                  disabled={disabled}
                  title="执行器重启后失效，可在设置与安全中撤销"
                  onClick={() => decide(t, true, "session")}
                >
                  {desktop
                    ? `本次运行允许${tierLabels[t.desktop_tier || "control"]}类`
                    : "本次运行允许此目录"}
                </button>
              )}
              <button
                disabled={disabled}
                className="primary"
                onClick={() => decide(t, true)}
              >
                允许一次
              </button>
            </div>
          </section>
        );
      })}
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
            <img src={appIcon} alt="" className="brand-icon" />
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
                    {c.ok ? <Check size={13} /> : c.optional ? <Info size={13} /> : <CircleAlert size={13} />}
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

const tierCopy: Record<DesktopTier, { title: string; detail: string }> = {
  observe: {
    title: "观察 · 看屏幕和窗口",
    detail: "截图、读取窗口和可访问性树。可能看到其他应用里的内容。",
  },
  control: {
    title: "操作 · 点击和输入",
    detail: "点击、输入、按键、滚动、拖动和切换窗口。操作时显示屏幕提示。",
  },
  high: {
    title: "高风险 · 不可撤回",
    detail: "结束进程、下载文件、回放录制等后端标为最高风险的工具。",
  },
};
const tierOrder: DesktopTier[] = ["observe", "control", "high"];
const policyOrder: TierPolicy[] = ["deny", "confirm", "allow"];
/** Desktop tool tiers. Defaults allow everything; each tier can be tightened. */
export function DesktopTiers({
  snapshot,
  act,
  disabled,
}: {
  snapshot: Snapshot | null;
  act: Act;
  disabled: boolean;
}) {
  const policy = snapshot?.safety.desktop;
  const tools: Record<DesktopTier, string[]> = {
    observe: [],
    control: [],
    high: [],
  };
  for (const backend of snapshot?.backends || [])
    for (const [name, tier] of Object.entries(backend.tiers || {}))
      tools[tier]?.push(name);
  const known = Object.values(tools).some((list) => list.length);
  const save = (tier: DesktopTier, value: TierPolicy) =>
    snapshot &&
    policy &&
    act(
      "control",
      {
        action: "safety",
        args: { ...snapshot.safety, desktop: { ...policy, [tier]: value } },
      },
      `${tierLabels[tier]}类桌面工具已设为“${tierPolicyLabels[value]}”`,
    );
  return (
    <section className="desktop-tiers" aria-label="桌面工具分级">
      <div className="row between section-heading">
        <h2>Agent 能对这台 Mac 做什么</h2>
        <small className="muted">
          按后端声明的只读与风险等级归类
          {known ? "" : "；读取工具列表后显示每类包含的工具"}
        </small>
      </div>
      <div className="tier-grid">
        {tierOrder.map((tier) => (
          <article className={`card tier-card tier-${tier}`} key={tier}>
            <b>{tierCopy[tier].title}</b>
            <small>{tierCopy[tier].detail}</small>
            <div
              className="feature-seg tier-seg"
              role="group"
              aria-label={`${tierLabels[tier]}类桌面工具`}
            >
              {policyOrder.map((value) => (
                <button
                  key={value}
                  className={policy?.[tier] === value ? "on" : ""}
                  aria-pressed={policy?.[tier] === value}
                  disabled={disabled || !policy}
                  onClick={() => save(tier, value)}
                >
                  {tierPolicyLabels[value]}
                </button>
              ))}
            </div>
            {tools[tier].length > 0 && (
              <small className="mono tier-tools" title={tools[tier].join(" · ")}>
                {tools[tier].slice(0, 5).join(" · ")}
                {tools[tier].length > 5 ? ` · 共 ${tools[tier].length} 个` : ""}
              </small>
            )}
          </article>
        ))}
      </div>
    </section>
  );
}
function ruleText(rule: AllowRule) {
  if (rule.kind === "mcp.call")
    return rule.scope === "similar"
      ? `${rule.server} · ${rule.tool}`
      : `${rule.server} · 全部${tierLabels[rule.tier || "control"]}类工具`;
  return rule.scope === "similar"
    ? `${rule.program} · ${rule.cwd} 及子目录`
    : `所有命令 · ${rule.cwd} 及子目录`;
}
/** Temporary approvals granted from prompts; held in worker memory only. */
export function AllowRules({
  rules,
  act,
  disabled,
}: {
  rules: AllowRule[];
  act: Act;
  disabled: boolean;
}) {
  return (
    <div className="feature-line settings-split allow-rules">
      <div className="feature-copy">
        <b>临时允许</b>
        <small>
          在确认弹窗中选择“15 分钟内允许同类”或“本次运行允许”后生成；执行器重启后全部失效。
        </small>
      </div>
      <div className="allow-rule-list">
        {rules.map((rule) => (
          <div className="allow-rule" key={rule.id}>
            <span className="mono ellipsis" title={ruleText(rule)}>
              {ruleText(rule)}
            </span>
            <small>
              {rule.expires_at
                ? `${new Date(rule.expires_at).toLocaleTimeString("zh-CN", { hour12: false, hour: "2-digit", minute: "2-digit" })} 前`
                : "直到执行器重启"}
            </small>
            <button
              disabled={disabled}
              onClick={() =>
                act(
                  "control",
                  { action: "revoke_rule", args: { rule_id: rule.id } },
                  "已撤销临时允许",
                )
              }
            >
              撤销
            </button>
          </div>
        ))}
        {!rules.length && <small className="muted">暂无临时允许</small>}
      </div>
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
    hint: "每条命令都需要你点“允许”；桌面调用按桌面控制页的分级管理。",
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
      <AllowRules
        rules={snapshot?.allow_rules || []}
        act={act}
        disabled={disabled}
      />
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
  const [limit, setLimit] = useState(6);
  const workspaces = snapshot?.workspaces || [];
  return (
    <section className="workspace-section">
      <div className="row between">
        <h2>工作区</h2>
        <small>由服务器同步过来的目录</small>
      </div>
      <div className="card workspace-list">
        {workspaces.length ? (
          workspaces.slice(0, limit).map((w) => (
            <div className="feature-line workspace-row" key={w.root}>
              <Folder size={17} />
              <div className="grow">
                <b className="ellipsis" title={w.root}>
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
      {workspaces.length > 6 && (
        <div className="row between workspace-pagination">
          <small>
            已显示 {Math.min(limit, workspaces.length)} / {workspaces.length}{" "}
            个工作区
          </small>
          <div className="actions">
            {limit > 6 && (
              <button className="link" onClick={() => setLimit(6)}>
                收起
              </button>
            )}
            {limit < workspaces.length && (
              <button className="link" onClick={() => setLimit((v) => v + 6)}>
                查看更多工作区
              </button>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

export function BackendPanel({
  snapshot,
  act,
  app,
  mode = "all",
  read = readNative,
}: {
  snapshot: Snapshot | null;
  act: Act;
  app: AppState | null;
  mode?: "permissions" | "advanced" | "all";
  read?: Act;
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
    [pending, setPending] = useState(""),
    [permissionPending, setPermissionPending] = useState(false),
    [permissionError, setPermissionError] = useState(""),
    [operationError, setOperationError] = useState(""),
    [configError, setConfigError] = useState(""),
    [configDirty, setConfigDirty] = useState(false),
    [configNotice, setConfigNotice] = useState(""),
    [observationError, setObservationError] = useState(""),
    [observationRetry, setObservationRetry] = useState(0);
  const operationLock = useRef(false),
    permissionLock = useRef(false);
  const check = useCallback(async () => {
    if (permissionLock.current) return;
    permissionLock.current = true;
    setPermissionPending(true);
    setPermissionError("");
    try {
      const results = await Promise.allSettled([
        read("permissions"),
        read("input_status"),
      ]);
      const errors = [];
      if (results[0].status === "fulfilled" && results[0].value)
        setPermissions(results[0].value);
      else {
        setPermissions(null);
        errors.push(
          results[0].status === "rejected"
            ? failureText(results[0].reason)
            : "未收到权限检查结果",
        );
      }
      if (results[1].status === "fulfilled" && results[1].value)
        setInput(results[1].value);
      else {
        setInput(null);
        errors.push(
          results[1].status === "rejected"
            ? failureText(results[1].reason)
            : "未收到输入检测结果",
        );
      }
      setPermissionError(errors.join("；"));
    } finally {
      permissionLock.current = false;
      setPermissionPending(false);
    }
  }, [read]);
  useEffect(() => {
    if (!app || mode === "advanced") return;
    const refreshPermissions = () => {
      if (document.visibilityState !== "hidden") void check();
    };
    refreshPermissions();
    window.addEventListener("focus", refreshPermissions);
    document.addEventListener("visibilitychange", refreshPermissions);
    const timer = window.setInterval(refreshPermissions, 5000);
    return () => {
      window.removeEventListener("focus", refreshPermissions);
      document.removeEventListener("visibilitychange", refreshPermissions);
      window.clearInterval(timer);
    };
  }, [!!app, mode, check]);
  const perform = async (
    name: string,
    action: () => Promise<void>,
    target: "permissions" | "config" | "tools" = "tools",
  ) => {
    if (operationLock.current) return;
    operationLock.current = true;
    setPending(name);
    const setError =
      target === "config"
        ? setConfigError
        : target === "permissions"
          ? setPermissionError
          : setOperationError;
    setError("");
    try {
      await action();
    } catch (error) {
      setError(failureText(error));
    } finally {
      operationLock.current = false;
      setPending("");
    }
  };
  const mutate = async (
    command: string,
    params?: Record<string, unknown>,
    success?: string,
  ) => {
    const result =
      success === undefined
        ? await act(command, params)
        : await act(command, params, success);
    if (result === undefined || result === false)
      throw new Error("操作未完成，请查看错误提示后重试。");
    return result;
  };
  const observe = async () => {
    let argumentsValue;
    try {
      argumentsValue = JSON.parse(args);
      if (
        !argumentsValue ||
        Array.isArray(argumentsValue) ||
        typeof argumentsValue !== "object"
      ) {
        setParseError("工具参数必须为 JSON 对象，例如 {}。");
        return;
      }
      setParseError("");
    } catch {
      setParseError("工具参数必须为有效 JSON");
      return;
    }
    await perform("observe", async () => {
      const r = await mutate("control", {
        action: "observe",
        args: {
          server,
          session: tools?.session,
          tool,
          arguments: argumentsValue,
        },
      });
      if (!r?.task_id)
        throw new Error("未收到操作记录，请在任务列表核对，勿重复执行。");
      setObservationError("");
      setObservation(r);
    });
  };
  useEffect(() => {
    if (
      !observation?.task_id ||
      observation?.ended_at ||
      terminalStatuses.has(observation?.status)
    )
      return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const r = await read("control", {
          action: "task_detail",
          args: { task_id: observation.task_id },
        });
        if (!r?.task_id) throw new Error("未收到操作状态");
        if (cancelled) return;
        setObservation(r);
        setObservationError("");
        if (!r.ended_at && !terminalStatuses.has(r.status))
          timer = setTimeout(poll, 1000);
      } catch (error) {
        if (!cancelled)
          setObservationError(
            `无法读取操作状态：${failureText(error)}。可重新读取状态，不会再次执行工具。`,
          );
      }
    };
    timer = setTimeout(poll, 1000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [observation?.task_id, observationRetry, read]);
  const observing =
    !!observation?.task_id &&
    !observation?.ended_at &&
    !terminalStatuses.has(observation?.status);
  const executorAvailable =
    !!snapshot && app?.worker_running !== false && !app?.worker_starting;
  const discoverTools = (append = false) =>
    perform("tools", async () => {
      const cursor = append ? tools?.result?.nextCursor : undefined;
      const value = await read("control", {
        action: "tools",
        args: { server, ...(append ? { session: tools.session, cursor } : {}) },
      });
      if (
        !value?.session ||
        !Array.isArray(value.result?.tools) ||
        value.result.tools.some(
          (t: any) => !t || typeof t.name !== "string" || !t.name,
        )
      )
        throw new Error("后端未返回有效工具列表");
      if (append && value.session !== tools.session)
        throw new Error("后端会话已变化，请重新读取工具。");
      const cursors = append ? [...(tools.cursors || []), cursor] : [];
      if (value.result.nextCursor && cursors.includes(value.result.nextCursor))
        throw new Error("后端重复返回分页位置，请重新读取工具。");
      const items = append
        ? [...tools.result.tools, ...value.result.tools]
        : value.result.tools;
      const uniqueTools = [
        ...new Map(items.map((t: any) => [t.name, t])).values(),
      ];
      setTools({
        ...value,
        cursors,
        result: { ...value.result, tools: uniqueTools },
      });
      if (!append) setTool("");
    });
  const checks = [
    {
      name: "辅助功能 · Macrun Desktop",
      optional: true,
      ok: permissions?.accessibility,
      detail: permissions?.accessibility
        ? "当前应用已授权；后端点击和输入仍使用后端自己的权限"
        : "当前应用未获授权；不代表桌面后端未授权",
      kind: "accessibility",
    },
    {
      name: "屏幕录制 · Macrun Desktop",
      optional: true,
      ok: permissions?.screen_recording,
      detail: permissions?.screen_recording
        ? "当前应用已授权；实际截图由后端完成"
        : "当前应用未获授权；截图由后端完成，无需为此重复授权",
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
            <h2>Macrun 自身状态</h2>
            <button
              className="link"
              disabled={permissionPending}
              onClick={check}
            >
              {permissionPending ? "检查中…" : "检查系统权限"}
            </button>
          </div>
          <div className="card">
            {checks.map((c) => (
              <div className="feature-line requirement-row" key={c.name}>
                <span
                  className={`check-icon ${c.ok ? "ok" : permissions && !c.optional ? "warn" : "unchecked"}`}
                >
                  {c.ok ? <Check size={13} /> : c.optional ? <Info size={13} /> : <CircleAlert size={13} />}
                </span>
                <div className="grow">
                  <b>{c.name}</b>
                  <small>{permissions ? c.detail : "尚未检查"}</small>
                </div>
                {c.kind && permissions && !c.ok && (
                  <button
                    disabled={!!pending}
                    onClick={() =>
                      perform(
                        "permission",
                        () => mutate("open_permission", { kind: c.kind }),
                        "permissions",
                      )
                    }
                  >
                    系统设置
                  </button>
                )}
                {c.awake && app && (
                  <button
                    disabled={!!pending}
                    onClick={() =>
                      perform(
                        "awake",
                        async () => {
                          await mutate("save_preferences", {
                            preferences: {
                              ...app.preferences,
                              keep_awake: !app.preferences.keep_awake,
                            },
                          });
                          await check();
                        },
                        "permissions",
                      )
                    }
                  >
                    {app.preferences.keep_awake ? "关闭保持唤醒" : "保持唤醒"}
                  </button>
                )}
              </div>
            ))}
          </div>
          <small className="input-permission-note">
            以上权限仅检查 Macrun
            应用，不代表桌面后端的权限。请为实际后端应用（例如 CuaDriver）授权，
            并在“后端实拍与状态核对”中检查；旧版 macrun 的授权也不等同于 Macrun Desktop。
          </small>
          {permissions && (!permissions.accessibility || !permissions.screen_recording) && (
            <small className="input-permission-note">
              如果系统设置中已开启，但当前应用仍未获授权，请完全退出后重新打开。
              更换过签名的旧授权可能需要在系统设置中重新添加当前应用。返回此页后会自动刷新。
            </small>
          )}
          {permissionError && (
            <p className="error-text" role="alert">
              {permissionError}
            </p>
          )}
          {input && !input.available && (
            <small className="input-permission-note">
              本机输入检测尚不可用。
              <button
                className="link"
                disabled={!!pending}
                onClick={() =>
                  perform(
                    "permission",
                    () => mutate("open_permission", { kind: "input" }),
                    "permissions",
                  )
                }
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
                工具由后端提供，可能包含点击和输入操作。请核对用途后选择只读观察或截图工具；结果未知的操作不会自动重放。
              </p>
              <label className="field">
                <span>后端</span>
                <select
                  value={server}
                  disabled={!!pending || observing}
                  onChange={(e) => {
                    setServer(e.target.value);
                    setTools(null);
                    setTool("");
                    setObservation(null);
                    setObservationError("");
                    setOperationError("");
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
                  disabled={
                    !server || !!pending || observing || !executorAvailable
                  }
                  onClick={() => discoverTools()}
                >
                  {pending === "tools" ? "读取工具中…" : "读取工具"}
                </button>
                <button
                  disabled={
                    !server || !!pending || observing || !executorAvailable
                  }
                  onClick={() =>
                    perform("restart", async () => {
                      await mutate(
                        "control",
                        { action: "restart_backend", args: { server } },
                        "后端会话已失效，请重新读取工具",
                      );
                      setTools(null);
                      setTool("");
                    })
                  }
                >
                  {pending === "restart" ? "重启中…" : "重启后端"}
                </button>
              </div>
              {!executorAvailable && (
                <small className="input-permission-note">
                  连接执行器后可读取工具和核对桌面操作。
                </small>
              )}
              {tools && (
                <>
                  <label className="field">
                    <span>工具（{tools.result?.tools?.length || 0}）</span>
                    <select
                      value={tool}
                      disabled={!!pending || observing}
                      onChange={(e) => setTool(e.target.value)}
                    >
                      <option value="">选择观察或截图工具</option>
                      {tools.result?.tools?.map((t: any) => (
                        <option key={t.name}>{t.name}</option>
                      ))}
                    </select>
                  </label>
                  {tools.result?.nextCursor && (
                    <button
                      disabled={!!pending || observing || !executorAvailable}
                      onClick={() => discoverTools(true)}
                    >
                      读取更多工具
                    </button>
                  )}
                  {tool && (
                    <>
                      <p className="muted">
                        {tools.result?.tools?.find((t: any) => t.name === tool)
                          ?.description ||
                          "后端未提供工具说明，请先核对其用途。"}
                      </p>
                      <pre className="schema">
                        {JSON.stringify(
                          tools.result?.tools?.find((t: any) => t.name === tool)
                            ?.inputSchema,
                          null,
                          2,
                        )}
                      </pre>
                    </>
                  )}
                  <label className="field">
                    <span>工具参数 JSON</span>
                    <textarea
                      value={args}
                      disabled={!!pending || observing}
                      autoCorrect="off"
                      autoCapitalize="off"
                      spellCheck={false}
                      onChange={(e) => setArgs(e.target.value)}
                    />
                  </label>
                  {parseError && (
                    <p className="error-text" role="alert">
                      {parseError}
                    </p>
                  )}
                  <button
                    disabled={
                      !tool ||
                      !executorAvailable ||
                      !!pending ||
                      observing ||
                      !snapshot?.policy.desktop_enabled ||
                      snapshot.policy.paused
                    }
                    onClick={observe}
                  >
                    {pending === "observe" ? "提交中…" : "执行观察并核对"}
                  </button>
                  {(!snapshot?.policy.desktop_enabled ||
                    snapshot.policy.paused) && (
                    <small className="input-permission-note">
                      请先连接执行器、取消暂停并开启桌面控制。
                    </small>
                  )}
                </>
              )}
              {operationError && (
                <p className="error-text" role="alert">
                  {operationError}
                </p>
              )}
              {observation && (
                <>
                  <p>
                    状态：
                    {statuses[observation.status as keyof typeof statuses] ||
                      observation.status}
                  </p>
                  {observation.error && (
                    <p className="error-text" role="alert">
                      {observation.error.message}
                    </p>
                  )}
                  <ToolResult value={observation.result} />
                </>
              )}
              {observationError && (
                <>
                  <p className="error-text" role="alert">
                    {observationError}
                  </p>
                  <button
                    onClick={() => {
                      setObservationError("");
                      setObservationRetry((v) => v + 1);
                    }}
                  >
                    重新读取状态
                  </button>
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
                disabled={!!pending || configDirty}
                onClick={() =>
                  perform(
                    "config-read",
                    async () => {
                      const config = await read("backend_config");
                      if (typeof config !== "string")
                        throw new Error("未收到后端配置");
                      setText(config);
                      setConfigNotice("");
                    },
                    "config",
                  )
                }
              >
                {pending === "config-read" ? "读取配置中…" : "读取配置"}
              </button>
              {text !== null && (
                <>
                  <label className="field">
                    <span>worker.toml（修改前断开连接）</span>
                    <textarea
                      className="config-editor"
                      value={text}
                      disabled={!!pending}
                      onChange={(e) => {
                        setText(e.target.value);
                        setConfigDirty(true);
                        setConfigNotice("");
                      }}
                      spellCheck={false}
                    />
                  </label>
                  <button
                    disabled={
                      !!pending || app?.worker_running || app?.worker_starting
                    }
                    onClick={() =>
                      perform(
                        "config-save",
                        async () => {
                          await mutate(
                            "save_backends",
                            { text },
                            "配置已保存并备份，重新连接后生效",
                          );
                          setConfigDirty(false);
                          setConfigNotice("配置已保存并备份，重新连接后生效。");
                        },
                        "config",
                      )
                    }
                  >
                    {pending === "config-save" ? "保存中…" : "保存配置"}
                  </button>
                  {configDirty && (
                    <small className="input-permission-note">
                      更改尚未保存。
                    </small>
                  )}
                  {(app?.worker_running || app?.worker_starting) && (
                    <small className="input-permission-note">
                      请先断开连接，再保存后端配置。
                    </small>
                  )}
                </>
              )}
              {configError && (
                <p className="error-text" role="alert">
                  {configError}
                </p>
              )}
              {configNotice && <p role="status">{configNotice}</p>}
            </div>
          </details>
        </div>
      )}
    </>
  );
}
const imageTypes = new Set([
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
]);
const toolContent = (value: any): any[] =>
  Array.isArray(value?.result?.content) ? value.result.content : [];
const imageSource = (content: any) =>
  content?.type === "image" &&
  imageTypes.has(content.mimeType) &&
  typeof content.data === "string" &&
  content.data.length > 0
    ? `data:${content.mimeType};base64,${content.data}`
    : undefined;
function ResultImage({
  src,
  alt,
  className,
}: {
  src: string;
  alt: string;
  className?: string;
}) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  return failed ? (
    <small role="status">截图无法显示，请核对后端返回的图片格式。</small>
  ) : (
    <img
      className={className}
      src={src}
      alt={alt}
      onError={() => setFailed(true)}
    />
  );
}
function structuredPreview(value: unknown) {
  let remaining = 250;
  let truncated = false;
  const omitted = () => {
    truncated = true;
    return "…（已省略）";
  };
  const limit = (item: unknown, depth = 0): unknown => {
    if (remaining-- <= 0 || depth > 6) return omitted();
    if (typeof item === "string")
      return item.length > 1000 ? item.slice(0, 1000) + omitted() : item;
    if (Array.isArray(item)) {
      const result = item.slice(0, 40).map((entry) => limit(entry, depth + 1));
      if (item.length > 40) result.push(omitted());
      return result;
    }
    if (item && typeof item === "object") {
      const keys = Object.keys(item);
      const result = Object.fromEntries(
        keys
          .slice(0, 40)
          .map((key) => [
            key.length > 120 ? key.slice(0, 120) + omitted() : key,
            limit((item as Record<string, unknown>)[key], depth + 1),
          ]),
      );
      if (keys.length > 40) result["…"] = omitted();
      return result;
    }
    return item;
  };
  let text = JSON.stringify(limit(value), null, 2) ?? "null";
  if (text.length > 16000) text = text.slice(0, 16000) + omitted();
  return { text, truncated };
}
export function ToolResult({ value }: { value: any }) {
  const content = toolContent(value);
  const structured =
    value?.result?.structuredContent === undefined
      ? null
      : structuredPreview(value.result.structuredContent);
  return (
    <>
      {value?.result?.isError === true && (
        <p className="error-text" role="alert">
          后端工具报告执行失败，请查看返回内容。
        </p>
      )}
      {content.map((c: any, i: number) =>
        imageSource(c) ? (
          <ResultImage
            className="observation"
            key={i}
            alt="后端观察截图"
            src={imageSource(c)!}
          />
        ) : c?.type === "text" && typeof c.text === "string" ? (
          <pre className="schema" key={i}>
            {c.text}
          </pre>
        ) : (
          <small className="input-permission-note" key={i}>
            此返回内容暂不支持预览。
          </small>
        ),
      )}
      {structured && (
        <details className="feature-disclosure">
          <summary>结构化结果</summary>
          <pre className="schema" aria-label="结构化结果 JSON">
            {structured.text}
          </pre>
          {structured.truncated && (
            <small className="input-permission-note">
              预览已截断较长内容；这里只显示结果，不会执行工具。
            </small>
          )}
        </details>
      )}
    </>
  );
}
export function Replay({
  tasks,
  onTasks,
  read = readNative,
}: {
  tasks: Task[];
  act: Act;
  onTasks?: () => void;
  read?: Act;
}) {
  const [detail, setDetail] = useState<any>(null);
  const [records, setRecords] = useState<Record<string, any>>({});
  const [loading, setLoading] = useState(false);
  const [loadingTask, setLoadingTask] = useState("");
  const [error, setError] = useState("");
  const recordsRef = useRef<Record<string, any>>({});
  const inFlight = useRef(new Map<string, Promise<any>>());
  const selection = useRef(0);
  const screenshotsLoading = useRef(false);
  const detailSection = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!detail?.task_id) return;
    detailSection.current?.scrollIntoView?.({
      block: "nearest",
      behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches
        ? "auto"
        : "smooth",
    });
  }, [detail?.task_id]);
  const recent = tasks.filter((t) => t.kind === "mcp.call").slice(0, 8);
  const recentIds = recent.map((t) => t.task_id).join("\u0000");
  const visibleIds = useRef(new Set<string>());
  visibleIds.current = new Set(recent.map((t) => t.task_id));
  useEffect(() => {
    recordsRef.current = Object.fromEntries(
      Object.entries(recordsRef.current).filter(([id]) =>
        visibleIds.current.has(id),
      ),
    );
    setRecords(recordsRef.current);
  }, [recentIds]);
  const loadRecord = (taskId: string): Promise<any> => {
    const cached = recordsRef.current[taskId];
    if (cached && terminalStatuses.has(cached.status))
      return Promise.resolve(cached);
    const existing = inFlight.current.get(taskId);
    if (existing) return existing;
    const request = (async () => {
      const record = await read("control", {
        action: "task_detail",
        args: { task_id: taskId },
      });
      if (record?.task_id !== taskId) throw new Error("未收到匹配的操作记录");
      if (visibleIds.current.has(taskId)) {
        recordsRef.current = { ...recordsRef.current, [taskId]: record };
        setRecords(recordsRef.current);
      }
      return record;
    })().finally(() => inFlight.current.delete(taskId));
    inFlight.current.set(taskId, request);
    return request;
  };
  const thumbnail = (taskId: string) =>
    imageSource(
      toolContent(records[taskId]?.result).find((c) => imageSource(c)),
    );
  const openRecord = async (taskId: string) => {
    const version = ++selection.current;
    setLoadingTask(taskId);
    setDetail(null);
    setError("");
    try {
      const record = await loadRecord(taskId);
      if (version === selection.current) setDetail(record);
    } catch (error) {
      if (version === selection.current)
        setError(`操作记录读取失败：${failureText(error)}`);
    } finally {
      if (version === selection.current) setLoadingTask("");
    }
  };
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
                if (screenshotsLoading.current) return;
                screenshotsLoading.current = true;
                setLoading(true);
                setError("");
                const failures: string[] = [];
                try {
                  for (const task of recent) {
                    try {
                      await loadRecord(task.task_id);
                    } catch (error) {
                      failures.push(
                        `${task.arguments.tool || "桌面操作"}：${failureText(error)}`,
                      );
                    }
                  }
                  if (failures.length)
                    setError(
                      `部分截图读取失败，可重试：${failures.join("；")}`,
                    );
                } finally {
                  screenshotsLoading.current = false;
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
      {error && (
        <p className="error-text" role="alert">
          {error}
        </p>
      )}
      {recent.length ? (
        <div className="replay-frames">
          {recent.map((t, i) => (
            <button
              className={`card replay-frame ${detail?.task_id === t.task_id ? "selected" : ""}`}
              key={t.task_id}
              aria-busy={loadingTask === t.task_id}
              disabled={loadingTask === t.task_id}
              onClick={() => openRecord(t.task_id)}
            >
              <div className="replay-preview">
                {thumbnail(t.task_id) ? (
                  <ResultImage
                    src={thumbnail(t.task_id)!}
                    alt={`${t.arguments.tool || "桌面操作"}的记录截图`}
                  />
                ) : (
                  <>
                    <Monitor size={24} />
                    <small>
                      {loadingTask === t.task_id
                        ? "读取记录中…"
                        : records[t.task_id]
                          ? terminalStatuses.has(records[t.task_id].status)
                            ? "这次操作没有截图"
                            : "操作进行中，点按刷新"
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
        <div className="card replay-detail" ref={detailSection}>
          <div className="row between">
            <h3>操作详情</h3>
            <button
              className="icon-button"
              aria-label="关闭操作详情"
              onClick={() => {
                selection.current++;
                setDetail(null);
                setLoadingTask("");
              }}
            >
              <X size={15} />
            </button>
          </div>
          <p>
            状态：
            {statuses[detail.status as keyof typeof statuses] || detail.status}
          </p>
          {detail.error?.message && (
            <p className="error-text" role="alert">
              {detail.error.message}
            </p>
          )}
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
