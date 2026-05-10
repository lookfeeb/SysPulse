import { useMemo } from "react";
import { Card, Collapse, Empty } from "antd";
import type { HwSnapshot } from "@/bindings";
import CpuPanel from "./CpuPanel";
import GpuPanel from "./GpuPanel";
import MemoryPanel from "./MemoryPanel";
import DiskPanel from "./DiskPanel";
import MotherboardPanel from "./MotherboardPanel";
import FanPanel from "./FanPanel";

export default function SensorList({
  snap,
  isAdmin,
  fanControlDisabled,
}: {
  snap: HwSnapshot | null;
  isAdmin: boolean;
  fanControlDisabled: boolean;
}) {
  const items = useMemo(() => {
    if (!snap) return [];
    const out: { key: string; label: string; node: React.ReactNode }[] = [];
    const gpus = snap.gpus ?? [];
    const disks = snap.disks ?? [];
    const fans = snap.fans ?? [];
    // 风扇放第一个
    if (fans.length)
      out.push({
        key: "fan",
        label: "风扇控制",
        node: <FanPanel fans={fans} isAdmin={isAdmin} disabled={fanControlDisabled} />,
      });
    if (snap.cpu) out.push({ key: "cpu", label: "CPU", node: <CpuPanel cpu={snap.cpu} /> });
    if (gpus.length)
      out.push({ key: "gpu", label: `GPU (${gpus.length})`, node: <GpuPanel gpus={gpus} /> });
    if (snap.memory) {
      const modCount = snap.memory.modules?.length ?? 0;
      out.push({
        key: "mem",
        label: modCount > 0 ? `内存 (${modCount})` : "内存",
        node: <MemoryPanel mem={snap.memory} />,
      });
    }
    if (disks.length)
      out.push({ key: "disk", label: `硬盘 (${disks.length})`, node: <DiskPanel disks={disks} /> });
    if (snap.motherboard)
      out.push({ key: "mb", label: "主板", node: <MotherboardPanel mb={snap.motherboard} /> });
    return out;
  }, [fanControlDisabled, isAdmin, snap]);

  if (!snap) {
    return (
      <Card
        title={<span style={{ fontWeight: 600, fontSize: 13, color: "#374151" }}>传感器详情</span>}
        style={{ borderRadius: 10 }}
      >
        <Empty description="等待 hw-helper 第一帧…" />
      </Card>
    );
  }

  return (
    <Card
      title={<span style={{ fontWeight: 600, fontSize: 13, color: "#374151" }}>传感器详情</span>}
      style={{ borderRadius: 10 }}
      styles={{ body: { padding: "12px 16px" } }}
    >
      <Collapse
        size="small"
        style={{ background: "transparent", border: "none" }}
        items={items.map((i) => ({
          key: i.key,
          label: <span style={{ fontWeight: 500 }}>{i.label}</span>,
          children: i.node,
          style: {
            marginBottom: 6,
            borderRadius: 8,
            border: "1px solid #e8eaed",
            overflow: "hidden",
          },
        }))}
        defaultActiveKey={[]}
      />
    </Card>
  );
}
