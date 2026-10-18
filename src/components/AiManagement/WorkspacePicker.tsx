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
      placeholder="暂无项目" showSearch={{ filterOption: (input, option) =>
        String(option?.searchText ?? "").toLocaleLowerCase().includes(input.toLocaleLowerCase()) }}
      options={workspaces.map((workspace) => ({ value: workspace.hash, label: workspace.name,
        searchText: `${workspace.name} ${workspace.path}`, workspace }))}
      optionRender={(option) => <div className="aim-project-option">
        <FolderOutlined /><div><strong>{option.data.workspace.name}</strong><small title={option.data.workspace.path}>{option.data.workspace.path}</small></div>
        <span>{option.data.workspace.sessions.length} 条</span>
      </div>}
      listHeight={280} listItemHeight={56} popupMatchSelectWidth={360}
      classNames={{ popup: { root: "aim-project-popup" } }}
      notFoundContent={<span className="aim-project-not-found">没有匹配的项目，试试名称或路径</span>}
      onChange={(hash) => { const next = workspaces.find((w) => w.hash === hash); if (next) onSelect(next); }}
    />
    <div className="aim-project-path" title={active?.path}>{active?.path ?? "扫描后会在这里显示项目"}</div>
  </div>;
}
