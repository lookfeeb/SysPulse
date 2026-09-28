import { MAX_PROJECT_ROOTS, normalizePathKey, removeProjectRoot } from "@/routes/cleanupSelection";
import { CheckCircleOutlined, CloseOutlined, DeleteOutlined, EditOutlined, FolderAddOutlined, FolderOpenOutlined, RadarChartOutlined } from "@ant-design/icons";
import { openPath } from "@tauri-apps/plugin-opener";
import { Button, Empty, Modal, Popconfirm, Space, Tag, Tooltip } from "antd";
import type { CleanupModel } from "./useCleanupModel";
export default function ProjectRootsModal({ model }: {
    model: CleanupModel;
}) {
    const { onAddProjectRoot, onReplaceProjectRoot, persistProjectRoots, projectManagerOpen, projectRoots, setProjectManagerOpen } = model;
    return (<Modal title={null} closable={false} open={projectManagerOpen} onCancel={() => setProjectManagerOpen(false)} footer={null} width={920} centered className="cleanup-review-modal cleanup-project-modal" styles={{ container: { padding: 0, overflow: "hidden" }, body: { padding: 0 } }}>
        <div className="cleanup-review-shell cleanup-project-shell">
          <header className="cleanup-review-head">
            <div className="cleanup-review-heading">
              <span className="cleanup-review-eyebrow">SCAN SCOPE · PERSISTED</span>
              <div className="cleanup-review-title-row">
                <span className="cleanup-review-title-icon"><FolderAddOutlined /></span>
                <div>
                  <strong>项目目录管理</strong>
                  <span>只发现符合规则的垃圾、缓存与编译产物，绝不会把项目根目录整体清空</span>
                </div>
              </div>
            </div>
            <div className="cleanup-review-head-actions">
              <Button type="text" className="cleanup-review-close" icon={<CloseOutlined />} onClick={() => setProjectManagerOpen(false)} aria-label="关闭"/>
            </div>
          </header>

          <div className="cleanup-review-overview cleanup-project-overview">
            <div className="cleanup-review-stat cleanup-review-stat--primary">
              <span>自定义扫描根</span>
              <strong>{projectRoots.length} / {MAX_PROJECT_ROOTS}</strong>
            </div>
            <div className="cleanup-review-stat">
              <span>剩余名额</span>
              <strong>{MAX_PROJECT_ROOTS - projectRoots.length}</strong>
            </div>
            <div className="cleanup-review-stat">
              <span>自动扫描范围</span>
              <strong>用户目录 + 固定磁盘</strong>
            </div>
            <div className="cleanup-review-stat">
              <span>配置策略</span>
              <strong>本地持久化</strong>
            </div>
          </div>

          <div className="cleanup-project-body">
            <aside className="cleanup-project-guide">
              <span className="cleanup-project-guide-title">扫描规则</span>
              <div className="cleanup-project-guide-item is-active">
                <span><RadarChartOutlined /></span>
                <div><strong>优先侦察</strong><small>自定义根先于宽泛磁盘扫描处理</small></div>
              </div>
              <div className="cleanup-project-guide-item">
                <span><CheckCircleOutlined /></span>
                <div><strong>规范化去重</strong><small>大小写、长路径前缀和父子目录统一处理</small></div>
              </div>
              <div className="cleanup-project-guide-item is-warning">
                <span>!</span>
                <div><strong>不会整目录删除</strong><small>战争蚁只接收快照中的缓存子路径</small></div>
              </div>
            </aside>

            <section className="cleanup-project-content">
              <div className="cleanup-project-content-head">
                <div>
                  <strong>自定义工作区</strong>
                  <span>侦察蚁会深入子目录匹配规则；添加的工作区自身永远不是清理目标。</span>
                </div>
                <Tag color="blue" variant="filled">{projectRoots.length} 个目录</Tag>
              </div>

              <div className="cleanup-project-roots">
                {projectRoots.length > 0 ? projectRoots.map((root, index) => (<article className="cleanup-project-root" key={normalizePathKey(root)}>
                    <span className="cleanup-project-root-index">{String(index + 1).padStart(2, "0")}</span>
                    <span className="cleanup-review-folder"><FolderOpenOutlined /></span>
                    <div className="cleanup-project-root-copy">
                      <Tooltip title={root} placement="topLeft">
                        <button type="button" onClick={() => void openPath(root).catch(() => undefined)}>{root}</button>
                      </Tooltip>
                      <span>优先扫描 · 配置已持久化</span>
                    </div>
                    <div className="cleanup-project-root-actions">
                      <Button type="text" size="small" icon={<EditOutlined />} onClick={() => void onReplaceProjectRoot(index)}>替换</Button>
                      <Popconfirm title="移除此扫描目录？" description="只移除扫描配置，不会删除磁盘上的目录。" onConfirm={() => persistProjectRoots(removeProjectRoot(projectRoots, index))}>
                        <Button type="text" danger size="small" icon={<DeleteOutlined />}>移除</Button>
                      </Popconfirm>
                    </div>
                  </article>)) : (<Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未添加项目目录，仍会自动扫描用户目录、固定磁盘候选目录和历史热点">
                    <Button type="primary" icon={<FolderAddOutlined />} onClick={() => void onAddProjectRoot()}>添加第一个目录</Button>
                  </Empty>)}
              </div>
            </section>
          </div>

          <footer className="cleanup-review-foot cleanup-project-foot">
            <div>
              已配置 <strong>{projectRoots.length}</strong>
              <span> 个优先扫描根 · 修改后立即保存</span>
            </div>
            <Space size={8}>
              <Popconfirm title="清空全部项目目录？" description="只移除扫描配置，不会删除磁盘上的任何文件。" onConfirm={() => persistProjectRoots([])} disabled={projectRoots.length === 0}>
                <Button danger disabled={projectRoots.length === 0}>清空全部</Button>
              </Popconfirm>
              <Button icon={<FolderAddOutlined />} disabled={projectRoots.length >= MAX_PROJECT_ROOTS} onClick={() => void onAddProjectRoot()}>
                添加扫描根
              </Button>
              <Button type="primary" onClick={() => setProjectManagerOpen(false)}>完成</Button>
            </Space>
          </footer>
        </div>
      </Modal>);
}
