import { Progress, Space, Tag, Typography } from "antd";
import {
  FireOutlined,
  ThunderboltOutlined,
  DashboardOutlined,
  PercentageOutlined,
} from "@ant-design/icons";
import type { CpuHw } from "@/bindings";
import { fmtMaybe } from "@/utils/format";

const { Text } = Typography;

function tempColor(t: number | null | undefined): string {
  if (t == null) return "#d1d5db";
  if (t >= 80) return "#ef4444";
  if (t >= 65) return "#f97316";
  if (t >= 50) return "#eab308";
  return "#22c55e";
}

function usageColor(u: number): string {
  if (u >= 90) return "#ef4444";
  if (u >= 70) return "#f97316";
  if (u >= 50) return "#eab308";
  return "#3388cc";
}

export default function CpuPanel({ cpu }: { cpu: CpuHw }) {
  const perCoreUsage = cpu.perCoreUsage ?? [];
  const perCoreTemps = cpu.perCoreTempsC ?? [];

  return (
    <Space direction="vertical" style={{ width: "100%" }} size={12}>
      {/* 标题 + 概览指标 */}
      <div>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Text strong style={{ fontSize: 13 }}>{cpu.name}</Text>
          <Text type="secondary" style={{ fontSize: 12 }}>
            {perCoreUsage.length} 核心
          </Text>
        </div>
        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            gap: "6px 16px",
            marginTop: 8,
          }}
        >
          <Tag icon={<FireOutlined />} color={tempColor(cpu.packageTempC)}>
            {fmtMaybe(cpu.packageTempC, (v) => `${v.toFixed(0)}°C`)}
          </Tag>
          <Tag icon={<ThunderboltOutlined />} color="blue">
            {fmtMaybe(cpu.frequencyMhz, (v) =>
              v >= 1000 ? `${(v / 1000).toFixed(2)} GHz` : `${v.toFixed(0)} MHz`
            )}
          </Tag>
          <Tag icon={<DashboardOutlined />} color="default">
            {fmtMaybe(cpu.powerW, (v) => `${v.toFixed(1)} W`)}
          </Tag>
          <Tag icon={<PercentageOutlined />} color="default">
            总占用 {cpu.totalUsage.toFixed(1)}%
          </Tag>
        </div>
      </div>

      {/* 核心列表 — 用进度条可视化 */}
      {perCoreUsage.length > 0 && (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))",
            gap: "6px 12px",
          }}
        >
          {perCoreUsage.map((usage, i) => {
            const temp = perCoreTemps[i] ?? null;
            return (
              <div
                key={i}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  padding: "4px 8px",
                  borderRadius: 6,
                  background: "#f9fafb",
                }}
              >
                <Text
                  type="secondary"
                  style={{
                    fontSize: 11,
                    width: 44,
                    flexShrink: 0,
                    fontVariantNumeric: "tabular-nums",
                  }}
                >
                  #{i}
                </Text>
                <Progress
                  percent={Math.round(usage)}
                  size="small"
                  strokeColor={usageColor(usage)}
                  showInfo={false}
                  style={{ flex: 1, margin: 0 }}
                />
                <Text
                  style={{
                    fontSize: 11,
                    width: 32,
                    textAlign: "right",
                    fontVariantNumeric: "tabular-nums",
                    color: usageColor(usage),
                    fontWeight: 600,
                  }}
                >
                  {usage.toFixed(0)}%
                </Text>
                <Text
                  style={{
                    fontSize: 11,
                    width: 36,
                    textAlign: "right",
                    fontVariantNumeric: "tabular-nums",
                    color: tempColor(temp),
                    fontWeight: 500,
                  }}
                >
                  {temp != null ? `${temp.toFixed(0)}°` : "—"}
                </Text>
              </div>
            );
          })}
        </div>
      )}
    </Space>
  );
}
