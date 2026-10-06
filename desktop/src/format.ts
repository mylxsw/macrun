import { tr, getLocale } from "./i18n.mjs";
import type { SavedConnection, Snapshot, Task } from "./types";

/** "13:06" in 24-hour local time. */
export const clock = (ms: number) =>
  new Date(ms).toLocaleTimeString(getLocale(), {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
  });

/** "5:56", or "1:02:03" past an hour. */
export function stopwatch(ms: number) {
  const sec = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(sec / 3600),
    m = Math.floor((sec % 3600) / 60),
    s = String(sec % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

export const taskDuration = (t: Task, now = Date.now()) =>
  t.status === "unknown" ? "—" : stopwatch((t.ended_at || now) - t.started_at);

/** "18 分钟", "2 小时" for how long something has been going. */
export function span(ms: number) {
  const min = Math.max(0, Math.floor(ms / 60000));
  if (min < 1) return tr("不到 1 分钟");
  if (min < 60) return tr("{0} 分钟", min);
  const h = Math.floor(min / 60);
  return h < 24 ? tr("{0} 小时", h) : tr("{0} 天", Math.floor(h / 24));
}

/** "今天", "昨天" or "10月3日" for list group headings. */
export function dayLabel(ms: number, now = Date.now()) {
  const day = (v: number) => new Date(v).toDateString();
  if (day(ms) === day(now)) return tr("今天");
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (day(ms) === yesterday.toDateString()) return tr("昨天");
  return new Date(ms).toLocaleDateString(getLocale(), {
    month: "short",
    day: "numeric",
  });
}

/** Last non-empty output line, for a one-line progress hint. */
export const lastLine = (text = "") =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .at(-1) || "";

export type ServerView = {
  id: string;
  label: string;
  address: string;
  connection?: Snapshot["connection"];
};
// Paired servers are saved under their address. Screens are often shared, so
// lists show a stable label and keep the address behind an explicit action.
const looksLikeAddress = (name: string) =>
  /^[\w.-]+:\d+$/.test(name) || /^\[?[\d.:a-f]+\]?(:\d+)?$/i.test(name);
export function serverViews(
  settings: { server: string; connections?: SavedConnection[] } | undefined,
  snapshot: Snapshot | null,
  live: boolean,
): ServerView[] {
  const saved = [
    ...(settings?.server
      ? [{ id: "primary", name: settings.server, server: settings.server }]
      : []),
    ...(settings?.connections || []),
  ];
  const fromSnapshot = snapshot?.connections?.length
    ? snapshot.connections.map((c) => ({
        id: c.id,
        name: c.name,
        server: c.connection.server,
      }))
    : [];
  const rows = saved.length ? saved : fromSnapshot;
  return rows.map((c, i) => {
    const connection = live
      ? snapshot?.connections?.find((v) => v.id === c.id)?.connection ||
        (c.id === "primary" || rows.length === 1
          ? snapshot?.connection
          : undefined)
      : undefined;
    return {
      id: c.id,
      label: looksLikeAddress(c.name)
        ? i === 0
          ? tr("主服务器")
          : tr("服务器 {0}", i + 1)
        : c.name,
      address: c.server,
      connection,
    };
  });
}
