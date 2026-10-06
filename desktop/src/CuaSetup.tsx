import { tr } from "./i18n.mjs";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Act } from "./Features";

type DriverStatus = {
  state: "missing" | "ready" | "unsupported" | "broken" | "busy";
  configured?: boolean;
  version?: string;
  detail?: string;
};
const readNative: Act = (command, args) => invoke(command, args);

export function CuaSetup({ act, read = readNative }: { act: Act; read?: Act }) {
  const [status, setStatus] = useState<DriverStatus | null>(null);
  const [pending, setPending] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const locked = useRef(false);
  const readRef = useRef(read);
  readRef.current = read;
  const detect = async () => {
    const value = await readRef.current("cua_status");
    if (!value?.state)
      throw new Error(tr("暂未取得 Cua Driver 检测结果，请重试。"));
    setStatus(value);
  };
  useEffect(() => {
    let disposed = false;
    const refresh = async () => {
      if (locked.current) return;
      try {
        const value = await readRef.current("cua_status");
        if (!disposed && value?.state) setStatus(value);
      } catch (e) {
        if (!disposed) setError(String(e));
      }
    };
    void refresh();
    window.addEventListener("focus", refresh);
    return () => {
      disposed = true;
      window.removeEventListener("focus", refresh);
    };
  }, []);
  const mutate = async (command: string, args?: Record<string, unknown>) => {
    const result = await act(command, args);
    if (result === undefined || result === false)
      throw new Error(tr("操作未完成，请查看错误提示后重试。"));
    return result;
  };
  const perform = async (label: string, operation: () => Promise<void>) => {
    if (locked.current) return;
    locked.current = true;
    setPending(label);
    setError("");
    setNotice("");
    try {
      await operation();
    } catch (e) {
      setError(String(e));
    } finally {
      locked.current = false;
      setPending("");
    }
  };
  const configure = async () => {
    // Re-read immediately before stopping: never trust a stale UI snapshot or
    // interrupt work that started after the button was rendered.
    const state = await readRef.current("app_state");
    if (typeof state?.worker_running !== "boolean" || state.worker_starting)
      throw new Error(tr("执行器状态尚未就绪，请稍后重试。"));
    if (state.worker_running) await mutate("stop_worker", { onlyIfIdle: true });
    try {
      await mutate("configure_cua_driver");
    } finally {
      if (state.worker_running) await mutate("start_worker");
    }
    await detect();
    setNotice(tr("Cua Driver 已接入。授权后请在下方通过后端实拍验证截图。"));
  };
  return (
    <section
      className="card cua-setup"
      aria-label={tr("Cua Driver 安装与授权")}
    >
      <div className="feature-line">
        <div className="grow">
          <b>Cua Driver</b>
          <small>
            {status?.state === "ready"
              ? `${status.version || tr("已安装")} · ${status.configured ? tr("已接入") : tr("尚未接入 Macrun")}`
              : status?.detail || tr("正在检测本机安装…")}
          </small>
        </div>
        <button
          disabled={!!pending}
          onClick={() => perform(tr("正在检测…"), detect)}
        >
          {tr("重新检测")}
        </button>
      </div>
      <p className="muted">
        {tr(
          "桌面控制可选。安装将从 cua.ai 下载并运行官方 Cua Driver 安装程序，需要 macOS 14+。",
        )}
      </p>
      <div className="actions">
        {(status?.state === "missing" || status?.state === "broken") && (
          <button
            disabled={!!pending}
            onClick={() =>
              perform(tr("正在下载并安装 Cua Driver…"), async () => {
                await mutate("install_cua_driver");
                await detect();
              })
            }
          >
            {tr("一键安装 Cua Driver")}
          </button>
        )}
        {status?.state === "ready" && (
          <>
            {!status.configured && (
              <button
                disabled={!!pending}
                onClick={() => perform(tr("正在接入 Cua Driver…"), configure)}
              >
                {tr("接入 Macrun（空闲时重连）")}
              </button>
            )}
            <button
              disabled={!!pending}
              onClick={() =>
                perform(tr("等待 CuaDriver 授权与截图验证…"), async () => {
                  await mutate("grant_cua_permissions");
                  await detect();
                  setNotice(
                    tr(
                      "CuaDriver 授权检查已完成。请继续通过 Macrun 后端实拍验证。",
                    ),
                  );
                })
              }
            >
              {tr("授权 CuaDriver 截图与控制")}
            </button>
          </>
        )}
      </div>
      <p className="muted">
        {tr(
          "系统设置中请启用 CuaDriver 的「辅助功能」和「屏幕与系统音频录制」。按系统提示重新打开 CuaDriver 后，可再次点击授权检查；Macrun 自身的权限不代表后端已授权。",
        )}
      </p>
      {pending && <p role="status">{pending}</p>}
      {notice && <p role="status">{notice}</p>}
      {error && (
        <p className="error-text" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
