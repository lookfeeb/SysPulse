import { DeleteOutlined, EllipsisOutlined, ReloadOutlined, SearchOutlined } from "@ant-design/icons";
import { App as AntdApp, Button, Dropdown, Input, Select, Tooltip } from "antd";
import { useRef } from "react";
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
  const searchRef = useRef<import("antd").InputRef>(null);
  const { activeWorkspace, activeWorkspaceHash, canDeleteWorkspace, deleteSession, deleteWorkspace, groupedSessions,
    hasMutation, mutatingKey, openWorkspace, refreshTree, selectSession, selectedSummary, sessionListScrollRef,
    sessionQuery, sessionSidebarRef, setSessionQuery, setSourceFilter, sourceFilter, sourceOptions,
    totalSessions, treeLoading, workspaces, visibleSessions, clearFilters } = model;
  const filtering = !!sessionQuery.trim() || sourceFilter !== "all";

  const confirmDeleteWorkspace = () => {
    if (!activeWorkspace || !canDeleteWorkspace || hasMutation || treeLoading) return;
    const workspace = activeWorkspace;
    modal.confirm({
      title: `删除“${workspace.name}”的全部会话？`,
      content: <div className="aim-session-delete-summary">
        <p>将删除这个项目下的 {workspace.sessions.length} 条本机会话，包含所有来源和当前筛选之外的记录。此操作不可恢复。</p>
        <small>项目目录：{workspace.path}</small>
      </div>,
      okText: `删除 ${workspace.sessions.length} 条会话`, cancelText: "保留会话", okButtonProps: { danger: true },
      onOk: () => deleteWorkspace(workspace),
    });
  };

  return <div className="aim-sessions">
    <aside className="aim-card aim-ses-side" ref={sessionSidebarRef} aria-label="项目会话导航"
      onKeyDown={(event) => {
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
          event.preventDefault(); searchRef.current?.focus();
        }
      }}>
      <header className="aim-library-header">
        <div><h2>会话库</h2><span>{totalSessions.toLocaleString()} 条本机会话</span></div>
        <Tooltip title="重新扫描本机会话">
          <Button type="text" icon={<ReloadOutlined />} loading={treeLoading} disabled={hasMutation}
            aria-label="重新扫描会话" onClick={() => void refreshTree()} />
        </Tooltip>
      </header>

      <WorkspacePicker workspaces={workspaces} active={activeWorkspace} disabled={hasMutation} onSelect={openWorkspace} />

      <div className="aim-ses-tools">
        <Input ref={searchRef} allowClear className="aim-input" value={sessionQuery} prefix={<SearchOutlined />}
          aria-label="搜索当前项目会话" placeholder="搜索标题、会话 ID…" disabled={hasMutation || !activeWorkspace}
          onChange={(event) => setSessionQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Escape") { setSessionQuery(""); event.stopPropagation(); }
            if (event.key === "ArrowDown") {
              const first = sessionListScrollRef.current?.querySelector<HTMLButtonElement>(".aim-ses-select");
              if (first) { event.preventDefault(); first.focus(); }
            }
          }} />
        <div className="aim-ses-filter-row">
          <Select className="aim-ses-source" classNames={{ popup: { root: "aim-select-popup" } }} size="small"
            value={sourceFilter} options={sourceOptions} aria-label="按会话来源筛选" onChange={setSourceFilter}
            disabled={hasMutation || !activeWorkspace} />
          {filtering && <Button type="text" size="small" onClick={clearFilters} disabled={hasMutation}>清除筛选</Button>}
        </div>
      </div>

      <div className="aim-ses-result-bar">
        <span aria-live="polite">{filtering ? `${visibleSessions.length} 条匹配` : `${visibleSessions.length} 条会话`}</span>
        <span>最近更新</span>
      </div>
      <VirtualSessionList groups={groupedSessions} scrollRef={sessionListScrollRef} loading={treeLoading}
        emptyText={!activeWorkspace ? "还没有本机会话" : filtering ? "没有匹配的会话" : "这个项目还没有会话"}
        emptyAction={filtering ? <Button size="small" onClick={clearFilters}>清除筛选</Button> :
          <Button size="small" loading={treeLoading} disabled={hasMutation} onClick={() => void refreshTree()}>重新扫描</Button>}
        resetKey={`${activeWorkspaceHash}:${sourceFilter}:${sessionQuery}`}
        renderSession={(summary) => <SessionListItem summary={summary} active={sameSession(selectedSummary, summary)}
          mutatingKey={mutatingKey} onSelect={selectSession} onDelete={deleteSession} />} />

      <footer className="aim-ses-foot">
        <span>↑ ↓ 浏览 · Enter 打开</span>
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
