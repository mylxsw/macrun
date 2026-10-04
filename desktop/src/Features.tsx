import { useEffect, useState } from "react";
import type { Snapshot, Task, AppState } from "./types";
type Act = (c: string, a?: Record<string, unknown>, s?: string) => Promise<any>;
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
            <pre className="command">{t.arguments.command}</pre>
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
export function Pairing({ act, running }: { act: Act; running: boolean }) {
  const [uri, setUri] = useState(""),
    [result, setResult] = useState<any>(null),
    [step, setStep] = useState(1),
    [pending, setPending] = useState(false);
  return (
    <section className="card">
      <h2>连接你的服务器</h2>
      <p className="muted">{step} / 3 · 配对码 → 检查连接 → 桌面控制</p>
      {step === 1 ? (
        <>
          <p>
            在服务器运行{" "}
            <code>macrun invite --data 服务端数据目录 --server 地址:7443</code>
            ，粘贴生成的十分钟一次性配对码。
          </p>
          <label className="field">
            <span>配对码</span>
            <textarea
              value={uri}
              onChange={(e) => setUri(e.target.value)}
              spellCheck={false}
              placeholder="macrun://pair/…"
            />
          </label>
          <button
            className="primary"
            disabled={pending || running || !uri.trim()}
            onClick={async () => {
              setPending(true);
              const r = await act("pair", { uri });
              setPending(false);
              if (r) {
                setResult(r);
                setUri("");
                setStep(2);
                await act("start_worker");
              }
            }}
          >
            安全配对
          </button>
        </>
      ) : step === 2 ? (
        <>
          <p>证书指纹已固定，凭据已保存至系统钥匙串。</p>
          <small className="mono wrap">{result?.fingerprint}</small>
          <button
            onClick={async () => {
              const r = await act("connection_check");
              if (r)
                setResult((v: any) => ({
                  ...v,
                  checks: r.checks,
                  error: r.error,
                }));
            }}
          >
            检查连接
          </button>
          <button
            onClick={async () => {
              const r = await act("control", { action: "self_test", args: {} });
              if (r) setResult((v: any) => ({ ...v, testId: r.task_id }));
            }}
          >
            试运行一条本机命令
          </button>
          {result?.testId && (
            <button
              onClick={async () => {
                const r = await act("control", {
                  action: "task_detail",
                  args: { task_id: result.testId },
                });
                if (r) setResult((v: any) => ({ ...v, testStatus: r.status }));
              }}
            >
              查询试运行：{result.testStatus || "等待结果"}
            </button>
          )}
          {result?.checks?.map((c: any) => (
            <div className="line" key={c.name}>
              {c.name}
              <span>{c.ok ? "已通过" : "未通过"}</span>
            </div>
          ))}
          {result?.error && <p className="error-text">{result.error}</p>}
          <button
            disabled={!result?.checks?.every((c: any) => c.ok)}
            onClick={() => setStep(3)}
          >
            继续
          </button>
        </>
      ) : (
        <>
          <p>连接完成。桌面控制为可选项，启用前请配置后端并检查权限。</p>
          <button
            onClick={() =>
              act(
                "control",
                { action: "desktop", args: { enabled: false } },
                "仅启用命令与文件能力",
              )
            }
          >
            暂不启用桌面控制
          </button>
          <button onClick={() => act("open_main", { route: "desktop" })}>
            配置桌面控制
          </button>
        </>
      )}
    </section>
  );
}
export function SafetyPanel({
  snapshot,
  act,
  disabled,
}: {
  snapshot: Snapshot | null;
  act: Act;
  disabled: boolean;
}) {
  const [draft, setDraft] = useState<any>(null),
    [dirty, setDirty] = useState(false);
  useEffect(() => {
    if (snapshot?.safety && !dirty) setDraft(snapshot.safety);
  }, [snapshot?.safety, dirty]);
  if (!draft)
    return <p className="muted">连接执行器后可设置工作目录和命令确认。</p>;
  const update = (k: string, v: any) => {
    setDirty(true);
    setDraft({ ...draft, [k]: v });
  };
  return (
    <section className="card">
      <h2>工作目录与命令确认</h2>
      <label className="line">
        限制工作目录
        <input
          type="checkbox"
          checked={draft.restrict_paths}
          onChange={(e) => update("restrict_paths", e.target.checked)}
        />
      </label>
      <label className="field">
        <span>允许的绝对目录，每行一个</span>
        <textarea
          value={draft.roots.join("\n")}
          onChange={(e) => update("roots", e.target.value.split("\n"))}
        />
      </label>
      <p className="muted">
        限制命令的起始目录、文件路径及同步目标。Shell
        仍有本机用户权限，这不是系统沙箱。
      </p>
      <label className="field">
        <span>执行命令前</span>
        <select
          value={draft.approval}
          onChange={(e) => update("approval", e.target.value)}
        >
          <option value="direct">直接执行</option>
          <option value="risk">风险命令先确认</option>
          <option value="all">每条都确认</option>
        </select>
      </label>
      <small>
        风险模式为规则筛选，不能识别所有脚本行为。需要逐条控制时请选择“每条都确认”。60
        秒未处理自动拒绝。
      </small>
      <label className="field">
        <span>任务记录保留</span>
        <select
          value={draft.retention_days}
          onChange={(e) => update("retention_days", Number(e.target.value))}
        >
          {[7, 30, 90].map((n) => (
            <option key={n} value={n}>
              {n} 天
            </option>
          ))}
        </select>
      </label>
      <div className="actions">
        <button
          disabled={disabled}
          onClick={async () => {
            const r = await act(
              "control",
              {
                action: "safety",
                args: {
                  ...draft,
                  roots: draft.roots.filter((r: string) => r.trim()),
                },
              },
              "安全设置已保存",
            );
            if (r) setDirty(false);
          }}
        >
          保存安全设置
        </button>
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
    </section>
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
    <section className="card">
      <h2>同步工作区</h2>
      {snapshot?.workspaces?.length ? (
        snapshot.workspaces.map((w) => (
          <div className="line" key={w.root}>
            <div className="grow">
              <b className="mono wrap">{w.root}</b>
              <small>
                {new Date(w.time).toLocaleString()} · {w.status}
                {w.error?.message && ` · ${w.error.message}`}
              </small>
            </div>
            <button
              onClick={() =>
                act("open_workspace", { root: w.root, terminal: false })
              }
            >
              访达
            </button>
            <button
              onClick={() =>
                act("open_workspace", { root: w.root, terminal: true })
              }
            >
              终端
            </button>
          </div>
        ))
      ) : (
        <p className="muted">完成首次同步后在这里显示。</p>
      )}
    </section>
  );
}
export function BackendPanel({
  snapshot,
  act,
  app,
}: {
  snapshot: Snapshot | null;
  act: Act;
  app: AppState | null;
}) {
  const [permissions, setPermissions] = useState<any>(null),
    [input, setInput] = useState<any>(null),
    [text, setText] = useState<string | null>(null),
    [server, setServer] = useState(""),
    [tools, setTools] = useState<any>(null),
    [tool, setTool] = useState(""),
    [args, setArgs] = useState("{}"),
    [observation, setObservation] = useState<any>(null),
    [parseError, setParseError] = useState("");
  const observe = async () => {
    let argumentsValue;
    try {
      argumentsValue = JSON.parse(args);
      setParseError("");
    } catch {
      setParseError("工具参数必须为有效 JSON");
      return;
    }
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
  return (
    <>
      <section className="card">
        <h2>本机条件检查</h2>
        <button
          onClick={async () => {
            setPermissions(await act("permissions"));
            setInput(await act("input_status"));
          }}
        >
          检查系统权限
        </button>
        {permissions && (
          <>
            <div className="line">
              应用辅助功能
              <span>{permissions.accessibility ? "已授权" : "未授权"}</span>
              <button
                onClick={() =>
                  act("open_permission", { kind: "accessibility" })
                }
              >
                系统设置
              </button>
            </div>
            <div className="line">
              应用屏幕录制
              <span>{permissions.screen_recording ? "已授权" : "未授权"}</span>
              <button
                onClick={() => act("open_permission", { kind: "screen" })}
              >
                系统设置
              </button>
            </div>
            <div className="line">
              本机输入检测
              <span>
                {input?.available
                  ? "已启用，输入时让出 30 秒"
                  : "不可用，请授权后重启应用"}
              </span>
              <button onClick={() => act("open_permission", { kind: "input" })}>
                系统设置
              </button>
            </div>
            <div className="line">
              图形登录会话
              <span>{permissions.graphical_session ? "已登录" : "不可用"}</span>
            </div>
            <div className="line">
              防自动休眠
              <span>
                {permissions.keep_awake ? "生效中" : "当前无桌面操作"}
              </span>
            </div>
            <small>
              后端程序有独立的权限。请用下面的实际截图工具核对；应用权限检查不能代替后端实拍。
            </small>
          </>
        )}
      </section>
      <section className="card">
        <h2>后端实拍与状态核对</h2>
        <p className="muted">
          选择后端公布的只读观察或截图工具，填写它要求的参数。结果未知的操作不会自动重放。
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
            disabled={!server}
            onClick={async () =>
              setTools(
                await act("control", { action: "tools", args: { server } }),
              )
            }
          >
            读取工具
          </button>
          <button
            disabled={!server}
            onClick={async () => {
              await act(
                "control",
                { action: "restart_backend", args: { server } },
                "后端会话已失效，请重新读取工具",
              );
              setTools(null);
            }}
          >
            重启后端
          </button>
        </div>
        {tools && (
          <>
            <label className="field">
              <span>工具（{tools.result?.tools?.length || 0}）</span>
              <select value={tool} onChange={(e) => setTool(e.target.value)}>
                <option value="">选择只读截图工具</option>
                {tools.result?.tools?.map((t: any) => (
                  <option key={t.name}>{t.name}</option>
                ))}
              </select>
            </label>
            <pre className="schema">
              {JSON.stringify(
                tools.result?.tools?.find((t: any) => t.name === tool)
                  ?.inputSchema,
                null,
                2,
              )}
            </pre>
            <label className="field">
              <span>工具参数 JSON</span>
              <textarea
                value={args}
                onChange={(e) => setArgs(e.target.value)}
              />
            </label>
            {parseError && <p className="error-text">{parseError}</p>}
            <button disabled={!tool} onClick={observe}>
              执行观察并核对
            </button>
          </>
        )}
        {observation && (
          <>
            <p>状态：{observation.status}</p>
            {observation.error && (
              <p className="error-text">{observation.error.message}</p>
            )}
            <ToolResult value={observation.result} />
          </>
        )}
      </section>
      <section className="card">
        <h2>添加 / 编辑 MCP 后端</h2>
        <button onClick={async () => setText(await act("backend_config"))}>
          读取配置
        </button>
        {text !== null && (
          <>
            <label className="field">
              <span>worker.toml（修改前断开连接）</span>
              <textarea
                className="config-editor"
                value={text || ""}
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
      </section>
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
export function Replay({ tasks, act }: { tasks: Task[]; act: Act }) {
  const [detail, setDetail] = useState<any>(null);
  return (
    <section className="card">
      <h2>最近桌面操作</h2>
      {tasks
        .filter((t) => t.kind === "mcp.call")
        .slice(0, 20)
        .map((t) => (
          <button
            className="line"
            key={t.task_id}
            onClick={async () =>
              setDetail(
                await act("control", {
                  action: "task_detail",
                  args: { task_id: t.task_id },
                }),
              )
            }
          >
            {new Date(t.started_at).toLocaleTimeString()} · {t.arguments.tool} ·{" "}
            {t.status}
          </button>
        ))}
      {detail && (
        <>
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
        </>
      )}
    </section>
  );
}
