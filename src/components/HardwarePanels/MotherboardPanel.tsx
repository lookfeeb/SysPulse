import { Space, Tag, Typography } from "antd";
import { FireOutlined, ThunderboltOutlined } from "@ant-design/icons";
import type { MotherboardHw } from "@/bindings";

const { Text } = Typography;

function tempColor(t: number): string {
  if (t >= 80) return "#ef4444";
  if (t >= 60) return "#f97316";
  if (t >= 45) return "#eab308";
  if (t <= 0) return "#d1d5db";
  return "#22c55e";
}

export default function MotherboardPanel({ mb }: { mb: MotherboardHw }) {
  const temperatures = (mb.temperaturesC ?? []).filter((t) => t.value > 0);
  const voltages = mb.voltagesV ?? [];

  return (
    <Space direction="vertical" style={{ width: "100%" }} size={12}>
      {/* 标题 */}
      <div>
        <Text strong style={{ fontSize: 13 }}>{mb.model}</Text>
        {mb.superIo && (
          <Text type="secondary" style={{ display: "block", fontSize: 11, marginTop: 2 }}>
            Super I/O: {mb.superIo}
          </Text>
        )}
      </div>

      {/* 温度 */}
      {temperatures.length > 0 && (
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 6 }}>
            <FireOutlined style={{ color: "#f97316", fontSize: 12 }} />
            <Text type="secondary" style={{ fontSize: 12 }}>温度</Text>
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
            {temperatures.map((t) => (
              <Tag
                key={t.identifier ?? t.name}
                color={tempColor(t.value)}
                style={{ fontSize: 11, borderRadius: 4 }}
              >
                {t.name} {t.value.toFixed(0)}°C
              </Tag>
            ))}
          </div>
        </div>
      )}

      {/* 电压 */}
      {voltages.length > 0 && (
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 6 }}>
            <ThunderboltOutlined style={{ color: "#3388cc", fontSize: 12 }} />
            <Text type="secondary" style={{ fontSize: 12 }}>电压</Text>
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
            {voltages.map((v) => (
              <Tag
                key={v.identifier ?? v.name}
                style={{ fontSize: 11, borderRadius: 4 }}
              >
                {v.name} {v.value.toFixed(2)}V
              </Tag>
            ))}
          </div>
        </div>
      )}
    </Space>
  );
}
