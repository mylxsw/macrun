import { tr } from "./i18n.mjs";
import { useState } from "react";
import {
  Activity,
  Check,
  ChevronRight,
  Folder,
  LoaderCircle,
  MousePointer2,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";
import { Approvals, type Act } from "./Features";
import {
  active,
  agentExit,
  attentionCount,
  explainTask,
  groupSessions,
  needsAttention,
  problemLabel,
  projectOf,
  stepSummary,
  taskHeadline,
} from "./model.mjs";
import {
  clock,
  lastLine,
  span,
  stopwatch,
  taskDuration,
  type ServerView,
} from "./format";
import { healthLine, type Health } from "./health";
import type { AppState, Snapshot, Task } from "./types";

type Session = ReturnType<typeof groupSessions>[number];
const DISMISSED = "macrun-dismissed-problems";
const RECENT_PROBLEM_MS = 60 * 60 * 1000;

/** Small status mark used in step lists: neutral for the agent's own exits. */
export function StepMark({ task }: { task: Task }) {
  if (task.status === "awaiting_approval")
    return <span className="v4-exit warn">{tr("待确认")}</span>;
  if (active(task))
    return (
      <LoaderCircle size={13} className="v4-spin" aria-label={tr("进行中")} />
    );
  if (agentExit(task))
    return (
      <span
        className="v4-exit"
        title={tr("命令以非零退出码结束，由 Agent 处理")}
      >
        {tr("退出 {0}", task.result?.exit_code)}
      </span>
    );
  if (needsAttention(task))
    return (
      <TriangleAlert
        size={13}
        className="v4-warn-icon"
        aria-label={problemLabel(task)}
      />
    );
  if (task.status === "succeeded")
    return <Check size={13} className="v4-faint" aria-label={tr("成功")} />;
  return <span className="v4-exit">{problemLabel(task)}</span>;
}

function desktopArgs(t: Task) {
  const args = JSON.stringify(t.arguments.arguments ?? {});
  return args.length > 160 ? `${args.slice(0, 160)}…` : args;
}

function ProjectCard({
  session,
  server,
  onTask,
  onCancel,
  onOpenDirectory,
  cancelDisabled,
  cancelPending,
}: {
  session: Session;
  server?: string;
  onTask: (t: Task) => void;
  onCancel: (t: Task) => void;
  onOpenDirectory: (t: Task) => void;
  cancelDisabled: (t: Task) => boolean;
  cancelPending: (t: Task) => boolean;
}) {
  const running = session.tasks.filter(
    (t: Task) => active(t) && t.status !== "awaiting_approval",
  );
  const current: Task = running[0] || session.tasks[0];
  const desktop = session.project.desktop;
  const sync = current.kind === "sync";
  const summary = stepSummary(current.arguments.command || "");
  const tail = lastLine(current.output_tail);
  // A waiting request is shown once, in "需要你", not again as a step.
  const steps = session.tasks
    .filter((t: Task) => t !== current && t.status !== "awaiting_approval")
    .slice(0, 3);
  const directory = current.arguments.cwd || current.arguments.remote_root;
  return (
    <article
      className={`v4-project ${desktop ? "desktop" : ""}`}
      aria-label={tr("{0} 进行中", session.project.name)}
    >
      <header>
        {desktop ? (
          <MousePointer2 size={15} className="v4-act" />
        ) : (
          <Folder size={15} className="v4-faint" />
        )}
        <b title={session.project.root}>{session.project.name}</b>
        {session.project.tag && (
          <span className="v4-chip">{session.project.tag}</span>
        )}
        {desktop && <span className="v4-chip act">{tr("操作中")}</span>}
        {server && <span className="v4-chip">{server}</span>}
        <span className="grow" />
        <small>
          {tr(
            "已工作 {0} · {1} 个任务",
            span(Date.now() - session.start),
            session.tasks.length,
          )}
        </small>
      </header>
      <div className="v4-now">
        {desktop ? (
          <MousePointer2 size={14} className="v4-act v4-now-mark" />
        ) : (
          <LoaderCircle size={14} className="v4-spin v4-now-mark" />
        )}
        <div className="grow">
          <div className="row">
            <h3 className="mono v4-step" title={current.arguments.command}>
              {desktop
                ? taskHeadline(current)
                : sync
                  ? tr("同步文件")
                  : summary.text}
            </h3>
            {summary.helpers > 0 && (
              <span
                className="v4-chip"
                title={tr(
                  "ls、cp、grep、tail 等辅助命令已折叠，详情中可看完整命令",
                )}
              >
                {tr("+ {0} 个辅助命令", summary.helpers)}
              </span>
            )}
            <span className="grow" />
            <span className="mono v4-muted">{taskDuration(current)}</span>
          </div>
          {desktop ? (
            <small className="mono v4-tail" title={desktopArgs(current)}>
              {current.arguments.server} · {desktopArgs(current)}
            </small>
          ) : sync ? (
            <small className="v4-tail">
              {tr(
                "{0} · 同步只更新文件，不会自动触发构建",
                current.progress
                  ? tr(
                      "{0} 个文件中已收到 {1} 个 · {2} KB",
                      current.progress.total,
                      current.progress.received,
                      Math.round(current.progress.bytes / 1024),
                    )
                  : tr("正在比对文件"),
              )}
            </small>
          ) : (
            tail && <small className="mono v4-tail">{tail}</small>
          )}
          <div className="v4-now-actions">
            <button className="v4-link" onClick={() => onTask(current)}>
              {tr("查看任务详情")}
            </button>
            {directory && !desktop && (
              <button
                className="v4-link"
                onClick={() => onOpenDirectory(current)}
              >
                {tr("在终端打开目录")}
              </button>
            )}
            {active(current) && (
              <button
                className="v4-link danger"
                disabled={cancelDisabled(current)}
                aria-busy={cancelPending(current)}
                onClick={() => onCancel(current)}
              >
                {desktop ? tr("让 Agent 停下") : tr("取消任务")}
              </button>
            )}
          </div>
        </div>
      </div>
      {steps.length > 0 && (
        <div className="v4-steps">
          {steps.map((t: Task) => (
            <button key={t.task_id} onClick={() => onTask(t)}>
              <span className="v4-time">{clock(t.started_at)}</span>
              <span className="v4-mark">
                <StepMark task={t} />
              </span>
              <span
                className={`grow ellipsis ${t.kind === "sync" ? "" : "mono"}`}
              >
                {taskHeadline(t)}
              </span>
              <span className="v4-time right">{taskDuration(t)}</span>
            </button>
          ))}
        </div>
      )}
    </article>
  );
}

export function Overview({
  snapshot,
  app,
  available,
  act,
  isPending,
  control,
  onTask,
  onPage,
  health,
  servers,
}: {
  snapshot: Snapshot | null;
  app: AppState | null;
  available: boolean;
  act: Act;
  isPending: (action: string, id?: string) => boolean;
  control: (action: string, args?: Record<string, unknown>) => unknown;
  onTask: (t: Task) => void;
  onPage: (page: string) => void;
  health: Health;
  servers: ServerView[];
}) {
  const tasks = snapshot?.tasks || [];
  const sessions = groupSessions(tasks, snapshot?.workspaces || []);
  const approvals = tasks.filter((t) => t.status === "awaiting_approval");
  // Code projects lead; a desktop operation is usually one step of them.
  const working = sessions
    .filter((s: Session) =>
      s.tasks.some((t: Task) => active(t) && t.status !== "awaiting_approval"),
    )
    .sort(
      (a: Session, b: Session) =>
        Number(a.project.desktop) - Number(b.project.desktop),
    );
  const finished = sessions.filter((s: Session) => !s.active).slice(0, 4);
  const paused = !!snapshot?.policy.paused;
  const multi = servers.length > 1;
  const serverLabel = (id?: string) =>
    multi ? servers.find((s) => s.id === id)?.label : undefined;
  const [dismissed, setDismissed] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(DISMISSED) || "[]");
    } catch {
      return [];
    }
  });
  const dismiss = (id: string) => {
    const next = [...dismissed, id].slice(-200);
    setDismissed(next);
    try {
      localStorage.setItem(DISMISSED, JSON.stringify(next));
    } catch {}
  };
  const now = Date.now();
  const issues = tasks
    .filter(
      (t) =>
        needsAttention(t) &&
        now - (t.ended_at || t.started_at) < RECENT_PROBLEM_MS &&
        !dismissed.includes(t.task_id) &&
        // Rejections were the person's own decision.
        t.error?.code !== "approval_rejected",
    )
    .slice(0, 3);
  const summary = snapshot?.today_summary || {};
  const problems = attentionCount(summary);
  const today = new Date().toDateString();
  const todayTasks = tasks.filter(
    (t) => new Date(t.started_at).toDateString() === today,
  );
  const todayProjects = new Set(
    groupSessions(todayTasks, snapshot?.workspaces || []).map(
      (s: Session) => s.project.key,
    ),
  ).size;
  const desktopToday = todayTasks.filter((t) => t.kind === "mcp.call").length;
  const codeProjects = working.filter((s: Session) => !s.project.desktop);
  const title = !available
    ? app?.worker_starting
      ? tr("正在启动执行器")
      : snapshot
        ? tr("执行器暂时不可用")
        : tr("执行器未运行")
    : !health.commands
      ? tr("未连接服务器")
      : working.length
        ? codeProjects.length === 0
          ? tr("Agent 正在操作桌面")
          : codeProjects.length === 1
            ? tr("Agent 正在处理 {0}", codeProjects[0].project.name)
            : tr("Agent 正在 {0} 个项目上工作", codeProjects.length)
        : approvals.length
          ? tr("有 {0} 个请求等你确认", approvals.length)
          : paused
            ? tr("已暂停接收新任务")
            : tr("就绪，等待 Agent");
  const subtitle = [
    snapshot ? tr("今天 {0} 个任务", summary.total || 0) : "",
    health.servers > 1
      ? tr("{0}/{1} 台服务器在线", health.online, health.servers)
      : health.commands
        ? tr("服务器已连接")
        : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const missing = health.desktop.filter(
    (c) => c.ok === false && c.key !== "enabled",
  );
  return (
    <>
      <section className="v4-hero">
        <div className="grow">
          <h2>{title}</h2>
          {subtitle && <p>{subtitle}</p>}
        </div>
        <button className="v4-health" onClick={() => onPage("desktop")}>
          {health.commands && !health.desktopMissing ? (
            <Check size={13} className="v4-ok" />
          ) : (
            <TriangleAlert size={13} className="v4-warn-icon" />
          )}
          {healthLine(health)}
          <ChevronRight size={13} />
        </button>
      </section>
      <div className="v4-cols">
        <div>
          {(approvals.length > 0 || issues.length > 0 || paused) && (
            <section aria-label={tr("需要你")}>
              <div className="v4-sh">
                <h2>{tr("需要你")}</h2>
              </div>
              <Approvals
                tasks={tasks}
                act={act}
                disabled={!available}
                workspaces={snapshot?.workspaces}
                serverLabel={(t) => serverLabel(t.connection_id) || ""}
                pending={(id) => isPending("approve", id)}
              />
              {paused && (
                <div className="v4-issue">
                  <span className="v4-ico">⏸</span>
                  <div className="grow">
                    <b>{tr("已暂停接收新任务")}</b>
                    <p>{tr("正在运行的任务继续完成；新请求返回 busy。")}</p>
                  </div>
                  <button
                    disabled={!available || isPending("pause")}
                    onClick={() => control("pause", { paused: false })}
                  >
                    {tr("恢复接收")}
                  </button>
                </div>
              )}
              {issues.map((t) => (
                <div className="v4-issue" key={t.task_id}>
                  <TriangleAlert size={15} className="v4-warn-icon" />
                  <div className="grow">
                    <b>
                      {projectOf(t, snapshot?.workspaces || []).name} ·{" "}
                      {problemLabel(t)}
                    </b>
                    <p>{explainTask(t)?.what}</p>
                  </div>
                  <button onClick={() => onTask(t)}>{tr("查看")}</button>
                  <button
                    className="v4-link"
                    onClick={() => dismiss(t.task_id)}
                  >
                    {tr("知道了")}
                  </button>
                </div>
              ))}
            </section>
          )}
          <section aria-label={tr("正在进行")}>
            <div className="v4-sh">
              <h2>{tr("进行中")}</h2>
              <small>{tr("按项目归组，每个项目只突出正在做的一步")}</small>
            </div>
            {working.map((s: Session) => (
              <ProjectCard
                key={s.id}
                session={s}
                server={serverLabel(s.connection_id)}
                onTask={onTask}
                onCancel={(t) => control("cancel", { task_id: t.task_id })}
                onOpenDirectory={(t) =>
                  act("open_workspace", {
                    root: t.arguments.cwd || t.arguments.remote_root,
                    terminal: true,
                    taskId: t.task_id,
                  })
                }
                cancelDisabled={(t) =>
                  !available || isPending("cancel", t.task_id)
                }
                cancelPending={(t) => isPending("cancel", t.task_id)}
              />
            ))}
            {!working.length && (
              <div className="v4-empty">
                {app?.worker_starting ? (
                  <RefreshCw size={24} className="loading-icon" />
                ) : (
                  <Activity size={24} />
                )}
                <h3>
                  {available && approvals.length
                    ? tr("请先处理上方的确认请求")
                    : available
                      ? tr("等待 Agent 发起任务")
                      : app?.worker_starting
                        ? tr("正在启动执行器")
                        : tr("连接你的服务器")}
                </h3>
                <p>
                  {available
                    ? tr(
                        "Agent 开始工作后，这里按项目显示它正在做的事和最近几步。",
                      )
                    : app?.worker_starting
                      ? tr(
                          "如有钥匙串授权窗口，请完成系统确认。连接成功后，任务会自动显示。",
                        )
                      : tr("启动执行器后，在这里查看这台电脑上的执行情况。")}
                </p>
                {!available && !app?.worker_starting && (
                  <button className="primary" onClick={() => onPage("desktop")}>
                    {tr("检查连接")}
                  </button>
                )}
              </div>
            )}
          </section>
        </div>
        <aside className="v4-side">
          <div className="v4-sh">
            <h2>{tr("今天")}</h2>
            <button className="v4-link" onClick={() => onPage("tasks")}>
              {tr("查看活动 ›")}
            </button>
          </div>
          <div className="v4-stats">
            <div>
              <b>{summary.total || 0}</b>
              <span>{tr("个任务")}</span>
            </div>
            <div>
              <b>{todayProjects}</b>
              <span>{tr("个项目")}</span>
            </div>
            <div>
              <b className={problems ? "v4-warn" : "v4-faint"}>{problems}</b>
              <span>{tr("个 Macrun 问题")}</span>
            </div>
            <div>
              <b>{desktopToday}</b>
              <span>{tr("次桌面操作")}</span>
            </div>
          </div>
          {(summary.exited || 0) > 0 && (
            <p className="v4-note">
              {tr(
                "另有 {0} 条命令退出码非零，由 Agent 自行处理，不计入问题。",
                summary.exited,
              )}
            </p>
          )}
          <div className="v4-sh">
            <h2>{tr("最近完成")}</h2>
          </div>
          <div className="v4-list">
            {finished.map((s: Session) => {
              const problem = s.tasks.find(needsAttention);
              return (
                <button key={s.id} onClick={() => onTask(s.tasks[0])}>
                  {problem ? (
                    <TriangleAlert size={13} className="v4-warn-icon" />
                  ) : (
                    <Check size={13} className="v4-ok" />
                  )}
                  <span className="grow ellipsis">
                    {s.project.name}
                    <span className="v4-faint">
                      {" "}
                      ·{" "}
                      {problem
                        ? problemLabel(problem)
                        : s.project.tag || tr("{0} 个任务", s.tasks.length)}
                    </span>
                  </span>
                  <span className="v4-time">{clock(s.end)}</span>
                </button>
              );
            })}
            {!finished.length && (
              <p className="v4-note">{tr("任务结束后显示在这里。")}</p>
            )}
          </div>
          <div className="v4-sh">
            <h2>{tr("这台 Mac")}</h2>
            <button className="v4-link" onClick={() => onPage("desktop")}>
              {tr("本机 ›")}
            </button>
          </div>
          <div className="v4-list static">
            <div>
              {health.commands ? (
                <Check size={13} className="v4-ok" />
              ) : (
                <TriangleAlert size={13} className="v4-warn-icon" />
              )}
              <span className="grow">
                {health.servers > 1
                  ? tr("{0}/{1} 台服务器在线", health.online, health.servers)
                  : health.commands
                    ? tr("服务器已连接")
                    : tr("服务器未连接")}
              </span>
            </div>
            <div>
              {health.keepAwake ? (
                <Check size={13} className="v4-ok" />
              ) : (
                <TriangleAlert size={13} className="v4-warn-icon" />
              )}
              <span className="grow">
                {health.keepAwake
                  ? tr("防止自动休眠已开启")
                  : tr("Mac 可能自动休眠")}
              </span>
            </div>
            {health.desktopEnabled ? (
              missing.slice(0, 2).map((c) => (
                <div key={c.key}>
                  <TriangleAlert size={13} className="v4-warn-icon" />
                  <span className="grow">{c.detail}</span>
                </div>
              ))
            ) : (
              <div>
                <span className="v4-ico small">—</span>
                <span className="grow">{tr("桌面控制已关闭")}</span>
              </div>
            )}
          </div>
          {(snapshot?.total_tasks || 0) > 200 && (
            <p className="v4-note">
              {tr("完整历史在活动页，可按项目、命令和目录查找。")}
            </p>
          )}
        </aside>
      </div>
    </>
  );
}
