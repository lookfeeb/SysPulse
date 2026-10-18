import { useEffect } from "react";
import { Alert, App } from "antd";
import { Link, useLocation } from "react-router-dom";
import { useCleanupStore } from "@/stores/cleanupStore";

export default function CleanupTaskStatus() {
  const phase = useCleanupStore((state) => state.phase);
  const scanProgress = useCleanupStore((state) => state.scanProgress);
  const cleanProgress = useCleanupStore((state) => state.cleanProgress);
  const notice = useCleanupStore((state) => state.notice);
  const { message } = App.useApp();
  const { pathname } = useLocation();
  useEffect(() => {
    if (notice) void message[notice.type]({ content: notice.text, key: "cleanup-task", duration: 4 });
  }, [message, notice]);
  if (phase === "idle" || pathname === "/cleanup") return null;
  return <Alert type="info" showIcon role="status" aria-live="polite" style={{ marginBottom: 16 }}
    title={phase === "cleaning" ? "磁盘清理正在后台进行" : phase === "refreshing" ? "正在后台刷新清理结果" : "磁盘扫描正在后台进行"}
    description={phase === "cleaning" ? `已处理 ${cleanProgress?.processedItems ?? 0} 项`
      : `已检查 ${(scanProgress?.scannedPaths ?? 0).toLocaleString()} 项，可继续使用其他页面`}
    action={<Link to="/cleanup">查看进度</Link>} />;
}
