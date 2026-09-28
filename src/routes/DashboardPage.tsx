import { NetworkSparkline } from "./dashboard/charts";
import { useHwStore } from "@/stores/hwStore";
import { useLiveStore } from "@/stores/liveStore";
import { fmtBytes, fmtFreq, fmtTemp } from "@/utils/format";
import { Card, Typography } from "antd";
import { useTranslation } from "react-i18next";
import { DiskCard, GpuCard, MetricCard, NetworkCard } from './dashboard/cards';
import './DashboardPage.css';
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
        className="dashboard-metric-grid"
      >
        <MetricCard
          title="CPU"
          value={cpu.usagePercent}
          precision={1}
          suffix="%"
          lines={[
            `温度 ${fmtTemp(hwCpu?.packageTempC)} · 频率 ${fmtFreq(hwCpu?.frequencyMhz)}`,
            `${cpu.model || hwCpu?.name || "--"}${cpu.physicalCores ? ` · ${cpu.physicalCores} ${t("dashboard.cores")}` : ""
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
        <GpuCard gpus={gpus} />
        <NetworkCard
          down={net.bytesRecvPerSec}
          up={net.bytesSentPerSec}
          totalDown={net.bytesRecvTotal}
          totalUp={net.bytesSentTotal}
        />
        <DiskCard disks={disks} />
      </div>

      <Card
        className="dashboard-history-card"
        title={t("dashboard.live60s")}
        style={{ marginTop: 16, borderRadius: 12 }}
      >
        <NetworkSparkline history={history} hwHistory={hwHistory} />
      </Card>
    </div>
  );
}
