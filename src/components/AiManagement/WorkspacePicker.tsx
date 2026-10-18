import { FolderOutlined } from "@ant-design/icons";
import { Select } from "antd";
import type { WorkspaceView } from "./sessionPresentation";

interface Props {
  workspaces: WorkspaceView[];
  active: WorkspaceView | null;
  disabled: boolean;
  onSelect: (workspace: WorkspaceView) => void;
}

export default function WorkspacePicker({ workspaces, active, disabled, onSelect }: Props) {
  return <div className="aim-project-picker">
    <label id="aim-workspace-title" htmlFor="aim-workspace-select">项目工作区 <span>{workspaces.length} 个项目</span></label>
    <Select id="aim-workspace-select" aria-labelledby="aim-workspace-title" className="aim-project-select"
      prefix={<FolderOutlined />} value={active?.hash} disabled={disabled || workspaces.length === 0}
      placeholder="暂无项目" showSearch={false}
      options={workspaces.map((workspace) => ({ value: workspace.hash, label: workspace.name,
        workspace }))}
      optionRender={(option) => <div className="aim-project-option">
        <FolderOutlined /><div><strong>{option.data.workspace.name}</strong><small title={option.data.workspace.path}>{option.data.workspace.path}</small></div>
        <span>{option.data.workspace.sessions.length} 条</span>
      </div>}
      listHeight={280} listItemHeight={56} popupMatchSelectWidth={360}
      classNames={{ popup: { root: "aim-project-popup" } }}
      notFoundContent={<span className="aim-project-not-found">暂无可选项目</span>}
      onChange={(hash) => { const next = workspaces.find((w) => w.hash === hash); if (next) onSelect(next); }}
    />
  </div>;
}
