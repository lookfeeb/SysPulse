import { Card, Col, Row, Tooltip, Typography } from "antd";
import { FireOutlined, ThunderboltOutlined, HddOutlined, DashboardOutlined } from "@ant-design/icons";
import type { HwSnapshot } from "@/bindings";
import { maxValid } from "@/utils/format";

const { Text } = Typography;

function TempCard({
  icon,
  label,
  source,
  tempC,
  color,
}: {
  icon: React.ReactNode;
  label: string;
  source: string;
  tempC: number | null;
  color: string;
}) {
  const val = tempC != null ? `${Math.round(tempC)}°C` : "—";
  const hot = tempC != null && tempC >= 80;
  const warm = tempC != null && tempC >= 65;
  const valueColor = hot ? "#ef4444" : warm ? "#f97316" : color;

  return (
    <Tooltip title={source}>
      <Card
        size="small"
        style={{ height: "100%", borderRadius: 10 }}
        styles={{ body: { padding: "14px 16px" } }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
          <span style={{ color: valueColor, fontSize: 16 }}>{icon}</span>
          <Text type="secondary" style={{ fontSize: 12 }}>{label}</Text>
        </div>
        <div style={{ fontSize: 28, fontWeight: 700, color: valueColor, lineHeight: 1 }}>
          {val}
        </div>
        <Text type="secondary" style={{ fontSize: 11, marginTop: 4, display: "block" }}>
          {source}
        </Text>
      </Card>
    </Tooltip>
  );
}

function FanCard({ rpm }: { rpm: number }) {
  return (
    <Card
      size="small"
      style={{ height: "100%", borderRadius: 10 }}
      styles={{ body: { padding: "14px 16px" } }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
        <span style={{ color: "#3388cc", fontSize: 16 }}><DashboardOutlined /></span>
        <Text type="secondary" style={{ fontSize: 12 }}>最高风扇</Text>
      </div>
      <div style={{ fontSize: 28, fontWeight: 700, color: "#3388cc", lineHeight: 1 }}>
        {rpm > 0 ? rpm : "—"}
      </div>
      {rpm > 0 && (
        <Text type="secondary" style={{ fontSize: 11, marginTop: 4, display: "block" }}>
          RPM
        </Text>
      )}
    </Card>
  );
}

export default function SummaryCards({ snap }: { snap: HwSnapshot | null }) {
  const cpuTemp = snap?.cpu?.packageTempC ?? null;
  const gpus = snap?.gpus ?? [];
  const disks = snap?.disks ?? [];
  const fans = snap?.fans ?? [];
  const mbTemps = snap?.motherboard?.temperaturesC ?? [];

  const maxGpuTemp = maxValid(gpus.map((g) => g.tempC), { positiveOnly: true });
  const maxDiskTemp = maxValid(disks.map((d) => d.tempC), { positiveOnly: true });
  const hottestDisk = [...disks].sort((a, b) => (b.tempC ?? 0) - (a.tempC ?? 0))[0];

  // 主板最高温度（排除 0°C 的无效传感器）
  const maxMbTemp = maxValid(mbTemps.map((t) => t.value), { positiveOnly: true });

  // 找到主板最热传感器的名称
  const hotMbSensor = mbTemps
    .filter((t) => t.value > 0 && Number.isFinite(t.value))
    .sort((a, b) => b.value - a.value)[0];

  const maxFan = fans.reduce((m, f) => Math.max(m, f.rpm ?? 0), 0);

  // GPU 名称
  const gpuName = gpus[0]?.name?.split(" ").slice(-3).join(" ") ?? "GPU";

  return (
    <Row gutter={[12, 12]}>
      <Col xs={12} sm={6}>
        <TempCard
          icon={<FireOutlined />}
          label="CPU 温度"
          source={snap?.cpu?.name?.split("(R)").join("").trim().split(" ").slice(-3).join(" ") ?? "CPU Package"}
          tempC={cpuTemp}
          color="#f97316"
        />
      </Col>
      <Col xs={12} sm={6}>
        <TempCard
          icon={<ThunderboltOutlined />}
          label="GPU 温度"
          source={gpus.length > 0 ? gpuName : "无 GPU 数据"}
          tempC={maxGpuTemp}
          color="#8b5cf6"
        />
      </Col>
      <Col xs={12} sm={6}>
        <TempCard
          icon={<HddOutlined />}
          label={maxDiskTemp != null && (maxMbTemp == null || maxDiskTemp >= (maxMbTemp ?? 0)) ? "硬盘温度" : "主板温度"}
          source={
            maxDiskTemp != null && (maxMbTemp == null || maxDiskTemp >= (maxMbTemp ?? 0))
              ? (hottestDisk?.model?.split(" ").slice(0, 3).join(" ") ?? "硬盘")
              : (hotMbSensor?.name ?? snap?.motherboard?.model ?? "主板")
          }
          tempC={
            maxDiskTemp != null && (maxMbTemp == null || maxDiskTemp >= (maxMbTemp ?? 0))
              ? maxDiskTemp
              : maxMbTemp
          }
          color="#10b981"
        />
      </Col>
      <Col xs={12} sm={6}>
        <FanCard rpm={maxFan} />
      </Col>
    </Row>
  );
}
