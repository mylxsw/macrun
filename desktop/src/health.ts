import { tr } from "./i18n.mjs";
import type { AppState, Snapshot } from "./types";
import type { DriverStatus } from "./CuaSetup";
import { clock } from "./format";

export type DesktopCheck = {
  key: "backend" | "verified" | "session" | "enabled";
  title: string;
  detail: string;
  action?: string;
  /** null when no current result is available. */
  ok: boolean | null;
};

function backendCheck(
  backend: Snapshot["backends"][number],
  tasks: Snapshot["tasks"],
  driver?: DriverStatus | null,
): DesktopCheck {
  const name = backend.backend_name || backend.name;
  if (
    driver?.state === "ready" &&
    driver.permission_backends?.[name] === backend.command
  ) {
    const grants = driver.permissions;
    const denied = [
      ...(grants?.accessibility === false ? [tr("辅助功能")] : []),
      ...(grants?.screen_recording === false ? [tr("屏幕录制")] : []),
    ];
    const granted =
      grants?.accessibility === true && grants.screen_recording === true;
    return {
      key: "verified",
      title: tr("CuaDriver 系统权限"),
      ok: denied.length ? false : granted || null,
      detail: denied.length
        ? tr("请为 CuaDriver 开启{0}权限", denied.join(tr("、")))
        : granted
          ? tr("辅助功能和屏幕录制均已授权")
          : tr("暂时无法读取权限，Macrun 会自动重新检测。"),
      action: tr("查看授权"),
    };
  }
  const call = tasks.find(
    (t) =>
      t.kind === "mcp.call" &&
      t.arguments.server === name &&
      (!backend.connection_id || t.connection_id === backend.connection_id) &&
      ["succeeded", "failed", "unknown", "timed_out"].includes(t.status),
  );
  return {
    key: "verified",
    title: tr("最近桌面调用"),
    ok: call ? call.status === "succeeded" : null,
    detail: !call
      ? tr("暂无调用记录，首次使用时可查看结果。")
      : call.status === "succeeded"
        ? tr(
            "最近一次桌面调用 {0} 成功",
            clock(call.ended_at || call.started_at),
          )
        : tr("最近的桌面调用没有成功，请实拍核对后端的辅助功能和屏幕录制权限"),
    action: tr("实拍核对"),
  };
}

export function desktopChecks(
  snapshot: Snapshot | null,
  permissions: { graphical_session?: boolean } | null,
  driver?: DriverStatus | null,
): DesktopCheck[] {
  const backend = snapshot?.backends[0];
  const checks: DesktopCheck[] = [
    {
      key: "backend",
      title: tr("已配置桌面后端"),
      ok: !!backend,
      detail: backend
        ? `${backend.display_name || backend.name} · ${
            backend.tool_count != null
              ? tr("已读取 {0} 个工具", backend.tool_count)
              : backend.state === "not_started"
                ? tr("首次调用时启动")
                : tr("已启动")
          }`
        : tr("需要一个本机 computer-use 后端，例如 CuaDriver"),
    },
  ];
  if (snapshot && backend)
    checks.push(backendCheck(backend, snapshot.tasks, driver));
  checks.push(
    {
      key: "session",
      title: tr("有人登录图形界面"),
      ok: permissions ? !!permissions.graphical_session : null,
      detail: !permissions
        ? tr("正在检查…")
        : permissions.graphical_session
          ? tr("当前用户已登录")
          : tr("桌面操作需要图形登录会话"),
    },
    {
      key: "enabled",
      title: tr("允许 Agent 操作桌面"),
      ok: !!snapshot?.policy.desktop_enabled,
      detail: snapshot?.policy.desktop_enabled
        ? tr("总开关；“停止全部”会关闭它")
        : tr("已关闭，Agent 的桌面请求会被拒绝"),
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
  desktopUnknown: boolean;
  desktopEnabled: boolean;
  keepAwake: boolean;
};

export function health(
  snapshot: Snapshot | null,
  app: AppState | null,
  available: boolean,
  permissions: { graphical_session?: boolean } | null,
  driver?: DriverStatus | null,
): Health {
  const connections = snapshot?.connections?.length
    ? snapshot.connections.map((c) => c.connection)
    : snapshot
      ? [snapshot.connection]
      : [];
  const online = available
    ? connections.filter((c) => c.state === "connected").length
    : 0;
  const desktop = desktopChecks(snapshot, permissions, driver);
  return {
    commands: available && online > 0,
    online,
    servers: Math.max(connections.length, app?.settings.server ? 1 : 0),
    desktop,
    desktopMissing: desktop.filter((c) => c.ok === false && c.key !== "enabled")
      .length,
    desktopUnknown: desktop.some((c) => c.ok === null),
    desktopEnabled: !!snapshot?.policy.desktop_enabled,
    keepAwake: !!app?.preferences.keep_awake,
  };
}

/** One-line summary for the overview pill. */
export function healthLine(h: Health) {
  if (!h.commands) return tr("命令不可用");
  if (!h.desktopEnabled) return tr("命令可用 · 桌面已关闭");
  if (!h.desktopMissing)
    return h.desktopUnknown ? tr("命令可用 · 桌面状态待确认") : tr("一切就绪");
  return tr("命令可用 · 桌面需要 {0} 步", h.desktopMissing);
}
