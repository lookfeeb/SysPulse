import type { CleanResult } from "@/bindings";
import { fmtBytes } from "@/utils/format";
import { DownOutlined, ExclamationCircleOutlined } from "@ant-design/icons";

export default function CleanupResultNotice({ result }: { result: CleanResult }) {
  if (!result.errors.length) return null;

  return <details className="cleanup-result-notice">
    <summary className="cleanup-result-notice-summary">
      <ExclamationCircleOutlined className="cleanup-result-notice-icon" aria-hidden="true" />
      <span className="cleanup-result-notice-title">部分内容已跳过</span>
      <span className="cleanup-result-notice-freed">已释放 <strong>{fmtBytes(result.freedBytes)}</strong></span>
      <span className="cleanup-result-notice-count">{result.errors.length.toLocaleString()} 条说明</span>
      <span className="cleanup-result-notice-toggle">详情 <DownOutlined aria-hidden="true" /></span>
    </summary>
    <div className="cleanup-result-notice-body">
      <p>正在使用、扫描后变化或无权访问的文件会保留，其余可清理文件继续处理。</p>
      <ul tabIndex={0} aria-label="清理跳过原因">
        {result.errors.map((error, index) => <li key={index}>{error}</li>)}
      </ul>
    </div>
  </details>;
}
