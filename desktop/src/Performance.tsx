import React, { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Metrics, Task, AppState } from "./types";
import "./performance.css";

type MetricTask = Task & {
  metrics?: Metrics;
  server_metrics?: Metrics;
  version?: string;
};
type Cursor = { started_at: number; connection_id: string; task_id: string };
export type Dashboard = {
  as_of: number;
  total: number;
  measured: number;
  counts: Record<string, number>;
  success_denominator: number;
  latency: {
    n: number;
    p50_ms: number | null;
    p95_ms: number | null;
    p99_ms: number | null;
  };
  bytes: Record<string, number | string>;
  phases_ms: Record<string, number>;
  operations: MetricTask[];
  next_cursor: Cursor | null;
  series: {
    time: number;
    count: number;
    p50_ms: number | null;
    p95_ms: number | null;
    payload_bytes: number | string;
  }[];
  projects: Record<string, string>;
  errors: { connection_id: string; message: string }[];
};
export function bytes(value?: number | string | null): string {
  if (value === undefined || value === null) return "未采集";
  const n = Number(value);
  if (!Number.isFinite(n) || n < 0) return "未采集";
  if (n < 1024) return `${n} B`;
  const power = Math.min(4, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** power).toFixed(2)} ${["B", "KiB", "MiB", "GiB", "TiB"][power]}`;
}
export function millis(value?: number | null): string {
  if (value === undefined || value === null || !Number.isFinite(value))
    return "未采集";
  return value < 1000
    ? `${value.toFixed(1)} ms`
    : `${(value / 1000).toFixed(2)} s`;
}
const names: Record<string, string> = {
  approval_wait: "审批等待",
  exec_slot_wait: "命令容量等待",
  resource_wait: "资源等待",
  backend_lock_wait: "后端锁等待",
  backend_start: "后端启动",
  backend_call: "工具执行",
  process_run: "命令执行",
  log_drain: "日志收尾",
  snapshot_copy: "输入快照",
  scan: "目录扫描",
  hash: "内容哈希",
  prepare: "接收准备",
  delta_signatures: "增量签名",
  receive_install: "接收与安装",
  pack_decode: "解包",
  install: "文件校验与安装",
  commit: "同步提交",
  copy_payload: "文件传输",
  publish: "文件发布",
  output_measure: "产物测量",
  fsync: "持久化",
  sync_total: "同步总过程",
  manifest_receive: "清单协商与接收",
  send_payload: "发送载荷",
  confirm_source: "源文件再次确认",
  pack_encode: "打包压缩",
  artifact_store: "图片产物保存",
};
const kinds: Record<string, string> = {
  sync: "同步",
  "exec.start": "命令",
  "mcp.call": "桌面工具",
  "desktop.sequence": "桌面序列",
  "file.upload": "上传",
  "file.download": "下载",
  "artifact.download": "产物传输",
};
const statuses: Record<string, string> = {
  succeeded: "成功",
  failed: "失败",
  cancelled: "已取消",
  denied: "已拒绝",
  timed_out: "超时",
  unknown: "结果未知",
  running: "进行中",
  accepted: "已接收",
  awaiting_approval: "等待确认",
};

export function PerformanceDetail({
  metrics,
  server,
}: {
  metrics?: Metrics;
  server?: Metrics;
}) {
  if (!metrics)
    return (
      <p className="perf-muted">此任务的性能指标未采集。旧任务不会补造指标。</p>
    );
  const transferMs =
    metrics.first_payload_offset_ms != null &&
    metrics.last_payload_offset_ms != null
      ? metrics.last_payload_offset_ms - metrics.first_payload_offset_ms
      : null;
  const payload = metrics.bytes.payload;
  const speed =
    transferMs != null && transferMs > 0 && payload !== undefined
      ? (Number(payload) * 1000) / transferMs
      : null;
  return (
    <section className="performance-detail" aria-label="任务性能详情">
      <h3>性能详情</h3>
      <p>
        本机总耗时 <b>{millis(metrics.wall_ms)}</b> ·{" "}
        {metrics.complete ? "本机测量完成" : "测量未完成"}
      </p>
      <p className="perf-muted">
        阶段可能包含子阶段，以下耗时不能直接相加。单调时钟计时，系统休眠期间的时间不保证包含。
      </p>
      {[
        { name: "Mac 执行器", metrics },
        ...(server ? [{ name: "服务器", metrics: server }] : []),
      ].map((track) => (
        <div key={track.name}>
          <h4>{track.name}</h4>
          <div className="perf-phases">
            {track.metrics.phases.map((phase) => (
              <div key={phase.name} className="perf-phase">
                <span>{names[phase.name] || phase.name}</span>
                <meter
                  min={0}
                  max={Math.max(track.metrics.wall_ms, phase.wall_ms, 1)}
                  value={phase.wall_ms}
                  aria-label={names[phase.name] || phase.name}
                />
                <b>{millis(phase.wall_ms)}</b>
                <small>{phase.count} 次</small>
              </div>
            ))}
          </div>
        </div>
      ))}
      {!server && metrics.remote_job_id && (
        <p className="perf-muted">服务器阶段尚未收到；断线恢复后会补充。</p>
      )}
      <dl className="perf-facts">
        <dt>链路 / 方向</dt>
        <dd>
          {metrics.transport || "未采集"} / {metrics.direction || "—"}
        </dd>
        <dt>任务起点 RTT（连接级）</dt>
        <dd>{millis(metrics.rtt_ms)}</dd>
        <dt>变化文件</dt>
        <dd>
          {metrics.files.changed ?? "未采集"} / 完成{" "}
          {metrics.files.completed ?? "未采集"}
        </dd>
        <dt>文件原始大小</dt>
        <dd>
          {bytes(metrics.bytes.full_file ?? metrics.bytes.changed_logical)}
        </dd>
        <dt>传输载荷</dt>
        <dd>{bytes(payload)}</dd>
        <dt>载荷传输窗口</dt>
        <dd>{millis(transferMs)}</dd>
        <dt>首个载荷到达</dt>
        <dd>{millis(metrics.first_payload_offset_ms)}</dd>
        <dt>传输窗口平均速度</dt>
        <dd>{speed === null ? "无需传输或未采集" : `${bytes(speed)}/s`}</dd>
        <dt>已验证续传前缀</dt>
        <dd>{bytes(metrics.bytes.resume_offset)}</dd>
        <dt>增量复用</dt>
        <dd>{bytes(metrics.bytes.reused)}</dd>
        <dt>pack 原始 / 压缩</dt>
        <dd>
          {bytes(metrics.bytes.pack_raw)} /{" "}
          {bytes(metrics.bytes.pack_compressed)}
        </dd>
        <dt>图片产物</dt>
        <dd>{bytes(metrics.bytes.artifacts)}</dd>
        <dt>声明输出合计（文件去重）</dt>
        <dd>{bytes(metrics.bytes.declared_outputs)}</dd>
        <dt>日志大小</dt>
        <dd>{bytes(metrics.bytes.stored_log)}</dd>
        <dt>哈希缓存命中 / 未命中</dt>
        <dd>
          {metrics.files.hash_cache_hits ?? "未采集"} /{" "}
          {metrics.files.hash_cache_misses ?? "未采集"}
        </dd>
      </dl>
      {metrics.parent_task_id && (
        <p className="perf-muted">
          父任务 / 产物来源：<code>{metrics.parent_task_id}</code>
        </p>
      )}
      {metrics.dimensions && (
        <p className="perf-muted">
          {Object.entries(metrics.dimensions)
            .map(([k, v]) => `${k}: ${v}`)
            .join(" · ")}
        </p>
      )}
      <h4>声明输出</h4>
      {metrics.outputs.length ? (
        <ul>
          {metrics.outputs.map((o, i) => (
            <li key={i}>
              {o.name} · {bytes(o.bytes)} · {o.status}
            </li>
          ))}
        </ul>
      ) : (
        <p className="perf-muted">
          未声明产物。使用 exec 的 --output 相对路径记录文件或目录大小。
        </p>
      )}
      {!!metrics.samples.length && (
        <Trend
          title="采样区间平均载荷速度（B/s）"
          values={metrics.samples.map((s, i) => {
            if (!i) return null;
            const previous = metrics.samples[i - 1];
            const elapsed = s.offset_ms - previous.offset_ms;
            return elapsed > 0
              ? ((Number(s.payload_bytes) - Number(previous.payload_bytes)) *
                  1000) /
                  elapsed
              : null;
          })}
        />
      )}
    </section>
  );
}
export function Trend({
  title,
  values,
}: {
  title: string;
  values: (number | null)[];
}) {
  const max = Math.max(
    1,
    ...values.filter((v): v is number => v != null && Number.isFinite(v)),
  );
  const segments: string[][] = [[]];
  values.forEach((v, i) => {
    if (v == null || !Number.isFinite(v)) {
      if (segments.at(-1)?.length) segments.push([]);
    } else
      segments
        .at(-1)!
        .push(
          `${(i * 100) / Math.max(1, values.length - 1)},${38 - (v / max) * 34}`,
        );
  });
  return (
    <figure className="perf-trend">
      <figcaption>{title} · 无样本区间留空</figcaption>
      <small>
        样本 {values.filter((v) => v !== null).length} · 最大值{" "}
        {values.some((v) => v !== null)
          ? max.toLocaleString(undefined, { maximumFractionDigits: 1 })
          : "未采集"}
      </small>
      <svg
        viewBox="0 0 100 40"
        role="img"
        aria-label={title}
        preserveAspectRatio="none"
      >
        {segments
          .filter((s) => s.length)
          .map((s, i) => (
            <polyline
              key={i}
              fill="none"
              stroke="currentColor"
              strokeWidth="0.8"
              points={s.join(" ")}
            />
          ))}
      </svg>
    </figure>
  );
}

export function Performance({
  active,
  app,
}: {
  active: boolean;
  app: AppState | null;
}) {
  const [days, setDays] = useState("7"),
    [connection, setConnection] = useState(""),
    [project, setProject] = useState(""),
    [kind, setKind] = useState(""),
    [status, setStatus] = useState("");
  const [data, setData] = useState<Dashboard | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(false),
    [refresh, setRefresh] = useState(0);
  const [cursors, setCursors] = useState<(Cursor | null)[]>([null]),
    [page, setPage] = useState(0),
    [selected, setSelected] = useState<MetricTask | null>(null);
  const [projects, setProjects] = useState<Record<string, string>>({}),
    [exporting, setExporting] = useState(false),
    [message, setMessage] = useState("");
  const detailGeneration = useRef(0);
  const window = useMemo(() => {
    const to = Date.now() + 1;
    return { from_ms: to - Number(days) * 86400000, to_ms: to };
  }, [days, refresh]);
  const filters = {
    ...window,
    ...(connection ? { connection_id: connection } : {}),
    ...(project ? { project_id: project } : {}),
    ...(kind ? { kind } : {}),
    ...(status ? { status } : {}),
  };
  useEffect(() => {
    setData(null);
    setCursors([null]);
    setPage(0);
    setSelected(null);
    detailGeneration.current++;
  }, [days, connection, project, kind, status]);
  useEffect(() => {
    if (!active || page !== 0) return;
    const timer = globalThis.setInterval(() => {
      if (document.visibilityState !== "hidden") setRefresh((v) => v + 1);
    }, 15000);
    return () => globalThis.clearInterval(timer);
  }, [active, page]);
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    setLoading(true);
    setError("");
    const args = { ...filters, cursor: cursors[page] || null, limit: 50 };
    invoke<Dashboard>("control", { action: "metrics_dashboard", args })
      .then((result) => {
        if (
          !result ||
          !Array.isArray(result.operations) ||
          !Array.isArray(result.series) ||
          typeof result.total !== "number" ||
          !result.latency ||
          !result.bytes ||
          !Array.isArray(result.errors)
        )
          throw new Error("性能数据格式无效");
        if (!cancelled) {
          setData(result);
          setProjects((p) => ({ ...p, ...result.projects }));
        }
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [active, days, connection, project, kind, status, page, refresh]);
  const choose = async (task: MetricTask) => {
    const generation = ++detailGeneration.current;
    setSelected(task);
    try {
      const detail = await invoke<MetricTask>("control", {
        action: "metrics_detail",
        args: { task_id: task.task_id, connection_id: task.connection_id },
      });
      if (generation === detailGeneration.current) setSelected(detail);
    } catch (e) {
      if (generation === detailGeneration.current) setError(String(e));
    }
  };
  const exportData = async (format: string) => {
    setExporting(true);
    setMessage("");
    try {
      const result = await invoke<{ cancelled: boolean; count: number }>(
        "export_metrics",
        { args: filters, format },
      );
      if (!result.cancelled) setMessage(`已导出 ${result.count} 条记录`);
    } catch (e) {
      setError(`导出失败：${String(e)}`);
    } finally {
      setExporting(false);
    }
  };
  return (
    <section className="performance-page">
      <header className="page-header v4-toolbar">
        <h1>性能</h1>
        <button
          onClick={() => {
            setPage(0);
            setCursors([null]);
            setRefresh((n) => n + 1);
          }}
          disabled={loading}
        >
          刷新
        </button>
        {["JSON", "CSV"].map((format) => (
          <button
            key={format}
            onClick={() => void exportData(format.toLowerCase())}
            disabled={!data || exporting}
          >
            导出 {format}
          </button>
        ))}
      </header>
      {message && <p role="status">{message}</p>}
      <p className="perf-muted">
        {app?.worker_running
          ? "本机保存的性能历史"
          : "执行器已停止 · 仍可查看本机保存的历史"}{" "}
        ·{" "}
        {data ? `${data.measured}/${data.total} 个任务有性能指标` : "正在读取"}
      </p>
      <div className="perf-filters">
        <label>
          时间
          <select value={days} onChange={(e) => setDays(e.target.value)}>
            <option value="1">最近24小时</option>
            <option value="7">最近7天</option>
            <option value="30">最近30天</option>
            <option value="90">最近90天</option>
          </select>
        </label>
        <label>
          服务器
          <select
            value={connection}
            onChange={(e) => setConnection(e.target.value)}
          >
            <option value="">全部服务器</option>
            <option value="primary">
              {app?.settings.server || "主服务器"}
            </option>
            {app?.settings.connections?.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name || c.server}
              </option>
            ))}
          </select>
        </label>
        <label>
          项目
          <select value={project} onChange={(e) => setProject(e.target.value)}>
            <option value="">全部项目</option>
            {Object.entries(projects).map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <label>
          操作
          <select value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="">全部操作</option>
            {Object.entries(kinds).map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <label>
          状态
          <select value={status} onChange={(e) => setStatus(e.target.value)}>
            <option value="">全部状态</option>
            {Object.entries(statuses).map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error && (
        <p role="alert">
          性能历史读取失败：{error}{" "}
          <button onClick={() => setRefresh((n) => n + 1)}>重试</button>
        </p>
      )}
      {data?.errors.map((e) => (
        <p key={e.connection_id} role="alert">
          {e.message}（{e.connection_id}）
        </p>
      ))}
      {loading && <p role="status">正在读取性能历史…</p>}
      {data && (
        <>
          <p className="perf-muted">
            {Object.entries(data.counts)
              .map(([key, count]) => `${statuses[key] || key} ${count}`)
              .join(" · ")}
          </p>
          <div className="perf-cards">
            <article>
              <span>任务数 / 成功率</span>
              <b>
                {data.total} /{" "}
                {data.success_denominator
                  ? `${(((data.counts.succeeded || 0) / data.success_denominator) * 100).toFixed(1)}%`
                  : "—"}
              </b>
              <small>取消与拒绝单列，分母 {data.success_denominator}</small>
            </article>
            <article>
              <span>成功任务耗时 p50 / p95</span>
              <b>
                {millis(data.latency.p50_ms)} / {millis(data.latency.p95_ms)}
              </b>
              <small>
                p99 {millis(data.latency.p99_ms)} · n={data.latency.n}
              </small>
            </article>
            <article>
              <span>图片产物大小</span>
              <b>{bytes(data.bytes.artifacts)}</b>
              <small>声明输出见任务详情</small>
            </article>
            <article>
              <span>传输尝试载荷总量</span>
              <b>{bytes(data.bytes.payload)}</b>
              <small>按任务接收时间筛选，包含失败尝试</small>
            </article>
          </div>
          <div className="perf-charts">
            <Trend
              title="成功任务 p95 耗时趋势（按完成时间）"
              values={data.series.map((s) => s.p95_ms)}
            />
            <Trend
              title="传输尝试载荷趋势（按接收时间）"
              values={data.series.map((s) => Number(s.payload_bytes))}
            />
          </div>
          <p className="perf-muted">
            审批/排队包含在本机总耗时中。只有已完成的成功测量进入分位数；缺少指标的旧任务仅计数量。子任务与产物传输单独展示详情，不重复计入业务任务数。
          </p>
          <div className="perf-table">
            <table>
              <thead>
                <tr>
                  <th>接收时间</th>
                  <th>服务器 / 项目</th>
                  <th>操作</th>
                  <th>状态</th>
                  <th>总耗时</th>
                  <th>载荷</th>
                  <th>详情</th>
                </tr>
              </thead>
              <tbody>
                {data.operations.map((t) => (
                  <tr key={`${t.connection_id}:${t.task_id}`}>
                    <td>{new Date(t.started_at).toLocaleString()}</td>
                    <td>
                      {t.connection_name} / {t.metrics?.project_name || "—"}
                    </td>
                    <td>{kinds[t.kind] || t.kind}</td>
                    <td>{statuses[t.status] || t.status}</td>
                    <td>{millis(t.metrics?.wall_ms)}</td>
                    <td>{bytes(t.metrics?.bytes.payload)}</td>
                    <td>
                      <button
                        onClick={() => void choose(t)}
                        aria-label={`查看 ${t.task_id} 性能`}
                      >
                        查看
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!data.operations.length && <p>暂无匹配的性能历史。</p>}
          <div className="perf-pagination">
            <button
              disabled={!page || loading}
              onClick={() => setPage(page - 1)}
            >
              上一页
            </button>
            <span>第 {page + 1} 页</span>
            <button
              disabled={!data.next_cursor || loading}
              onClick={() => {
                setCursors((v) => [...v.slice(0, page + 1), data.next_cursor]);
                setPage(page + 1);
              }}
            >
              下一页
            </button>
          </div>
          {selected && (
            <section className="perf-selection">
              <button
                onClick={() => {
                  setSelected(null);
                  detailGeneration.current++;
                }}
              >
                关闭详情
              </button>
              <p className="mono">{selected.task_id}</p>
              <PerformanceDetail
                metrics={selected.metrics}
                server={selected.server_metrics}
              />
            </section>
          )}
        </>
      )}
    </section>
  );
}
