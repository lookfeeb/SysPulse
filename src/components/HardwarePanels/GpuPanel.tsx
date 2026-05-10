import { Progress, Space, Tag, Typography } from "antd";
import {
  FireOutlined,
  ThunderboltOutlined,
  DashboardOutlined,
} from "@ant-design/icons";
import type { GpuHw } from "@/bindings";

const { Text } = Typography;

function tempColor(t: number | null | undefined): string {
  if (t == null) return "#d1d5db";
  if (t >= 80) return "#ef4444";
  if (t >= 65) return "#f97316";
  if (t >= 50) return "#eab308";
  return "#22c55e";
}

export default function GpuPanel({ gpus }: { gpus: GpuHw[] }) {
  return (
    <Space direction="vertical" style={{ width: "100%" }} size={12}>
      {gpus.map((gpu) => (
        <div
          key={gpu.index}
          style={{
            background: "#f9fafb",
            borderRadius: 8,
            padding: "12px 14px",
            border: "1px solid #f0f0f0",
          }}
        >
          {/* 型号 + 厂商 */}
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
            <Text strong style={{ fontSize: 13 }}>{gpu.name}</Text>
            <Tag style={{ fontSize: 11 }}>{gpu.vendor}</Tag>
          </div>

          {/* 指标行 */}
          <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 16px", marginBottom: 10 }}>
            <Tag icon={<DashboardOutlined />} color="blue">
              占用 {gpu.usagePercent != null ? `${gpu.usagePercent.toFixed(0)}%` : "—"}
            </Tag>
            <Tag icon={<FireOutlined />} color={tempColor(gpu.tempC)}>
              {gpu.tempC != null ? `${gpu.tempC.toFixed(0)}°C` : "—"}
            </Tag>
            <Tag icon={<ThunderboltOutlined />} color="default">
              {gpu.powerW != null ? `${gpu.powerW.toFixed(1)} W` : "—"}
            </Tag>
            {gpu.fanRpm != null && (
              <Tag color="default">
                风扇 {gpu.fanRpm.toFixed(0)} RPM
                {gpu.fanPwm != null ? ` / ${gpu.fanPwm.toFixed(0)}%` : ""}
              </Tag>
            )}
          </div>

          {/* 显存进度条 */}
          {gpu.memUsedMb != null && gpu.memTotalMb != null && (
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <Text type="secondary" style={{ fontSize: 11, flexShrink: 0 }}>显存</Text>
              <Progress
                percent={Math.round((gpu.memUsedMb / gpu.memTotalMb) * 100)}
                size="small"
                strokeColor="#8b5cf6"
                showInfo={false}
                style={{ flex: 1, margin: 0 }}
              />
              <Text style={{ fontSize: 11, fontVariantNumeric: "tabular-nums", flexShrink: 0 }}>
                {gpu.memUsedMb.toFixed(0)} / {gpu.memTotalMb.toFixed(0)} MB
              </Text>
            </div>
          )}
        </div>
      ))}
    </Space>
  );
}
