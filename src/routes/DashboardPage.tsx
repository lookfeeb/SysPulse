import { Card, Statistic, Tooltip, Typography } from "antd";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { DiskHw, GpuHw, HwSnapshot, Snapshot } from "@/bindings";
import { useHwStore } from "@/stores/hwStore";
import { useLiveStore } from "@/stores/liveStore";
import { fmtBytes, fmtFreq, fmtSpeed, fmtTemp, maxValid, sumValid } from "@/utils/format";

function firstText(values: (string | null | undefined)[]): string {
  return values.find((value) => value && value.trim()) ?? "--";
}

function joinLines(lines: string[]): ReactNode {
  return (
    <div>
      {lines.map((line, index) => (
        <div key={`${line}-${index}`}>{line}</div>
      ))}
    </div>
  );
}

export default function DashboardPage() {
  const { t } = useTranslation();
  const current = useLiveStore((s) => s.current);
  const history = useLiveStore((s) => s.history);
  const hw = useHwStore((s) => s.current);
  const hwHistory = useHwStore((s) => s.history);

  if (!current) {
    return <Typography.Text type="secondary">{t("dashboard.noData")}</Typography.Text>;
  }

  const cpu = current.cpu;
  const mem = current.memory;
  const net = current.network.total;
  const hwCpu = hw?.cpu;
  const gpus = hw?.gpus ?? [];
  const disks = hw?.disks ?? [];

  return (
    <div>
      <div
        style={{
          display: "grid",
          gap: 16,
          gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))",
        }}
      >
        <MetricCard
          title="CPU"
          value={cpu.usagePercent}
          precision={1}
          suffix="%"
          lines={[
            `温度 ${fmtTemp(hwCpu?.packageTempC)} · 频率 ${fmtFreq(hwCpu?.frequencyMhz)}`,
            `${cpu.model || hwCpu?.name || "--"}${
              cpu.physicalCores ? ` · ${cpu.physicalCores} ${t("dashboard.cores")}` : ""
            }`,
          ]}
          details={[
            `CPU 占用：${cpu.usagePercent.toFixed(1)}%`,
            `CPU 温度：${fmtTemp(hwCpu?.packageTempC)}`,
            `CPU 频率：${fmtFreq(hwCpu?.frequencyMhz)}`,
            `型号：${cpu.model || hwCpu?.name || "--"}`,
            `核心：${cpu.physicalCores || "--"}`,
          ]}
        />
        <MetricCard
          title={t("dashboard.memory")}
          value={mem.usedPercent}
          precision={1}
          suffix="%"
          lines={[
            `${fmtBytes(mem.usedBytes)} / ${fmtBytes(mem.totalBytes)}`,
            hw?.memory?.frequencyMhz
              ? `频率 ${fmtFreq(hw.memory.frequencyMhz)}`
              : "频率 --",
          ]}
          details={[
            `内存占用：${mem.usedPercent.toFixed(1)}%`,
            `已用：${fmtBytes(mem.usedBytes)}`,
            `总量：${fmtBytes(mem.totalBytes)}`,
            `频率：${fmtFreq(hw?.memory?.frequencyMhz)}`,
          ]}
        />
        <NetworkCard
          down={net.bytesRecvPerSec}
          up={net.bytesSentPerSec}
          totalDown={net.bytesRecvTotal}
          totalUp={net.bytesSentTotal}
        />
        <GpuCard gpus={gpus} />
        <DiskCard disks={disks} />
      </div>

      <Card title={t("dashboard.live60s")} style={{ marginTop: 16, borderRadius: 10 }}>
        <NetworkSparkline history={history} hwHistory={hwHistory} />
      </Card>
    </div>
  );
}

function MetricCard({
  title,
  value,
  precision,
  suffix,
  lines,
  details,
}: {
  title: string;
  value: number;
  precision?: number;
  suffix?: string;
  lines: string[];
  details: string[];
}) {
  return (
    <Tooltip title={joinLines(details)}>
      <Card styles={{ body: { minWidth: 0 } }} style={{ borderRadius: 10 }}>
        <Statistic title={title} value={value} precision={precision} suffix={suffix} />
        <CardLines lines={lines} />
      </Card>
    </Tooltip>
  );
}

function NetworkCard({
  down,
  up,
  totalDown,
  totalUp,
}: {
  down: number;
  up: number;
  totalDown: number;
  totalUp: number;
}) {
  return (
    <Tooltip
      title={joinLines([
        `下行速度：${fmtSpeed(down)}`,
        `上行速度：${fmtSpeed(up)}`,
        `下行累计：${fmtBytes(totalDown)}`,
        `上行累计：${fmtBytes(totalUp)}`,
      ])}
    >
      <Card styles={{ body: { minWidth: 0 } }} style={{ borderRadius: 10 }}>
        <Typography.Text type="secondary">网速</Typography.Text>
        <SpeedRow>
          <SpeedValue label="↓" value={fmtSpeed(down)} color="#3388cc" />
          <SpeedValue label="↑" value={fmtSpeed(up)} color="#ff8844" />
        </SpeedRow>
        <CardLines
          lines={[
            `下行累计 ${fmtBytes(totalDown)}`,
            `上行累计 ${fmtBytes(totalUp)}`,
          ]}
        />
      </Card>
    </Tooltip>
  );
}

function GpuCard({ gpus }: { gpus: GpuHw[] }) {
  const usage = maxValid(gpus.map((gpu) => gpu.usagePercent));
  const temp = maxValid(gpus.map((gpu) => gpu.tempC));
  const memUsed = sumValid(gpus.map((gpu) => gpu.memUsedMb));
  const memTotal = sumValid(gpus.map((gpu) => gpu.memTotalMb));

  return (
    <Tooltip
      title={joinLines([
        `显卡占用：${usage == null ? "--" : `${usage.toFixed(1)}%`}`,
        `显卡温度：${fmtTemp(temp)}`,
        `显存：${
          memUsed != null && memTotal != null
            ? `${memUsed.toFixed(0)} / ${memTotal.toFixed(0)} MB`
            : "--"
        }`,
        `数量：${gpus.length}`,
        ...gpus.map((gpu) => gpu.name || `GPU ${gpu.index}`),
      ])}
    >
      <Card styles={{ body: { minWidth: 0 } }} style={{ borderRadius: 10 }}>
        <Statistic title="显卡" value={usage ?? 0} precision={1} suffix="%" />
        <CardLines
          lines={[
            `温度 ${fmtTemp(temp)} · 显存 ${
              memUsed != null && memTotal != null
                ? `${memUsed.toFixed(0)} / ${memTotal.toFixed(0)} MB`
                : "--"
            }`,
            firstText(gpus.map((gpu) => gpu.name)),
          ]}
        />
      </Card>
    </Tooltip>
  );
}

function DiskCard({ disks }: { disks: DiskHw[] }) {
  const read = sumValid(disks.map((disk) => disk.readBytesPerSec));
  const write = sumValid(disks.map((disk) => disk.writeBytesPerSec));
  const primaryDisk = disks.find(
    (disk) =>
      disk.totalBytes > 0 &&
      disk.usedBytes != null &&
      Number.isFinite(disk.usedBytes),
  );
  const temp = maxValid(disks.map((disk) => disk.tempC));
  const available =
    primaryDisk && primaryDisk.usedBytes != null
      ? Math.max(0, primaryDisk.totalBytes - primaryDisk.usedBytes)
      : null;
  const spaceText =
    primaryDisk && available != null
      ? `可用 ${fmtBytes(available)} / ${fmtBytes(primaryDisk.totalBytes)}`
      : "--";

  return (
    <Tooltip
      title={joinLines([
        `读取速度：${fmtSpeed(read)}`,
        `写入速度：${fmtSpeed(write)}`,
        `最高温度：${fmtTemp(temp)}`,
        `主盘空间：${spaceText}`,
        `数量：${disks.length}`,
        ...disks.map(
          (disk) =>
            `${disk.model || `Disk ${disk.index}`} · 温度 ${fmtTemp(disk.tempC)} · 健康 ${
              disk.health || "--"
            }`,
        ),
      ])}
    >
      <Card styles={{ body: { minWidth: 0 } }} style={{ borderRadius: 10 }}>
        <Typography.Text type="secondary">硬盘</Typography.Text>
        <SpeedRow>
          <SpeedValue label="读" value={fmtSpeed(read)} color="#3388cc" />
          <SpeedValue label="写" value={fmtSpeed(write)} color="#ff8844" />
        </SpeedRow>
        <CardLines
          lines={[
            `温度 ${fmtTemp(temp)} · ${spaceText}`,
            firstText(disks.map((disk) => disk.model)),
          ]}
        />
      </Card>
    </Tooltip>
  );
}

function SpeedRow({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        columnGap: 12,
        display: "grid",
        gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)",
        marginTop: 4,
      }}
    >
      {children}
    </div>
  );
}

function SpeedValue({
  label,
  value,
  color,
}: {
  label: string;
  value: string;
  color: string;
}) {
  return (
    <div style={{ minWidth: 0 }}>
      <Typography.Text type="secondary">{label}</Typography.Text>
      <div
        style={{
          color,
          fontSize: 22,
          lineHeight: 1.3,
          maxWidth: "100%",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
        title={value}
      >
        {value}
      </div>
    </div>
  );
}

function CardLines({ lines }: { lines: string[] }) {
  return (
    <div style={{ marginTop: 8 }}>
      {lines.map((line, index) => (
        <Typography.Text
          key={`${line}-${index}`}
          type="secondary"
          style={{
            display: "block",
            fontSize: 12,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
          title={line}
        >
          {line}
        </Typography.Text>
      ))}
    </div>
  );
}

type SparklineSeries = {
  key: string;
  label: string;
  color: string;
  values: Array<number | null | undefined>;
  format: (value: number) => string;
  unit: "speed" | "percent" | "temp";
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

// 计算美观的刻度值（取整到合适的量级）
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

function NetworkSparkline({
  history,
  hwHistory,
}: {
  history: Snapshot[];
  hwHistory: HwSnapshot[];
}) {
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const [hiddenSeries, setHiddenSeries] = useState<Set<string>>(() => new Set());

  // 画布尺寸（逻辑坐标）
  const W = 800;
  const H = 180;
  const PAD_LEFT = 56;  // Y 轴刻度区域（左：网速）
  const PAD_BOTTOM = 24; // X 轴刻度区域
  const PAD_TOP = 8;
  const PAD_RIGHT = 50; // Y 轴刻度区域（右：温度/百分比）
  const CHART_W = W - PAD_LEFT - PAD_RIGHT;
  const CHART_H = H - PAD_TOP - PAD_BOTTOM;

  const series: SparklineSeries[] = [
    {
      key: "down",
      label: "下行",
      color: "#3388cc",
      unit: "speed",
      values: history.map((s) => s.network.total.bytesRecvPerSec),
      format: fmtSpeed,
    },
    {
      key: "up",
      label: "上行",
      color: "#ff8844",
      unit: "speed",
      values: history.map((s) => s.network.total.bytesSentPerSec),
      format: fmtSpeed,
    },
    {
      key: "cpu-temp",
      label: "CPU 温度",
      color: "#d946ef",
      unit: "temp",
      values: hwHistory.map((s) => s.cpu?.packageTempC),
      format: fmtTemp,
    },
    {
      key: "gpu-temp",
      label: "GPU 温度",
      color: "#22c55e",
      unit: "temp",
      values: hwHistory.map((s) => maxValid((s.gpus ?? []).map((gpu) => gpu.tempC))),
      format: fmtTemp,
    },
    {
      key: "mem",
      label: "内存",
      color: "#64748b",
      unit: "percent",
      values: history.map((s) => s.memory.usedPercent),
      format: (value) => `${value.toFixed(0)}%`,
    },
  ];

  const availableSeries = series.filter((item) =>
    item.values.some((value) => value != null && Number.isFinite(value)),
  );
  const visibleSeries = availableSeries.filter((item) => !hiddenSeries.has(item.key));
  const maxLen = Math.max(0, ...availableSeries.map((item) => item.values.length));

  if (maxLen < 2) {
    return (
      <Typography.Text type="secondary">
        样本累积中…(已有 {maxLen} 个采样点)
      </Typography.Text>
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

  // 每条曲线独立 Y 轴（归一化到 0~1，再映射到图表高度）
  const toY = (value: number, seriesItem: SparklineSeries) => {
    const max = niceTickMax(seriesMax(seriesItem.values), seriesItem.unit);
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

  // Y 轴刻度（基于第一条可见 speed 系列，或 temp，或 percent）
  const primarySeries = visibleSeries.find((s) => s.unit === "speed")
    ?? visibleSeries.find((s) => s.unit === "temp")
    ?? visibleSeries[0];

  const yTicks = primarySeries
    ? (() => {
        const max = niceTickMax(seriesMax(primarySeries.values), primarySeries.unit);
        const count = 4;
        return Array.from({ length: count + 1 }, (_, i) => {
          const v = (max * i) / count;
          return { value: v, y: PAD_TOP + CHART_H - (i / count) * CHART_H };
        });
      })()
    : [];

  // 右侧 Y 轴刻度（温度或百分比）
  const secondarySeries = visibleSeries.find((s) => s.unit === "temp")
    ?? visibleSeries.find((s) => s.unit === "percent");
  const hasSecondaryAxis = secondarySeries && secondarySeries !== primarySeries;

  const yTicksRight = hasSecondaryAxis
    ? (() => {
        const max = niceTickMax(seriesMax(secondarySeries.values), secondarySeries.unit);
        const count = 4;
        return Array.from({ length: count + 1 }, (_, i) => {
          const v = (max * i) / count;
          return { value: v, y: PAD_TOP + CHART_H - (i / count) * CHART_H };
        });
      })()
    : [];

  // X 轴时间刻度（最近 60 秒，每 15 秒一个刻度）
  const xTickCount = 4;
  const xTicks = Array.from({ length: xTickCount + 1 }, (_, i) => {
    const secAgo = 60 - (60 * i) / xTickCount;
    const x = PAD_LEFT + (i / xTickCount) * CHART_W;
    const label = secAgo === 0 ? "现在" : `-${secAgo.toFixed(0)}s`;
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
    setHoverIndex(index);
  };

  return (
    <div style={{ position: "relative" }}>
      {/* 图例 */}
      <div style={{ display: "flex", flexWrap: "wrap", gap: "4px 12px", marginBottom: 8 }}>
        {availableSeries.map((item) => {
          const hidden = hiddenSeries.has(item.key);
          return (
            <span
              key={item.key}
              onClick={() => toggleSeries(item.key)}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 4,
                cursor: "pointer",
                userSelect: "none",
                opacity: hidden ? 0.35 : 1,
              }}
              title={hidden ? "点击显示" : "点击隐藏"}
            >
              <span
                style={{
                  display: "inline-block",
                  width: 20,
                  height: 2,
                  background: item.color,
                  borderRadius: 1,
                  verticalAlign: "middle",
                  textDecoration: hidden ? "line-through" : "none",
                }}
              />
              <Typography.Text
                style={{
                  color: item.color,
                  fontSize: 12,
                  textDecoration: hidden ? "line-through" : "none",
                }}
              >
                {item.label}
              </Typography.Text>
            </span>
          );
        })}
      </div>

      {visibleSeries.length === 0 && (
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          点击上方图例显示曲线
        </Typography.Text>
      )}

      <svg
        viewBox={`0 0 ${W} ${H}`}
        style={{ width: "100%", height: 220, display: "block" }}
        onMouseMove={(event) => updateHover(event.clientX, event.currentTarget)}
        onMouseLeave={() => setHoverIndex(null)}
      >
        {/* 背景 */}
        <rect
          x={PAD_LEFT}
          y={PAD_TOP}
          width={CHART_W}
          height={CHART_H}
          fill="#f8fafc"
          rx={4}
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
              fill={secondarySeries?.unit === "temp" ? "#d946ef" : "#64748b"}
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
            strokeWidth={1.8}
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

      {/* 悬停 Tooltip */}
      {hover && (
        <div
          style={{
            background: "rgba(255,255,255,0.98)",
            border: "1px solid rgba(15,23,42,0.1)",
            borderRadius: 8,
            boxShadow: "0 4px 16px rgba(15,23,42,0.12)",
            left: `min(calc(${((hover.x) / W) * 100}% + 8px), calc(100% - 160px))`,
            padding: "8px 10px",
            pointerEvents: "none",
            position: "absolute",
            top: 32,
            minWidth: 140,
            zIndex: 1,
          }}
        >
          <Typography.Text strong style={{ display: "block", fontSize: 11, color: "#64748b", marginBottom: 4 }}>
            最近 60 秒
          </Typography.Text>
          {hover.lines.map((item) => (
            <div key={item.key} style={{ display: "flex", justifyContent: "space-between", gap: 12 }}>
              <Typography.Text style={{ color: item.color, fontSize: 12 }}>
                {item.label}
              </Typography.Text>
              <Typography.Text style={{ color: item.color, fontSize: 12, fontWeight: 600, fontVariantNumeric: "tabular-nums" }}>
                {item.format(item.value)}
              </Typography.Text>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
