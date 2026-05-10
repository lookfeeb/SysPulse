import { Progress, Space, Tag, Typography } from "antd";
import { HddOutlined } from "@ant-design/icons";
import type { DiskHw } from "@/bindings";
import { fmtBytesCompact } from "@/utils/format";

const { Text } = Typography;

function tempColor(t: number | null | undefined): string {
  if (t == null) return "#d1d5db";
  if (t >= 60) return "#ef4444";
  if (t >= 50) return "#f97316";
  if (t >= 40) return "#eab308";
  return "#22c55e";
}

function healthColor(h: string): string {
  if (h === "good") return "green";
  if (h === "warning") return "orange";
  if (h === "critical") return "red";
  return "default";
}

export default function DiskPanel({ disks }: { disks: DiskHw[] }) {
  return (
    <Space vertical style={{ width: "100%" }} size={8}>
      {disks.map((disk) => {
        const usedPct =
          disk.totalBytes > 0 && disk.usedBytes != null
            ? (disk.usedBytes / disk.totalBytes) * 100
            : null;

        return (
          <div
            key={disk.identifier}
            style={{
              background: "#f9fafb",
              borderRadius: 8,
              padding: "10px 14px",
              border: "1px solid #f0f0f0",
            }}
          >
            {/* 型号 + 标签 */}
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
              <HddOutlined style={{ color: "#6b7280" }} />
              <Text strong style={{ fontSize: 12 }}>{disk.model || `Disk ${disk.index}`}</Text>
              <Tag style={{ fontSize: 10 }}>{disk.bus}</Tag>
              <Tag color={healthColor(disk.health)} style={{ fontSize: 10 }}>
                {disk.health || "—"}
              </Tag>
              <Tag color={tempColor(disk.tempC)} style={{ fontSize: 10 }}>
                {disk.tempC != null ? `${disk.tempC.toFixed(0)}°C` : "—"}
              </Tag>
            </div>

            {/* 容量进度条 */}
            {usedPct != null && (
              <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                <Text type="secondary" style={{ fontSize: 11, flexShrink: 0 }}>容量</Text>
                <Progress
                  percent={Math.round(usedPct)}
                  size="small"
                  strokeColor={usedPct >= 90 ? "#ef4444" : usedPct >= 75 ? "#f97316" : "#3388cc"}
                  showInfo={false}
                  style={{ flex: 1, margin: 0 }}
                />
                <Text style={{ fontSize: 11, fontVariantNumeric: "tabular-nums", flexShrink: 0 }}>
                  {fmtBytesCompact(disk.usedBytes!)} / {fmtBytesCompact(disk.totalBytes)}
                </Text>
              </div>
            )}
          </div>
        );
      })}
    </Space>
  );
}
