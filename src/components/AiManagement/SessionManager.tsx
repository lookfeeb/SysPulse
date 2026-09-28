import { DeleteOutlined, DownOutlined, FolderOutlined, ReloadOutlined, SearchOutlined } from "@ant-design/icons";
import { Button, Dropdown, Input, Popconfirm, Select, Tooltip } from "antd";
import SessionDetails from "./SessionDetails";
import SessionListItem from "./SessionListItem";
import "./SessionManager.css";
import { sameSession } from "./sessionPresentation";
import { useSessionManager } from "./useSessionManager";
import VirtualSessionList from "./VirtualSessionList";
export default function SessionManager() {
    const model = useSessionManager();
    const { activeWorkspace, activeWorkspaceHash, canDeleteWorkspace, deleteSession, deleteWorkspace, groupedSessions, hasMutation, mutatingKey, openWorkspace, refreshTree, selectSession, selectedSummary, sessionListScrollRef, sessionQuery, sessionSidebarRef, setSessionQuery, setSourceFilter, sourceFilter, sourceOptions, totalSessions, treeLoading, workspaceForDelete, workspaces } = model;
    return (<div className="aim-sessions">
      <aside className="aim-card aim-ses-side" ref={sessionSidebarRef} aria-labelledby="aim-workspace-title">
        <Dropdown trigger={["click"]} classNames={{ root: "aim-menu aim-ws-menu" }} menu={{
            selectedKeys: activeWorkspace ? [activeWorkspace.hash] : [],
            items: workspaces.map((workspace) => ({
                key: workspace.hash,
                label: (<div className="aim-ws-option">
                  <FolderOutlined />
                  <div>
                    <strong>{workspace.name}</strong>
                    <small title={workspace.path}>{workspace.path}</small>
                  </div>
                  <b>{workspace.sessions.length}</b>
                </div>),
            })),
            onClick: ({ key }) => {
                const workspace = workspaces.find((item) => item.hash === key);
                if (workspace)
                    openWorkspace(workspace);
            },
        }}>
          <button type="button" className="aim-ws" disabled={workspaces.length === 0 || hasMutation} aria-label="切换项目工作区">
            <span className="aim-ws-icon"><FolderOutlined /></span>
            <span className="aim-ws-copy">
              <small>当前项目</small>
              <strong id="aim-workspace-title">{activeWorkspace?.name ?? "暂无项目"}</strong>
            </span>
            <span className="aim-ws-count" title={"本机共 " + workspaces.length + " 个项目、" + totalSessions + " 条会话"}>
              {workspaces.length}
            </span>
            <DownOutlined />
          </button>
        </Dropdown>

        <div className="aim-ses-tools">
          <Input allowClear className="aim-input aim-input--sm" value={sessionQuery} prefix={<SearchOutlined />} placeholder="搜索会话" onChange={(event) => setSessionQuery(event.target.value)}/>
          <Select className="aim-ses-source" classNames={{ popup: { root: "aim-select-popup" } }} size="small" value={sourceFilter} options={sourceOptions} aria-label="按会话来源筛选" onChange={setSourceFilter}/>
        </div>

        <VirtualSessionList groups={groupedSessions} scrollRef={sessionListScrollRef} loading={treeLoading} emptyText={activeWorkspace?.sessions.length ? "没有匹配的会话" : "当前项目没有会话"} resetKey={activeWorkspaceHash + ":" + sourceFilter + ":" + sessionQuery} renderSession={(summary) => (<SessionListItem summary={summary} active={sameSession(selectedSummary, summary)} mutatingKey={mutatingKey} onSelect={selectSession} onDelete={deleteSession}/>)}/>

        <footer className="aim-ses-foot">
          {workspaceForDelete && (<Tooltip title={canDeleteWorkspace
                ? "删除当前项目的全部会话"
                : "项目包含只读索引或备份会话，不能整组删除"}>
              <span>
                <Popconfirm title="删除项目会话" description={"确定删除 “" + workspaceForDelete.name + "” 下的全部会话？此操作不可恢复。"} okText="全部删除" cancelText="取消" okButtonProps={{ danger: true }} disabled={!canDeleteWorkspace || hasMutation} onConfirm={() => deleteWorkspace(workspaceForDelete)}>
                  <Button className="aim-act aim-act--sm aim-act--ghost aim-act--danger" danger disabled={!canDeleteWorkspace || (hasMutation
                && mutatingKey !== "workspace:" + workspaceForDelete.hash)} loading={mutatingKey === "workspace:" + workspaceForDelete.hash} icon={<DeleteOutlined />}>
                    删除项目
                  </Button>
                </Popconfirm>
              </span>
            </Tooltip>)}
          <Button className="aim-act aim-act--sm aim-act--ghost" icon={<ReloadOutlined />} loading={treeLoading} disabled={hasMutation} onClick={() => void refreshTree()}>
            重新扫描
          </Button>
        </footer>
      </aside>

      <SessionDetails model={model}/>
    </div>);
}
