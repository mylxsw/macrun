import type { AppState, Snapshot } from "./types";
import { clock } from "./format";

export type DesktopCheck = {
  key: "backend" | "verified" | "session" | "enabled";
  title: string;
  detail: string;
  /** null while the state is still being read. */
  ok: boolean | null;
};

/**
 * What desktop control still needs, in the order a person would fix it.
 * Only facts Macrun can observe are claimed: the backend's own macOS
 * permissions are proven by a successful desktop call, not by Macrun's.
 */
export function desktopChecks(
  snapshot: Snapshot | null,
  permissions: { graphical_session?: boolean } | null,
): DesktopCheck[] {
  const backend = snapshot?.backends[0];
  const calls = (snapshot?.tasks || []).filter((t) => t.kind === "mcp.call");
  const lastGood = calls.find((t) => t.status === "succeeded");
  const lastBad = calls.find((t) =>
    ["failed", "unknown", "timed_out"].includes(t.status),
  );
  const checks: DesktopCheck[] = [
    {
      key: "backend",
      title: "已配置桌面后端",
      ok: !!backend,
      detail: backend
        ? `${backend.display_name || backend.name} · ${
            backend.tool_count != null
              ? `已读取 ${backend.tool_count} 个工具`
              : backend.state === "not_started"
                ? "首次调用时启动"
                : "已启动"
          }`
        : "需要一个本机 computer-use 后端，例如 CuaDriver",
    },
  ];
  if (backend)
    checks.push({
      key: "verified",
      title: "后端能截图和操作",
      ok: !!lastGood,
      detail: lastGood
        ? `最近一次桌面调用 ${clock(lastGood.ended_at || lastGood.started_at)} 成功`
        : lastBad
          ? "最近的桌面调用没有成功，请实拍核对后端的辅助功能和屏幕录制权限"
          : "还没有成功的桌面调用；实拍一次确认后端已获系统授权",
    });
  checks.push(
    {
      key: "session",
      title: "有人登录图形界面",
      ok: permissions ? !!permissions.graphical_session : null,
      detail: !permissions
        ? "正在检查…"
        : permissions.graphical_session
          ? "当前用户已登录"
          : "桌面操作需要图形登录会话",
    },
    {
      key: "enabled",
      title: "允许 Agent 操作桌面",
      ok: !!snapshot?.policy.desktop_enabled,
      detail: snapshot?.policy.desktop_enabled
        ? "总开关；“停止全部”会关闭它"
        : "已关闭，Agent 的桌面请求会被拒绝",
    },
  );
  return checks;
}

export type Health = {
  commands: boolean;
  online: number;
  servers: number;
  desktop: DesktopCheck[];
  /** Steps still missing; switching desktop control off is a choice, not one. */
  desktopMissing: number;
  desktopEnabled: boolean;
  keepAwake: boolean;
};

export function health(
  snapshot: Snapshot | null,
  app: AppState | null,
  available: boolean,
  permissions: { graphical_session?: boolean } | null,
): Health {
  const connections = snapshot?.connections?.length
    ? snapshot.connections.map((c) => c.connection)
    : snapshot
      ? [snapshot.connection]
      : [];
  const online = available
    ? connections.filter((c) => c.state === "connected").length
    : 0;
  const desktop = desktopChecks(snapshot, permissions);
  return {
    commands: available && online > 0,
    online,
    servers: Math.max(connections.length, app?.settings.server ? 1 : 0),
    desktop,
    desktopMissing: desktop.filter((c) => c.ok === false && c.key !== "enabled")
      .length,
    desktopEnabled: !!snapshot?.policy.desktop_enabled,
    keepAwake: !!app?.preferences.keep_awake,
  };
}

/** One-line summary for the overview pill. */
export function healthLine(h: Health) {
  if (!h.commands) return "命令不可用";
  if (!h.desktopEnabled) return "命令可用 · 桌面已关闭";
  if (!h.desktopMissing) return "一切就绪";
  return `命令可用 · 桌面需要 ${h.desktopMissing} 步`;
}
