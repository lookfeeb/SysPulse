import { MAX_PROJECT_ROOTS } from "@/routes/cleanupSelection";
import { fmtBytes } from "@/utils/format";
import { ExportOutlined, FolderAddOutlined, RadarChartOutlined, ScanOutlined, StopOutlined, ThunderboltOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Checkbox, Collapse, Popconfirm, Progress, Space, Spin, Table, Tooltip } from "antd";
import CleanupDetailModal from "./cleanup/CleanupDetailModal";
import CleanupResultNotice from "./cleanup/CleanupResultNotice";
import "./cleanup/CleanupPage.css";
import ProjectRootsModal from "./cleanup/ProjectRootsModal";
import { useCleanupModel } from "./cleanup/useCleanupModel";
export default function CleanupPage() {
    const model = useCleanupModel();
    const { lastCleanResult, phase } = model;
    const { Text, allSafeSelected, busy, canClean, canExport, categories, categoryColumns, categoryRows, cleanProgress, cleaning, cleanupCache, compactNumber, exporting, heroEngineerTasks, heroScanDetail, heroScanState, heroScoutTasks, onClean, onExport, onScanButton, openDetail, partiallySafeSelected, progressMeta, projectRoots, safeCategories, scanButton, scanExpired, scanIncomplete, scanProgress, scanning, selectedAdvanced, selectedCaution, selectedFiles, selectedPathCount, setCategoriesChecked, setProjectManagerOpen, totalPathCount, totalSelected } = model;
    return (<Space vertical size={16} style={{ width: "100%" }}>
      {(scanExpired || scanIncomplete) && categories.length > 0 && (<Alert type="warning" showIcon title={scanExpired ? "扫描结果已过期" : "扫描结果不完整"} description="当前结果可导出审核，但不能直接清理；请重新完成扫描。" style={{ borderRadius: 8 }}/>)}

      <section className="cleanup-hero">
        <div className="cleanup-hero-copy">
          <span className="cleanup-hero-eyebrow">DISK HYGIENE · READ ONLY</span>
          <h1>先看清楚，再释放空间</h1>
          <p>检查用户目录、固定磁盘和开发工具缓存。扫描与清理会在后台继续，切换页面后可随时回来查看；清理由你确认后执行。</p>

          <div className={`cleanup-hero-status${scanning ? " is-scanning" : ""}`}>
            <div className="cleanup-hero-status-primary">
              <span className={`cleanup-hero-dot${scanning ? " is-scanning" : cleanupCache && (!cleanupCache.complete || cleanupCache.cancelled) ? " is-warning" : ""}`}/>
              <div>
                <span>{cleaning ? "清理进行中" : phase === "refreshing" ? "清理已结束，正在刷新扫描" : scanning ? "扫描进行中" : cleanupCache ? "扫描快照" : "等待扫描"}</span>
                <strong>{heroScanState}</strong>
              </div>
              {scanning && <Spin size="small"/>}
            </div>
            <div className="cleanup-hero-status-path" title={heroScanDetail}>{heroScanDetail}</div>
            <div className="cleanup-hero-status-counts">
              <span><b>{compactNumber(heroScoutTasks)}</b> 侦察任务</span>
              <span><b>{compactNumber(heroEngineerTasks)}</b> 工兵任务</span>
            </div>
          </div>

          <div className="cleanup-hero-metrics">
            <div><span>已选择</span><strong>{fmtBytes(totalSelected)}</strong><small>{compactNumber(selectedPathCount)} 条路径</small></div>
            <div><span>验证路径</span><strong>{compactNumber(totalPathCount)}</strong><small>{compactNumber(selectedFiles)} 个已选文件</small></div>
            <div><span>扫描用时</span><strong>{cleanupCache ? `${(cleanupCache.durationMs / 1000).toFixed(1)} s` : "—"}</strong><small>{cleanupCache ? `${compactNumber(cleanupCache.scannedPaths)} 项` : scanning ? "正在扫描，可切换页面" : "等待扫描"}</small></div>
            <button type="button" className="cleanup-hero-metric-action" disabled={scanning || cleaning} onClick={() => setProjectManagerOpen(true)} title="管理项目扫描根；所有固定磁盘分区仍会自动进入快速侦察">
              <span>扫描范围</span><strong>{projectRoots.length} / {MAX_PROJECT_ROOTS}</strong><small>管理扫描根 · 全固定盘</small>
            </button>
          </div>
        </div>

        <div className="cleanup-hero-visual">
          <div className={`cleanup-hero-gauge${scanning ? " is-scanning" : ""}`}>
            <div className="cleanup-hero-gauge-inner">
              <strong>{fmtBytes(cleanupCache?.totalSizeBytes ?? 0)}</strong>
              <span>可释放空间</span>
            </div>
          </div>
          <Tooltip title={scanButton.label}>
            <Button className="cleanup-hero-scan" type="primary" danger={scanButton.danger} shape="circle" icon={scanning ? <StopOutlined /> : <ScanOutlined />} disabled={scanButton.disabled} onClick={onScanButton} aria-label={scanButton.label}/>
          </Tooltip>
          <span className="cleanup-hero-scan-label">{scanButton.label}</span>
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

      {!!cleanupCache?.warnings.length && <Alert type="warning" showIcon title="扫描范围说明"
        description={<Collapse ghost items={[{ key: "coverage", label: `查看 ${cleanupCache.warnings.length} 条扫描说明`,
          children: <div style={{ maxHeight: 240, overflow: "auto", overflowWrap: "anywhere" }}>
            {cleanupCache.warnings.map((warning, index) => <p key={index}>{warning}</p>)}
          </div> }]} />} />}

      {(scanning || categories.length > 0) && (<Card className={`cleanup-results-card${scanning ? " is-scanning" : ""}`} size="small" title={<div className="cleanup-results-heading">
              <span className="cleanup-results-heading-icon"><RadarChartOutlined /></span>
              <div>
                <strong>{scanning ? "扫描进度" : "扫描结果"}</strong>
                <span>
                  {scanning
                    ? `已检查 ${compactNumber(scanProgress?.scannedPaths ?? 0)} 项 · 已派发 ${compactNumber(heroScoutTasks + heroEngineerTasks)} 个任务`
                    : `${categories.length} 个类别 · ${totalPathCount} 条已验证路径`}
                </span>
              </div>
            </div>} extra={(<div className="cleanup-results-toolbar">
              <div className="cleanup-results-toolbar-group cleanup-results-toolbar-scope">
                <Button className="cleanup-results-scope-button" size="small" icon={<FolderAddOutlined />} disabled={scanning || cleaning} onClick={() => setProjectManagerOpen(true)}>
                  项目目录 <b>{projectRoots.length}/{MAX_PROJECT_ROOTS}</b>
                </Button>
                <Tooltip title={canExport ? "仅导出当前勾选的具体缓存路径" : "扫描并勾选路径后可导出 JSON 明细"}>
                  <Button className="cleanup-results-export-button" size="small" icon={<ExportOutlined />} loading={exporting} disabled={!canExport} onClick={() => void onExport()}>
                    导出明细
                  </Button>
                </Tooltip>
              </div>
              {categories.length > 0 && (<div className="cleanup-results-toolbar-group cleanup-results-toolbar-selection">
                  <Checkbox checked={allSafeSelected} indeterminate={partiallySafeSelected} disabled={busy || safeCategories.length === 0} onChange={() => setCategoriesChecked(safeCategories, !allSafeSelected)}>
                    全选安全项
                  </Checkbox>
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
          {categories.length > 0 ? (<Table className="cleanup-result-table" rowKey={(row) => row.category.id} columns={categoryColumns} dataSource={categoryRows} pagination={false} size="small" tableLayout="fixed" scroll={{ x: 720 }} onRow={(row) => ({ onClick: () => openDetail(row.category), style: { cursor: "pointer" } })}/>) : (<div className="cleanup-live-results">
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
                <span><b>{compactNumber(heroScoutTasks)}</b><small>侦察任务</small></span>
                <span><b>{compactNumber(heroEngineerTasks)}</b><small>工兵任务</small></span>
                <span><b>{compactNumber(scanProgress?.skippedPaths ?? 0)}</b><small>跳过异常</small></span>
              </div>
            </div>)}
        </Card>)}

      <CleanupDetailModal model={model}/>

      <ProjectRootsModal model={model}/>
    </Space>);
}
