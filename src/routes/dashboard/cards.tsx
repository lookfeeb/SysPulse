import type { DiskHw, GpuHw } from "@/bindings";
import { fmtBytes, fmtSpeed, fmtTemp, maxValid, sumValid } from "@/utils/format";
import { Card, Statistic, Tooltip, Typography } from "antd";
import { type ReactNode } from "react";
import { firstText, joinLines } from './helpers';
export function MetricCard({
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
      <Card
        className="dashboard-card dashboard-card--third"
        styles={{ body: { minWidth: 0 } }}
        style={{ borderRadius: 10 }}
      >
        <Statistic title={title} value={value} precision={precision} suffix={suffix} />
        <CardLines lines={lines} />
      </Card>
    </Tooltip>
  );
}

export function NetworkCard({
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
      <Card
        className="dashboard-card dashboard-card--half"
        styles={{ body: { minWidth: 0 } }}
        style={{ borderRadius: 10 }}
      >
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

export function GpuCard({ gpus }: { gpus: GpuHw[] }) {
  const usage = maxValid(gpus.map((gpu) => gpu.usagePercent));
  const temp = maxValid(gpus.map((gpu) => gpu.tempC));
  const memUsed = sumValid(gpus.map((gpu) => gpu.memUsedMb));
  const memTotal = sumValid(gpus.map((gpu) => gpu.memTotalMb));

  return (
    <Tooltip
      title={joinLines([
        `显卡占用：${usage == null ? "--" : `${usage.toFixed(1)}%`}`,
        `显卡温度：${fmtTemp(temp)}`,
        `显存：${memUsed != null && memTotal != null
          ? `${memUsed.toFixed(0)} / ${memTotal.toFixed(0)} MB`
          : "--"
        }`,
        `数量：${gpus.length}`,
        ...gpus.map((gpu) => gpu.name || `GPU ${gpu.index}`),
      ])}
    >
      <Card
        className="dashboard-card dashboard-card--third"
        styles={{ body: { minWidth: 0 } }}
        style={{ borderRadius: 10 }}
      >
        <Statistic title="显卡" value={usage ?? 0} precision={1} suffix="%" />
        <CardLines
          lines={[
            `温度 ${fmtTemp(temp)} · 显存 ${memUsed != null && memTotal != null
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

export function DiskCard({ disks }: { disks: DiskHw[] }) {
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
            `${disk.model || `Disk ${disk.index}`} · 温度 ${fmtTemp(disk.tempC)} · 健康 ${disk.health || "--"
            }`,
        ),
      ])}
    >
      <Card
        className="dashboard-card dashboard-card--half"
        styles={{ body: { minWidth: 0 } }}
        style={{ borderRadius: 10 }}
      >
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
  return <div className="dashboard-speed-row">{children}</div>;
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
    <div className="dashboard-speed-value">
      <Typography.Text className="dashboard-speed-label" type="secondary">
        {label}
      </Typography.Text>
      <div
        className="dashboard-speed-number"
        style={{
          color,
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
