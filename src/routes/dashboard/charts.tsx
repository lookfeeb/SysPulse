import type { Snapshot } from "@/bindings";
import type { HwHistoryPoint } from "@/stores/hwStore";
import { fmtSpeed, fmtTemp } from "@/utils/format";
import { useState } from "react";
import { alignHistory, historyTimeline } from "./historyTimeline";
type SparklineSeries = {
  key: string;
  label: string;
  color: string;
  values: Array<number | null | undefined>;
  format: (value: number) => string;
  unit: "speed" | "percent" | "temp";
  axis: "left" | "right";
};

function latestSeriesValue(
  values: Array<number | null | undefined>,
  index: number,
  maxLen: number,
) {
  if (values.length === 0) return null;
  if (maxLen <= 1) return values[values.length - 1] ?? null;
  const sourceIndex = Math.max(
    0,
    Math.min(values.length - 1, Math.round((index / (maxLen - 1)) * (values.length - 1))),
  );
  const value = values[sourceIndex];
  return value != null && Number.isFinite(value) ? value : null;
}

function seriesMax(values: Array<number | null | undefined>) {
  return Math.max(
    1,
    ...values.filter((v): v is number => v != null && Number.isFinite(v)),
  );
}

function niceTickMax(rawMax: number, unit: SparklineSeries["unit"]): number {
  if (unit === "percent") return 100;
  if (unit === "temp") {
    if (rawMax <= 50) return 50;
    if (rawMax <= 70) return 70;
    if (rawMax <= 90) return 90;
    return 100;
  }
  // speed: 取最近的 2 的幂次 × 合适单位
  const KB = 1024, MB = 1024 * 1024, GB = 1024 * 1024 * 1024;
  if (rawMax < KB) return Math.ceil(rawMax / 100) * 100 || 100;
  if (rawMax < MB) return Math.ceil(rawMax / KB / 100) * 100 * KB || KB;
  if (rawMax < GB) return Math.ceil(rawMax / MB / 10) * 10 * MB || MB;
  return Math.ceil(rawMax / GB) * GB;
}

function historyTimeLabel(index: number, timestamps: number[]) {
  const secondsAgo = Math.round(((timestamps.at(-1) ?? 0) - (timestamps[index] ?? 0)) / 1000);
  return secondsAgo <= 0 ? "最新采样" : `${secondsAgo} 秒前`;
}

export function NetworkSparkline({
  history,
  hwHistory,
}: {
  history: Snapshot[];
  hwHistory: HwHistoryPoint[];
}) {
  const timestamps = historyTimeline(history, hwHistory);
  const live = alignHistory(history, timestamps, 6000);
  const hardware = alignHistory(hwHistory, timestamps, 5000);
  const networkSeries: SparklineSeries[] = [
    {
      key: "down",
      label: "下行",
      color: "#3388cc",
      unit: "speed",
      values: live.map((s) => s?.network.total.bytesRecvPerSec),
      format: fmtSpeed,
      axis: "left",
    },
    {
      key: "up",
      label: "上行",
      color: "#ff8844",
      unit: "speed",
      values: live.map((s) => s?.network.total.bytesSentPerSec),
      format: fmtSpeed,
      axis: "left",
    },
  ];
  const systemSeries: SparklineSeries[] = [
    {
      key: "cpu-temp",
      label: "CPU 温度",
      color: "#d946ef",
      unit: "temp",
      values: hardware.map((s) => s?.cpuTempC),
      format: fmtTemp,
      axis: "left",
    },
    {
      key: "gpu-temp",
      label: "GPU 温度",
      color: "#22c55e",
      unit: "temp",
      values: hardware.map((s) => s?.gpuTempC),
      format: fmtTemp,
      axis: "left",
    },
    {
      key: "mem",
      label: "内存",
      color: "#64748b",
      unit: "percent",
      values: live.map((s) => s?.memory.usedPercent),
      format: (value) => `${value.toFixed(0)}%`,
      axis: "right",
    },
  ];

  return (
    <div className="dashboard-history-grid">
      <HistoryChart
        chartId="network"
        title="网络流量"
        subtitle="上下行使用同一速度刻度"
        series={networkSeries}
        timestamps={timestamps}
      />
      <HistoryChart
        chartId="system"
        title="系统状态"
        subtitle="左轴温度 · 右轴内存占用"
        series={systemSeries}
        timestamps={timestamps}
      />
    </div>
  );
}

function HistoryChart({
  chartId,
  title,
  subtitle,
  series,
  timestamps,
}: {
  chartId: string;
  title: string;
  subtitle: string;
  series: SparklineSeries[];
  timestamps: number[];
}) {
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const [hiddenSeries, setHiddenSeries] = useState<Set<string>>(() => new Set());
  const W = 720;
  const H = 210;
  const PAD_LEFT = 58;
  const PAD_BOTTOM = 26;
  const PAD_TOP = 12;
  const PAD_RIGHT = series.some((item) => item.axis === "right") ? 42 : 12;
  const CHART_W = W - PAD_LEFT - PAD_RIGHT;
  const CHART_H = H - PAD_TOP - PAD_BOTTOM;

  const availableSeries = series.filter((item) =>
    item.values.some((value) => value != null && Number.isFinite(value)),
  );
  const visibleSeries = availableSeries.filter((item) => !hiddenSeries.has(item.key));
  const maxLen = Math.max(0, ...availableSeries.map((item) => item.values.length));

  if (maxLen < 2) {
    return (
      <section className="dashboard-history-panel">
        <div className="dashboard-history-heading"><strong>{title}</strong><span>{subtitle}</span></div>
        <div className="dashboard-history-empty">样本累积中…（已有 {maxLen} 个采样点）</div>
      </section>
    );
  }

  const stepX = CHART_W / Math.max(1, maxLen - 1);

  const toggleSeries = (key: string) => {
    setHiddenSeries((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const axisMax = (axis: "left" | "right") => {
    const axisSeries = visibleSeries.filter((item) => item.axis === axis);
    const unit = axisSeries[0]?.unit ?? "percent";
    return niceTickMax(Math.max(1, ...axisSeries.map((item) => seriesMax(item.values))), unit);
  };
  const leftMax = axisMax("left");
  const rightMax = axisMax("right");

  const toY = (value: number, seriesItem: SparklineSeries) => {
    const max = seriesItem.axis === "right" ? rightMax : leftMax;
    const ratio = Math.min(1, Math.max(0, value / max));
    return PAD_TOP + CHART_H - ratio * CHART_H;
  };

  const toX = (index: number) => PAD_LEFT + index * stepX;

  const toPath = (seriesItem: SparklineSeries) => {
    const parts: string[] = [];
    for (let index = 0; index < maxLen; index += 1) {
      const value = latestSeriesValue(seriesItem.values, index, maxLen);
      if (value == null) continue;
      const x = toX(index).toFixed(1);
      const y = toY(value, seriesItem).toFixed(1);
      parts.push(`${parts.length === 0 ? "M" : "L"}${x},${y}`);
    }
    return parts.join(" ");
  };

  const primarySeries = visibleSeries.find((item) => item.axis === "left");
  const secondarySeries = visibleSeries.find((item) => item.axis === "right");

  const yTicks = primarySeries
    ? (() => {
      const count = 4;
      return Array.from({ length: count + 1 }, (_, i) => {
        const v = (leftMax * i) / count;
        return { value: v, y: PAD_TOP + CHART_H - (i / count) * CHART_H };
      });
    })()
    : [];

  const yTicksRight = secondarySeries
    ? (() => {
      const count = 4;
      return Array.from({ length: count + 1 }, (_, i) => {
        const v = (rightMax * i) / count;
        return { value: v, y: PAD_TOP + CHART_H - (i / count) * CHART_H };
      });
    })()
    : [];

  // Use actual sample times, including variable collection intervals.
  const durationSeconds = ((timestamps.at(-1) ?? 0) - (timestamps[0] ?? 0)) / 1000;
  const xTickCount = 4;
  const xTicks = Array.from({ length: xTickCount + 1 }, (_, i) => {
    const secAgo = durationSeconds * (1 - i / xTickCount);
    const x = PAD_LEFT + (i / xTickCount) * CHART_W;
    const label = secAgo === 0 ? "最新" : `-${secAgo.toFixed(0)}s`;
    return { x, label };
  });

  const hover =
    hoverIndex == null
      ? null
      : {
        lines: visibleSeries
          .map((item) => {
            const value = latestSeriesValue(item.values, hoverIndex, maxLen);
            return value == null ? null : { ...item, value };
          })
          .filter((item): item is SparklineSeries & { value: number } => item != null),
        x: toX(hoverIndex),
      };

  const updateHover = (clientX: number, svg: SVGSVGElement) => {
    const rect = svg.getBoundingClientRect();
    const x = ((clientX - rect.left) / rect.width) * W;
    const chartX = x - PAD_LEFT;
    const index = Math.max(0, Math.min(maxLen - 1, Math.round(chartX / stepX)));
    setHoverIndex((current) => (current === index ? current : index));
  };

  return (
    <section className="dashboard-history-panel">
      <div className="dashboard-history-heading">
        <div><strong>{title}</strong><span>{subtitle}</span></div>
        <span className="dashboard-history-window">{durationSeconds.toFixed(0)} 秒</span>
      </div>
      <div className="dashboard-history-legend">
        {availableSeries.map((item) => {
          const hidden = hiddenSeries.has(item.key);
          const latest = latestSeriesValue(item.values, maxLen - 1, maxLen);
          return (
            <button
              type="button"
              key={item.key}
              onClick={() => toggleSeries(item.key)}
              className={`dashboard-history-legend-item${hidden ? " is-hidden" : ""}`}
              title={hidden ? "点击显示" : "点击隐藏"}
            >
              <span className="dashboard-history-dot" style={{ background: item.color }} />
              <span>{item.label}</span>
              <b style={{ color: item.color }}>{latest == null ? "--" : item.format(latest)}</b>
            </button>
          );
        })}
      </div>

      {visibleSeries.length === 0 && (
        <div className="dashboard-history-empty">点击上方图例显示曲线</div>
      )}

      <div className="dashboard-history-chart">
        <svg viewBox={`0 0 ${W} ${H}`} onMouseMove={(event) => updateHover(event.clientX, event.currentTarget)} onMouseLeave={() => setHoverIndex(null)}>
          <defs>
            <linearGradient id={`${chartId}-surface`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#f8fbff" />
              <stop offset="100%" stopColor="#f3f7fb" />
            </linearGradient>
          </defs>
          {/* 背景 */}
          <rect
            x={PAD_LEFT}
            y={PAD_TOP}
            width={CHART_W}
            height={CHART_H}
            fill={`url(#${chartId}-surface)`}
            rx={8}
          />

          {/* Y 轴网格线 + 左侧刻度（网速） */}
          {yTicks.map((tick, i) => (
            <g key={i}>
              <line
                x1={PAD_LEFT}
                x2={PAD_LEFT + CHART_W}
                y1={tick.y}
                y2={tick.y}
                stroke={i === 0 ? "rgba(15,23,42,0.12)" : "rgba(15,23,42,0.06)"}
                strokeWidth={i === 0 ? 1 : 0.8}
              />
              <text
                x={PAD_LEFT - 4}
                y={tick.y + 4}
                textAnchor="end"
                fontSize={9}
                fill="#94a3b8"
                fontFamily="system-ui, sans-serif"
              >
                {primarySeries
                  ? primarySeries.format(tick.value).replace(" ", "\u00A0")
                  : ""}
              </text>
            </g>
          ))}

          {/* 右侧 Y 轴刻度（温度/百分比） */}
          {yTicksRight.map((tick, i) => (
            <g key={`r${i}`}>
              <text
                x={PAD_LEFT + CHART_W + 4}
                y={tick.y + 4}
                textAnchor="start"
                fontSize={9}
                fill="#64748b"
                fontFamily="system-ui, sans-serif"
              >
                {secondarySeries
                  ? secondarySeries.format(tick.value).replace(" ", "\u00A0")
                  : ""}
              </text>
            </g>
          ))}

          {/* X 轴刻度 */}
          {xTicks.map((tick, i) => (
            <g key={i}>
              <line
                x1={tick.x}
                x2={tick.x}
                y1={PAD_TOP}
                y2={PAD_TOP + CHART_H + 4}
                stroke="rgba(15,23,42,0.08)"
                strokeWidth={0.8}
              />
              <text
                x={tick.x}
                y={PAD_TOP + CHART_H + 16}
                textAnchor="middle"
                fontSize={9}
                fill="#94a3b8"
                fontFamily="system-ui, sans-serif"
              >
                {tick.label}
              </text>
            </g>
          ))}

          {/* 数据曲线 */}
          {visibleSeries.map((item) => (
            <path
              key={item.key}
              d={toPath(item)}
              stroke={item.color}
              fill="none"
              strokeWidth={2.2}
              vectorEffect="non-scaling-stroke"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          ))}

          {/* 悬停竖线 */}
          {hover && (
            <line
              x1={hover.x}
              x2={hover.x}
              y1={PAD_TOP}
              y2={PAD_TOP + CHART_H}
              stroke="rgba(15,23,42,0.3)"
              strokeDasharray="3 3"
              strokeWidth={1}
            />
          )}

          {/* 悬停点 */}
          {hover &&
            hover.lines.map((item) => {
              const value = latestSeriesValue(item.values, hoverIndex!, maxLen);
              if (value == null) return null;
              return (
                <circle
                  key={item.key}
                  cx={hover.x}
                  cy={toY(value, item)}
                  r={3}
                  fill={item.color}
                  stroke="white"
                  strokeWidth={1.5}
                />
              );
            })}

          {/* 图表边框 */}
          <rect
            x={PAD_LEFT}
            y={PAD_TOP}
            width={CHART_W}
            height={CHART_H}
            fill="none"
            stroke="rgba(15,23,42,0.1)"
            strokeWidth={1}
            rx={4}
          />
        </svg>

        {hover && (
          <div className="dashboard-history-tooltip" style={{ left: `min(calc(${(hover.x / W) * 100}% + 10px), calc(100% - 178px))` }}>
            <strong>{historyTimeLabel(hoverIndex!, timestamps)}</strong>
            {hover.lines.map((item) => (
              <div key={item.key}>
                <span><i style={{ background: item.color }} />{item.label}</span>
                <b style={{ color: item.color }}>{item.format(item.value)}</b>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
