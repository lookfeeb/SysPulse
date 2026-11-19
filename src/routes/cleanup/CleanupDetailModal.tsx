import { PROGRAMMING_CATEGORY_ID, cleanupCategorySelection, normalizePathKey } from "@/routes/cleanupSelection";
import { fmtBytes } from "@/utils/format";
import { displayPath } from "@/utils/path";
import { CheckCircleOutlined, CloseOutlined, CodeOutlined, DatabaseOutlined, FolderOpenOutlined, PlusOutlined } from "@ant-design/icons";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { App, Button, Checkbox, Empty, Modal, Tag } from "antd";
import type { CleanupModel } from "./useCleanupModel";
import "./CleanupModal.css";
import "./CleanupDetailModal.css";
export default function CleanupDetailModal({ model }: {
    model: CleanupModel;
}) {
    const { message } = App.useApp();
    const { activeDetailCategory, activeDetailSelection, busy, categoryIcon, closeDetail, compactNumber, detailAllSelection, detailCat, detailCategories, detailSelectedFiles, detailSelectedPaths, detailSelectedSize, detailToolchains, discoveredToolchains, excludedPaths, riskMeta, selected, setActiveToolchainId, setCategoriesChecked, sourceLabel, toggleCategory, togglePath, visibleDetailPaths } = model;
    return (<Modal title={null} closable={false} open={!!detailCat} onCancel={closeDetail} footer={null} width={detailCat?.childCategories ? 1040 : 760} centered className="cleanup-modal cleanup-review-modal" styles={{ container: { padding: 0, overflow: "hidden" }, body: { padding: 0 } }}>
        {detailCat && activeDetailCategory && (<div className="cleanup-review-shell cleanup-modal-shell">
            <header className="cleanup-modal-header">
              <span className="cleanup-modal-icon">
                {detailCat.id === PROGRAMMING_CATEGORY_ID ? <CodeOutlined /> : <DatabaseOutlined />}
              </span>
              <div className="cleanup-modal-heading">
                <h2>{detailCat.name}</h2>
                <p>{detailCat.childCategories
                ? "仅展示本次扫描实际发现缓存的工具链 · 保留可复用依赖目录"
                : detailCat.description}</p>
              </div>
              <Button type="text" className="cleanup-modal-close" icon={<CloseOutlined />} onClick={closeDetail} aria-label="关闭缓存审核"/>
            </header>

            <div className="cleanup-review-overview">
              <div className="cleanup-review-stat cleanup-review-stat--primary">
                <span>可审核缓存</span>
                <strong>{fmtBytes(detailCat.sizeBytes)}</strong>
              </div>
              <div className="cleanup-review-stat">
                <span>{detailCat.childCategories ? "已发现工具链" : "风险等级"}</span>
                <strong>{detailCat.childCategories ? `${discoveredToolchains} 个` : riskMeta(detailCat.riskLevel).label}</strong>
              </div>
              <div className="cleanup-review-stat">
                <span>已验证路径</span>
                <strong>{detailCat.paths.length} 条</strong>
              </div>
              <div className="cleanup-review-stat">
                <span>当前已选</span>
                <strong>{fmtBytes(detailSelectedSize)}</strong>
              </div>
            </div>

            <div className={detailCat.childCategories ? "cleanup-review-body" : "cleanup-review-body cleanup-review-body--single"}>
              {detailCat.childCategories && (<aside className="cleanup-review-rail">
                  <div className="cleanup-review-rail-heading">
                    <span>工具链</span>
                    <Button type="text" className="cleanup-select-all-button" size="small" disabled={busy || detailCategories.every((category) => category.paths.length === 0)} onClick={() => setCategoriesChecked(detailCategories, !detailAllSelection?.checked)}>
                      {detailAllSelection?.checked ? "清除全部勾选" : "勾选全部"}
                    </Button>
                  </div>
                  <div className="cleanup-review-rail-list">
                    {detailToolchains.map((category) => {
                    const selection = cleanupCategorySelection(category, selected, excludedPaths);
                    const active = category.id === activeDetailCategory.id;
                    return (<button type="button" key={category.id} className={`cleanup-review-tool${active ? " is-active" : ""}`} onClick={() => {
                            setActiveToolchainId(category.id);
                        }}>
                          <Checkbox checked={selection.checked} indeterminate={selection.indeterminate} disabled={busy} onClick={(event) => event.stopPropagation()} onChange={() => toggleCategory(category)}/>
                          <span className="cleanup-review-tool-icon">{categoryIcon(category.id)}</span>
                          <span className="cleanup-review-tool-copy">
                            <strong>{category.name}</strong>
                            <small>{selection.checkedPathCount}/{selection.totalPathCount} 路径</small>
                          </span>
                          <b>{fmtBytes(category.sizeBytes)}</b>
                        </button>);
                })}
                  </div>
                </aside>)}

              <section className="cleanup-review-content">
                <div className="cleanup-review-content-head">
                  <div className="cleanup-review-content-heading">
                    <div>
                      <div className="cleanup-review-content-titleline">
                        <strong>{detailCat.childCategories ? activeDetailCategory.name : "缓存路径"}</strong>
                        <span className="cleanup-risk-pill" style={{ color: riskMeta(activeDetailCategory.riskLevel).color, background: riskMeta(activeDetailCategory.riskLevel).background }}>
                          {riskMeta(activeDetailCategory.riskLevel).label}
                        </span>
                      </div>
                      <span>{detailCat.childCategories ? activeDetailCategory.description : "核对目录与匹配规则，勾选需要加入清理队列的缓存。"}</span>
                    </div>
                  </div>
                  <div className="cleanup-review-content-tools">
                    <Button className="cleanup-select-current-button" type={activeDetailSelection?.checked ? "default" : "primary"} disabled={busy || activeDetailCategory.paths.length === 0} onClick={() => toggleCategory(activeDetailCategory)}>
                      {activeDetailSelection?.checked ? "清除当前勾选" : detailCat.childCategories ? "勾选当前工具链" : "勾选全部路径"}
                    </Button>
                  </div>
                </div>

                <div className="cleanup-review-paths">
                  {visibleDetailPaths.length > 0 ? visibleDetailPaths.map((path) => {
                const readablePath = displayPath(path.path);
                const checked = selected.has(activeDetailCategory.id)
                    && !excludedPaths.has(normalizePathKey(path.path));
                return (<article className={`cleanup-review-path${checked ? " is-selected" : ""}`} key={path.path}>
                        <Checkbox checked={checked} disabled={busy} onChange={() => togglePath(activeDetailCategory, path)}/>
                        <div className="cleanup-review-path-main">
                          <button type="button" title={`在文件管理器中定位：${readablePath}`} onClick={() => void revealItemInDir(path.path).catch(() => message.error("无法定位此目录，请检查目录是否仍存在或有访问权限"))}>
                            <span className="cleanup-review-folder"><FolderOpenOutlined /></span>
                            <span>{readablePath}</span>
                          </button>
                          <small title={path.matchedRule}>命中规则：{path.matchedRule}</small>
                        </div>
                        <div className="cleanup-review-path-source">
                          <Tag variant="filled">{sourceLabel(path.source)}</Tag>
                          <span style={{ color: riskMeta(activeDetailCategory.riskLevel).color }}>● {riskMeta(activeDetailCategory.riskLevel).label}</span>
                        </div>
                        <div className="cleanup-review-path-size">
                          <strong>{fmtBytes(path.sizeBytes)}</strong>
                          <small>{compactNumber(path.fileCount)} 文件</small>
                        </div>
                      </article>);
            }) : (<Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={detailCat.childCategories ? "未发现该工具链缓存" : "未发现可清理的缓存路径"}/>)}
                </div>
              </section>
            </div>

            <footer className="cleanup-review-foot cleanup-modal-footer">
              <div>
                <CheckCircleOutlined /> 已选 <strong>{fmtBytes(detailSelectedSize)}</strong>
                <span> · {detailSelectedPaths.length} 条路径 · {compactNumber(detailSelectedFiles)} 文件</span>
              </div>
              <Button type="primary" icon={<PlusOutlined />} disabled={busy || detailSelectedPaths.length === 0} onClick={closeDetail}>
                加入清理队列
              </Button>
            </footer>
          </div>)}
      </Modal>);
}
