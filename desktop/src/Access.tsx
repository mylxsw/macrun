import { tr } from "./i18n.mjs";
import {
  AllowRules,
  DesktopTiers,
  EnvProtection,
  SafetyPanel,
  type Act,
} from "./Features";
import { presetOf, presets } from "./model.mjs";
import type { Snapshot } from "./types";

/** One page for everything the remote agent is allowed to do on this Mac. */
export function Access({
  snapshot,
  act,
  available,
  pending,
}: {
  snapshot: Snapshot | null;
  act: Act;
  available: boolean;
  pending: boolean;
}) {
  const safety = snapshot?.safety;
  const current = presetOf(safety);
  const disabled = !available || pending;
  const apply = (key: keyof typeof presets) => {
    if (!safety?.desktop) return;
    const p = presets[key];
    act(
      "control",
      {
        action: "safety",
        args: { ...safety, approval: p.approval, desktop: { ...p.desktop } },
      },
      tr("已切换到“{0}”", p.label),
    );
  };
  return (
    <div className="v4-access">
      <div className="v4-presets" role="radiogroup" aria-label={tr("权限预设")}>
        {(Object.keys(presets) as (keyof typeof presets)[]).map((key) => (
          <button
            key={key}
            role="radio"
            aria-checked={current === key}
            className={`v4-preset ${current === key ? "on" : ""}`}
            disabled={disabled || !safety?.desktop}
            onClick={() => current !== key && apply(key)}
          >
            <span className="v4-radio" aria-hidden />
            <span className="grow">
              <b>
                {presets[key].label}
                {current === key && (
                  <span className="v4-chip">{tr("当前")}</span>
                )}
              </b>
              <small>{presets[key].detail}</small>
            </span>
          </button>
        ))}
      </div>
      <p className="v4-note">
        {tr(
          "{0}这些规则在 Macrun 内生效；命令仍以你的用户身份运行，不是沙箱。",
          current === "custom"
            ? tr(
                "当前为自定义设置。选择上面的预设可一次改好命令和桌面规则；目录限制不受预设影响。",
              )
            : tr("修改下面任意一项后显示为“自定义”。"),
        )}
      </p>
      <div className="v4-cols access">
        <div className="v4-group">
          <div className="v4-group-title">{tr("命令与文件")}</div>
          <SafetyPanel
            snapshot={snapshot}
            act={act}
            disabled={disabled}
            includeRetention={false}
            includeExtras={false}
          />
          <div className="v4-group-title split">
            {tr("桌面 ")}
            <span>{tr("· 需要桌面后端，见“本机”")}</span>
          </div>
          <DesktopTiers snapshot={snapshot} act={act} disabled={disabled} />
        </div>
        <aside className="v4-side">
          <div className="v4-sh">
            <h2>{tr("你临时放行的")}</h2>
            <small>{tr("执行器重启后失效")}</small>
          </div>
          <AllowRules
            rules={snapshot?.allow_rules || []}
            act={act}
            disabled={disabled}
          />
          <div className="v4-sh">
            <h2>{tr("保护")}</h2>
          </div>
          <div className="v4-plain-list">
            <EnvProtection />
            <div className="v4-perm">
              <div className="grow">
                <b>{tr("无人确认时自动拒绝")}</b>
                <p>{tr("60 秒内没有处理，Agent 会收到 approval_expired。")}</p>
              </div>
              <span className="v4-chip">{tr("60 秒")}</span>
            </div>
            <div className="v4-perm">
              <div className="grow">
                <b>{tr("紧急停止快捷键")}</b>
                <p>{tr("取消所有任务、暂停接收新任务并关闭桌面控制。")}</p>
              </div>
              <kbd>⌃ ⌥ ⌘ .</kbd>
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}
