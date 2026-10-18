import { MAX_PROJECT_ROOTS, normalizePathKey, removeProjectRoot } from "@/routes/cleanupSelection";
import { CheckCircleOutlined, CloseOutlined, DeleteOutlined, EditOutlined, FolderAddOutlined, FolderOpenOutlined, SafetyCertificateOutlined } from "@ant-design/icons";
import { openPath } from "@tauri-apps/plugin-opener";
import { App, Button, Modal, Popconfirm, Tooltip } from "antd";
import type { CleanupModel } from "./useCleanupModel";
import "./ProjectRootsModal.css";

export default function ProjectRootsModal({ model }: { model: CleanupModel }) {
  const { message } = App.useApp();
  const { busy, onAddProjectRoot, onReplaceProjectRoot, persistProjectRoots, projectManagerOpen, projectRoots, setProjectManagerOpen } = model;
  return <Modal title={null} closable={false} open={projectManagerOpen} onCancel={() => setProjectManagerOpen(false)}
    footer={null} width={760} centered className="cleanup-project-modal"
    styles={{ container: { padding: 0, overflow: "hidden" }, body: { padding: 0 } }}>
    <div className="project-scope">
      <header className="project-scope-header">
        <span className="project-scope-icon"><FolderOpenOutlined /></span>
        <div><h2>项目目录管理</h2><p>添加常用工作区，优先检查缓存与编译产物</p></div>
        <Button type="text" icon={<CloseOutlined />} onClick={() => setProjectManagerOpen(false)} aria-label="关闭项目目录管理" />
      </header>
      <div className="project-scope-auto">
        <span><SafetyCertificateOutlined /> 自动扫描</span>
        <strong>用户目录</strong><strong>固定磁盘</strong>
        <span className="project-scope-auto-note">无需手动添加</span>
      </div>
      <section className="project-scope-content" aria-label="优先扫描目录">
        <div className="project-scope-toolbar">
          <div><h3>优先扫描目录 <span>{projectRoots.length} / {MAX_PROJECT_ROOTS}</span></h3><p>只清理匹配规则的缓存，保留项目源文件。</p></div>
          <Button type="primary" icon={<FolderAddOutlined />} disabled={busy || projectRoots.length >= MAX_PROJECT_ROOTS} onClick={() => void onAddProjectRoot()}>添加目录</Button>
        </div>
        <div className="project-scope-list">
          {projectRoots.length ? projectRoots.map((root, index) => {
            const name = root.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || root;
            return <article className="project-scope-root" key={normalizePathKey(root)}>
              <span className="project-scope-root-icon"><FolderOpenOutlined /></span>
              <div className="project-scope-root-copy">
                <strong>{name}</strong>
                <Tooltip title={root} placement="topLeft"><button type="button" title={`打开 ${root}`} onClick={() => void openPath(root).catch(() => message.error("无法打开此目录，请检查路径是否存在"))}>{root}</button></Tooltip>
              </div>
              <div className="project-scope-root-actions">
                <Tooltip title="替换目录"><Button type="text" size="small" icon={<EditOutlined />} aria-label={`替换目录 ${root}`} disabled={busy} onClick={() => void onReplaceProjectRoot(index)} /></Tooltip>
                <Popconfirm title="移除此扫描目录？" description="仅移除扫描配置，磁盘上的文件会保留。" disabled={busy} onConfirm={() => persistProjectRoots(removeProjectRoot(projectRoots, index))}>
                  <Button type="text" danger size="small" icon={<DeleteOutlined />} aria-label={`移除目录 ${root}`} disabled={busy} />
                </Popconfirm>
              </div>
            </article>;
          }) : <div className="project-scope-empty">
            <span><FolderAddOutlined /></span><strong>添加你的第一个工作区</strong>
            <p>选择存放项目的文件夹，扫描时会优先检查。<br />暂不添加也可使用自动扫描。</p>
          </div>}
        </div>
      </section>
      <footer className="project-scope-footer">
        <span><CheckCircleOutlined /> 更改自动保存</span>
        <Popconfirm title="清空全部项目目录？" description="仅移除扫描配置，磁盘上的文件会保留。" onConfirm={() => persistProjectRoots([])} disabled={busy || projectRoots.length === 0}>
          <Button type="text" danger disabled={busy || projectRoots.length === 0}>清空列表</Button>
        </Popconfirm>
        <Button onClick={() => setProjectManagerOpen(false)}>完成</Button>
      </footer>
    </div>
  </Modal>;
}
