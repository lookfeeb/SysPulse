import { Progress, Space, Tag, Typography } from "antd";
import { DatabaseOutlined } from "@ant-design/icons";
import type { MemoryHw } from "@/bindings";
import { fmtBytesCompact, fmtMaybe } from "@/utils/format";

const { Text } = Typography;

export default function MemoryPanel({ mem }: { mem: MemoryHw }) {
  const modules = mem.modules ?? [];
  const usedPct = mem.usedPercent;

  return (
    <Space vertical style={{ width: "100%" }} size={12}>
      {/* 概览 */}
      <div
        style={{
          background: "#f9fafb",
          borderRadius: 8,
          padding: "12px 14px",
          border: "1px solid #f0f0f0",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10 }}>
          <Text type="secondary" style={{ fontSize: 12, flexShrink: 0 }}>使用率</Text>
          <Progress
            percent={Math.round(usedPct)}
            size="small"
            strokeColor={usedPct >= 90 ? "#ef4444" : usedPct >= 70 ? "#f97316" : "#3388cc"}
            showInfo={false}
            style={{ flex: 1, margin: 0 }}
          />
          <Text style={{ fontSize: 12, fontVariantNumeric: "tabular-nums", fontWeight: 600, flexShrink: 0 }}>
            {usedPct.toFixed(1)}%
          </Text>
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 12px" }}>
          <Tag color="blue">
            {fmtBytesCompact(mem.usedBytes)} / {fmtBytesCompact(mem.totalBytes)}
          </Tag>
          <Tag color="default">
            频率 {fmtMaybe(mem.frequencyMhz, (v) => `${v} MT/s`)}
          </Tag>
          <Tag color="default">
            通道 {fmtMaybe(mem.channels, (v) => String(v))}
          </Tag>
          {modules.length > 0 && (
            <Tag color="default">
              {modules.length} 条内存
            </Tag>
          )}
        </div>
      </div>

      {/* 内存条列表 */}
      {modules.length > 0 && (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))",
            gap: 8,
          }}
        >
          {modules.map((m) => (
            <div
              key={m.slot}
              style={{
                background: "#f9fafb",
                borderRadius: 8,
                padding: "10px 14px",
                border: "1px solid #f0f0f0",
                display: "flex",
                alignItems: "center",
                gap: 10,
              }}
            >
              <DatabaseOutlined style={{ fontSize: 16, color: "#6b7280", flexShrink: 0 }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <Text strong style={{ fontSize: 12, display: "block" }}>
                  {m.slot}
                </Text>
                <Text type="secondary" style={{ fontSize: 11 }}>
                  {m.manufacturer} · {m.partNumber}
                </Text>
              </div>
              <div style={{ textAlign: "right", flexShrink: 0 }}>
                <Text style={{ fontSize: 13, fontWeight: 700, color: "#3388cc", display: "block" }}>
                  {fmtBytesCompact(m.capacityBytes)}
                </Text>
                <Text type="secondary" style={{ fontSize: 11 }}>
                  {m.speedMtps != null ? `${m.speedMtps} MT/s` : "—"}
                </Text>
              </div>
            </div>
          ))}
        </div>
      )}
    </Space>
  );
}
