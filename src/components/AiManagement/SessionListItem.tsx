import { Button, Popconfirm, Tooltip } from "antd";
import { DeleteOutlined, LockOutlined } from "@ant-design/icons";
import type { AiSessionSummary } from "@/bindings";
import { isReadOnlySession, isCodexIndexSession } from "./sessionCapabilities";
import { sessionDisplayTitle, sourceMeta, sourceMonogram, formatClock, timestampDate } from "./sessionPresentation";

interface Props {
  summary: AiSessionSummary;
  active: boolean;
  mutatingKey: string | null;
  onSelect: (summary: AiSessionSummary) => Promise<void>;
  onDelete: (summary: AiSessionSummary) => Promise<void>;
}

export default function SessionListItem({ summary, active, mutatingKey, onSelect, onDelete }: Props) {
  const brand = summary.source === "codex" ? "openai" : summary.source === "claude" ? "claude" : null;
  const readonly = isReadOnlySession(summary);
  const indexOnly = isCodexIndexSession(summary);
  const deleting = mutatingKey === "session:" + summary.workspaceHash + ":" + summary.sessionId;
  const title = sessionDisplayTitle(summary.title);
  return <article className={"aim-ses-item" + (active ? " is-active" : "")}>
    <button type="button" className="aim-ses-select" aria-current={active ? "true" : undefined}
      disabled={mutatingKey !== null} onClick={() => void onSelect(summary)} title={summary.title}>
      <strong className="aim-ses-title">{title}</strong>
      <span className="aim-ses-meta">
        <span className={`aim-ses-logo${brand ? " is-brand" : ""}`} data-source={summary.source.replace(/[^a-zA-Z0-9-]/g, "-")} aria-hidden="true">{brand ? <img src={`/brands/${brand}.svg`} alt="" width={16} height={16} /> : sourceMonogram(summary.source)}</span>
        <span className="aim-ses-origin">{sourceMeta(summary.source).label}</span>
        <span>· {indexOnly ? "仅索引" : `${summary.messageCount} 条`}</span>
        <time className="aim-ses-time" dateTime={timestampDate(summary.modifiedAt)?.toISOString()}
          title={timestampDate(summary.modifiedAt)?.toLocaleString("zh-CN")}>{formatClock(summary.modifiedAt)}</time>
      </span>
    </button>
    {readonly ? <Tooltip title={summary.source === "antigravity-backup" ? "备份会话保持只读" : "该来源只提供只读索引"}>
      <span className="aim-ses-lock" aria-label="只读会话"><LockOutlined /></span>
    </Tooltip> : <Popconfirm title={indexOnly ? "清理残留索引？" : "删除这条会话？"}
      description={<div className="aim-session-delete-summary"><p>{title}</p><span>{indexOnly ? "将清理残留索引和数据库记录。" : "将删除这条本机会话记录。"}此操作不可恢复。</span></div>}
      okText={indexOnly ? "清理索引" : "删除"} cancelText="保留" okButtonProps={{ danger: true }}
      disabled={mutatingKey !== null} onConfirm={() => onDelete(summary)}>
      <Button className="aim-ses-del" type="text" size="small" loading={deleting} disabled={mutatingKey !== null}
        aria-label={indexOnly ? "清理残留索引" : "删除会话"} icon={<DeleteOutlined />} />
    </Popconfirm>}
  </article>;
}
