import type { CleanResult } from "@/bindings";
import { fmtBytes } from "@/utils/format";
import { CheckCircleOutlined, DownOutlined, ExclamationCircleOutlined } from "@ant-design/icons";

export default function CleanupResultNotice({ result }: { result: CleanResult }) {
  const retained = result.skippedFiles > 0;
  return <section className="cleanup-report" aria-label="清理结果" aria-live="polite">
    <header>
      {retained || result.cancelled ? <ExclamationCircleOutlined /> : <CheckCircleOutlined />}
      <strong>{result.cancelled ? "清理已停止" : "清理完成"}</strong>
      <span>已清理 <b>{fmtBytes(result.freedBytes)}</b></span>
      <span>{result.deletedFiles.toLocaleString()} 个文件</span>
      {retained && <span>保留 {result.skippedFiles.toLocaleString()} 个文件</span>}
    </header>
    <p>{result.cancelled ? "已完成的清理已保留，剩余文件未处理。" : "使用中的文件和扫描后变化的文件已保留。"}再次清理前请重新扫描。</p>
    {result.paths.length > 0 && <details>
      <summary>查看各目录结果 <DownOutlined /></summary>
      <div className="cleanup-report-paths">
        {result.paths.map((path, index) => <article key={`${path.path}:${index}`}>
          <strong>{path.category}</strong>
          <div className="cleanup-report-path" title={path.path}>{path.path}</div>
          <div className="cleanup-report-counts">
            <span>已清理 {path.deletedFiles.toLocaleString()} 个 · {fmtBytes(path.freedBytes)}</span>
            {path.busyFiles > 0 && <span>使用中 {path.busyFiles.toLocaleString()}</span>}
            {path.changedFiles > 0 && <span>已变化 {path.changedFiles.toLocaleString()}</span>}
            {path.missingFiles > 0 && <span>已不存在 {path.missingFiles.toLocaleString()}</span>}
            {path.failedFiles > 0 && <span>无法清理 {path.failedFiles.toLocaleString()}</span>}
            {path.cancelledFiles > 0 && <span>未处理 {path.cancelledFiles.toLocaleString()}</span>}
          </div>
          {path.examples.length > 0 && <details><summary>原因</summary><ul>
            {path.examples.map((example, i) => <li key={i}>{example}</li>)}
          </ul></details>}
        </article>)}
      </div>
    </details>}
  </section>;
}
