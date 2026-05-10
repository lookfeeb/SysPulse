import { useEffect, useState } from "react";
import { Alert, App as AntdApp, Card, Space, Typography } from "antd";
import { useHwStore, bindHwEvents } from "@/stores/hwStore";
import { isAdmin, resetAllFanControls } from "@/ipc";
import SummaryCards from "@/components/HardwarePanels/SummaryCards";
import SensorList from "@/components/HardwarePanels/SensorList";

export default function HardwarePage() {
  const { message } = AntdApp.useApp();
  const snap = useHwStore((s) => s.current);
  const helperStatus = useHwStore((s) => s.helperStatus);
  const helperReason = useHwStore((s) => s.helperReason);
  const fanControl = useHwStore((s) => s.fanControl);
  const setFanControl = useHwStore((s) => s.setFanControl);
  const prime = useHwStore((s) => s.prime);

  const [admin, setAdmin] = useState(false);

  useEffect(() => {
    void bindHwEvents();
    void prime();
    void isAdmin().then(setAdmin).catch(() => setAdmin(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function resetAllFans() {
    try {
      const next = await resetAllFanControls();
      setFanControl(next);
      void message.success("已恢复全部风扇 BIOS 控制");
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    }
  }

  const fanControlDisabled = helperStatus !== "running";

  return (
    <Space direction="vertical" size={16} style={{ width: "100%" }}>
      {/* 状态提示 */}
      {helperStatus === "unavailable" && (
        <Alert
          type="error"
          showIcon
          message="hw-helper 不可用"
          description={
            helperReason ||
            "未找到 hw-helper.exe。请在仓库根运行 scripts/build-helper.ps1，然后重启程序。"
          }
          style={{ borderRadius: 8 }}
        />
      )}
      {helperStatus === "restarting" && (
        <Alert type="warning" showIcon message="hw-helper 正在重启…" style={{ borderRadius: 8 }} />
      )}
      {helperStatus === "starting" && (
        <Alert type="info" showIcon message="hw-helper 正在启动…" style={{ borderRadius: 8 }} />
      )}
      {fanControl.fuseHold && (
        <Alert
          type="error"
          showIcon
          message="风扇控制已熔断"
          description={fanControl.fuseReason || "已恢复 BIOS 控制，温度下降后可重新启用。"}
          action={
            <Typography.Link onClick={() => void resetAllFans()}>
              恢复 BIOS 控制
            </Typography.Link>
          }
          style={{ borderRadius: 8 }}
        />
      )}

      {/* 概览卡片 */}
      <Card
        size="small"
        title={
          <span style={{ fontWeight: 600, fontSize: 13, color: "#374151" }}>
            硬件概览
          </span>
        }
        style={{ borderRadius: 10 }}
        styles={{ body: { padding: "12px 16px" } }}
      >
        <SummaryCards snap={snap} />
      </Card>

      {/* 传感器详情 */}
      <SensorList snap={snap} isAdmin={admin} fanControlDisabled={fanControlDisabled} />
    </Space>
  );
}
