import { CommentOutlined, DeleteOutlined, EllipsisOutlined, ReloadOutlined } from "@ant-design/icons";
import { App as AntdApp, Button, Dropdown, Tooltip } from "antd";
import SessionDetails from "./SessionDetails";
import SessionListItem from "./SessionListItem";
import "./SessionManager.css";
import { sameSession } from "./sessionPresentation";
import { useSessionManager } from "./useSessionManager";
import VirtualSessionList from "./VirtualSessionList";
import WorkspacePicker from "./WorkspacePicker";

export default function SessionManager() {
  const model = useSessionManager();
  const { modal } = AntdApp.useApp();
  const { activeWorkspace, activeWorkspaceHash, canDeleteWorkspace, deleteSession, deleteWorkspace, groupedSessions,
    hasMutation, mutatingKey, openWorkspace, refreshTree, selectSession, selectedSummary, sessionListScrollRef,
    sessionSidebarRef, treeLoading, workspaces } = model;

  const confirmDeleteWorkspace = () => {
    if (!activeWorkspace || !canDeleteWorkspace || hasMutation || treeLoading) return;
    const workspace = activeWorkspace;
    modal.confirm({
      title: `删除“${workspace.name}”的全部会话？`,
      content: <div className="aim-session-delete-summary">
        <p>将删除这个项目下的 {workspace.sessions.length} 条本机会话，包含所有来源的记录。此操作不可恢复。</p>
        <small>项目目录：{workspace.path}</small>
      </div>,
      okText: `删除 ${workspace.sessions.length} 条会话`, cancelText: "保留会话", okButtonProps: { danger: true },
      onOk: () => deleteWorkspace(workspace),
    });
  };

  return <div className="aim-sessions">
    <aside className="aim-card aim-ses-side" ref={sessionSidebarRef} aria-label="项目会话导航">
      <header className="aim-library-header">
        <div className="aim-library-identity"><span className="aim-library-icon"><CommentOutlined /></span><div><h2>会话库</h2></div></div>
        <Tooltip title="重新扫描本机会话">
          <Button type="text" icon={<ReloadOutlined />} loading={treeLoading} disabled={hasMutation}
            aria-label="重新扫描会话" onClick={() => void refreshTree()} />
        </Tooltip>
      </header>

      <WorkspacePicker workspaces={workspaces} active={activeWorkspace} disabled={hasMutation} onSelect={openWorkspace} />

      <VirtualSessionList groups={groupedSessions} scrollRef={sessionListScrollRef} loading={treeLoading}
        emptyText={!activeWorkspace ? "还没有本机会话" : "这个项目还没有会话"}
        emptyAction={<Button size="small" loading={treeLoading} disabled={hasMutation} onClick={() => void refreshTree()}>重新扫描</Button>}
        resetKey={activeWorkspaceHash ?? ""}
        renderSession={(summary) => <SessionListItem summary={summary} active={sameSession(selectedSummary, summary)}
          mutatingKey={mutatingKey} onSelect={selectSession} onDelete={deleteSession} />} />

      <footer className="aim-ses-foot">
        <span><kbd>↑</kbd><kbd>↓</kbd> 浏览 <i /> <kbd>Enter</kbd> 打开</span>
        <Dropdown trigger={["click"]} menu={{ items: [{ key: "delete", icon: <DeleteOutlined />,
          danger: true, label: "删除本项目全部会话", disabled: !canDeleteWorkspace || hasMutation || treeLoading,
          onClick: confirmDeleteWorkspace }] }}>
          <Button type="text" size="small" icon={<EllipsisOutlined />} aria-label="项目会话操作"
            disabled={!activeWorkspace || hasMutation} />
        </Dropdown>
      </footer>
    </aside>
    <SessionDetails model={model} />
  </div>;
}
