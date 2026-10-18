import { MAX_PROJECT_ROOTS } from "@/routes/cleanupSelection";
import { fmtBytes } from "@/utils/format";
import { FolderAddOutlined, RadarChartOutlined, DatabaseOutlined, ScanOutlined, StopOutlined, ThunderboltOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Popconfirm, Progress, Table } from "antd";
import CleanupDetailModal from "./cleanup/CleanupDetailModal";
import CleanupResultNotice from "./cleanup/CleanupResultNotice";
import "./cleanup/CleanupPage.css";
import ProjectRootsModal from "./cleanup/ProjectRootsModal";
import { useCleanupModel } from "./cleanup/useCleanupModel";
export default function CleanupPage() {
    const model = useCleanupModel();
    const { lastCleanResult, phase } = model;
    const { Text, canClean, categories, categoryColumns, categoryRows, cleanProgress, cleaning, cleanupCache, compactNumber, heroScanDetail, heroScanState, onClean, onScanButton, openDetail, progressMeta, projectRoots, scanButton, scanIncomplete, scanProgress, scanning, selectedAdvanced, selectedCaution, selectedFiles, selectedPathCount, setProjectManagerOpen, totalPathCount, totalSelected } = model;
    const totalSize = cleanupCache?.totalSizeBytes ?? 0;
    const selectedPercent = totalSize > 0 ? Math.min(100, totalSelected / totalSize * 100) : 0;
    return (<div className="cleanup-page">
      {scanIncomplete && categories.length > 0 && (<Alert type="warning" showIcon title="扫描结果不完整" description="请重新完成扫描后再清理。" />)}

      <section className="cleanup-overview" aria-label="磁盘清理概览">
        <header className="cleanup-overview-header">
          <div className="cleanup-overview-title"><span><DatabaseOutlined /></span><div><h1>磁盘清理</h1><p>检查缓存与临时文件，释放磁盘空间</p></div></div>
          <Button icon={<FolderAddOutlined />} disabled={scanning || cleaning} onClick={() => setProjectManagerOpen(true)}>
            项目目录 <span className="cleanup-scope-count">{projectRoots.length}/{MAX_PROJECT_ROOTS}</span>
          </Button>
        </header>
        <div className="cleanup-overview-body">
          <div className="cleanup-space-summary">
            <span className="cleanup-space-label">{scanning ? "正在统计空间" : "发现可清理空间"}</span>
            {cleanupCache ? <>
              <strong className="cleanup-space-value">{fmtBytes(totalSize)}</strong>
              <div className="cleanup-space-track" role="img" aria-label={`已选空间占比 ${Math.round(selectedPercent)}%`}><i style={{ width: `${selectedPercent}%` }} /></div>
              <div className="cleanup-space-caption"><span>已选 <b>{fmtBytes(totalSelected)}</b></span><span>{compactNumber(selectedPathCount)} 条路径 · {compactNumber(selectedFiles)} 个文件</span></div>
            </> : <div className="cleanup-space-empty">
              <span className="cleanup-space-empty-icon" aria-hidden="true"><ScanOutlined /></span>
              <div><strong>{scanning ? "正在发现可释放空间" : "让磁盘轻装上阵"}</strong><p>{scanning ? "扫描完成后显示空间统计" : "扫描后查看可释放的磁盘空间"}</p></div>
            </div>}
          </div>
          <div className="cleanup-scan-summary">
            <div className="cleanup-scan-state" aria-live="polite"><span className={`cleanup-state-dot${scanning || cleaning ? " is-busy" : scanIncomplete ? " is-warning" : ""}`} /><strong>{cleaning ? "清理进行中" : phase === "refreshing" ? "正在刷新结果" : scanning ? "扫描进行中" : cleanupCache ? "扫描快照" : "准备就绪"}</strong><span>{scanning || cleaning ? "后台运行中" : cleanupCache ? heroScanState : "等待首次扫描"}</span></div>
            <p className="cleanup-scan-description" title={heroScanDetail}>{scanning || cleaning ? heroScanDetail : "扫描与清理可在后台继续，切换页面不会中断。"}</p>
            <div className="cleanup-scan-metrics"><span><b>{compactNumber(scanning ? scanProgress?.scannedPaths ?? 0 : cleanupCache?.scannedPaths ?? 0)}</b> 已检查项</span><span><b>{cleanupCache ? `${(cleanupCache.durationMs / 1000).toFixed(1)} s` : "—"}</b> 扫描用时</span></div>
            <Button className="cleanup-scan-button" type="primary" danger={scanButton.danger} icon={scanning ? <StopOutlined /> : <ScanOutlined />} disabled={scanButton.disabled} onClick={onScanButton}>{scanButton.label}</Button>
          </div>
        </div>
      </section>

      {cleanProgress && !(cleanProgress.done && lastCleanResult?.errors.length) && (<Card size="small" style={{ borderRadius: 10 }} styles={{ body: { padding: "12px 16px" } }}>
          <div className="cleanup-clean-status">
            <div style={{ minWidth: 0 }}>
              <Text strong>{cleanProgress.done ? lastCleanResult?.errors.length ? "清理结束，部分内容已跳过" : "清理完成" : cleanProgress.currentCategory || "正在清理"}</Text>
              {cleanProgress.currentPath && <Text type="secondary" ellipsis title={cleanProgress.currentPath} style={{ display: "block", fontSize: 11 }}>{cleanProgress.currentPath}</Text>}
            </div>
            <div style={{ textAlign: "right", whiteSpace: "nowrap" }}>
              <Text strong style={{ color: "#cf1322" }}>{fmtBytes(cleanProgress.freedBytes)}</Text>
              <Text type="secondary" style={{ display: "block", fontSize: 11 }}>{compactNumber(cleanProgress.deletedFiles)} 文件</Text>
            </div>
          </div>
          <Progress percent={cleanProgress.percent} status={cleanProgress.done ? lastCleanResult?.errors.length ? "normal" : "success" : "active"} size="small" style={{ marginTop: 8 }}/>
        </Card>)}

      {lastCleanResult && <CleanupResultNotice result={lastCleanResult} />}

      {(scanning || categories.length > 0) && (<Card className={`cleanup-results-card${scanning ? " is-scanning" : ""}`} size="small" title={<div className="cleanup-results-heading">
              <span className="cleanup-results-heading-icon"><RadarChartOutlined /></span>
              <div>
                <strong>{scanning ? "扫描进度" : "扫描结果"}</strong>
                <span>
                  {scanning
                    ? `已检查 ${compactNumber(scanProgress?.scannedPaths ?? 0)} 项 · 已派发 ${compactNumber((scanProgress?.scoutTasks ?? 0) + (scanProgress?.engineerTasks ?? 0))} 个任务`
                    : `${categoryRows.length} 个类别 · ${totalPathCount} 条路径 · 点击查看明细`}
                </span>
              </div>
            </div>} extra={(<div className="cleanup-results-toolbar">
              {categories.length > 0 && (<div className="cleanup-results-toolbar-group cleanup-results-toolbar-selection">
                  <Popconfirm title="确认清理选中路径" description={selectedAdvanced
                        ? `包含高级维护项，将清理 ${fmtBytes(totalSelected)}；请先查看路径和命中规则。`
                        : selectedCaution
                            ? `包含可再生成缓存，将清理 ${fmtBytes(totalSelected)}；被占用或扫描后变化的文件会保留。`
                            : `将清理 ${fmtBytes(totalSelected)}，操作不可撤销。`} onConfirm={() => void onClean()} disabled={!canClean}>
                    <Button type="primary" danger size="small" icon={<ThunderboltOutlined />} loading={cleaning} disabled={!canClean}>
                      清理 {fmtBytes(totalSelected)}
                    </Button>
                  </Popconfirm>
                </div>)}
            </div>)} style={{ borderRadius: 10 }} styles={{ body: { padding: categories.length > 0 ? 0 : 16 } }}>
          {categories.length > 0 ? (<Table className="cleanup-result-table" rowKey={(row) => row.category.id} columns={categoryColumns} dataSource={categoryRows} pagination={false} size="small" tableLayout="fixed" scroll={{ x: 540 }} rowClassName={(row) => row.selection.checked || row.selection.indeterminate ? "is-selected" : ""} onRow={(row) => ({ onClick: () => openDetail(row.category), style: { cursor: "pointer" } })}/>) : (<div className="cleanup-live-results">
              <div className="cleanup-live-results-stage">
                <span className="cleanup-live-results-pulse"/>
                <div>
                  <strong>{progressMeta.label}</strong>
                  <span>{scanProgress?.phase ?? "正在建立扫描任务"}</span>
                </div>
                <b>{compactNumber(scanProgress?.scannedPaths ?? 0)}</b>
                <small>已检查项</small>
              </div>
              <div className="cleanup-live-results-path" title={scanProgress?.currentPath ?? undefined}>
                {scanProgress?.currentPath ?? "正在检查缓存路径，扫描结束后按类别展示结果"}
              </div>
              <div className="cleanup-live-results-tasks">
                <span><b>{compactNumber(scanProgress?.scoutTasks ?? 0)}</b><small>目录发现</small></span>
                <span><b>{compactNumber(scanProgress?.engineerTasks ?? 0)}</b><small>缓存检查</small></span>
                <span><b>{compactNumber(scanProgress?.skippedPaths ?? 0)}</b><small>跳过异常</small></span>
              </div>
            </div>)}
        </Card>)}

      <CleanupDetailModal model={model}/>

      <ProjectRootsModal model={model}/>
    </div>);
}
