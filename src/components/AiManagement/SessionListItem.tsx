import { Button, Popconfirm, Tooltip } from "antd";
import { DeleteOutlined, LockOutlined } from "@ant-design/icons";
import type { AiSessionSummary } from "@/bindings";
import { isReadOnlySession, isCodexIndexSession } from "./sessionCapabilities";
import { cleanTitle, sourceMeta, sourceMonogram, formatClock } from "./sessionPresentation";

interface Props {
  summary: AiSessionSummary;
  active: boolean;
  mutatingKey: string | null;
  onSelect: (summary: AiSessionSummary) => Promise<void>;
  onDelete: (summary: AiSessionSummary) => Promise<void>;
}

export default function SessionListItem({ summary, active, mutatingKey, onSelect, onDelete }: Props) {
  const readonly = isReadOnlySession(summary);
  const indexOnly = isCodexIndexSession(summary);
  const deletingSession = mutatingKey === "session:" + summary.workspaceHash + ":" + summary.sessionId;
  const hasMutation = mutatingKey !== null;
  const sourceClass = summary.source.replace(/[^a-zA-Z0-9-]/g, "-");
  return (
                    <article
                      key={summary.workspaceHash + ":" + summary.sessionId}
                      className={"aim-ses-item" + (active ? " is-active" : "")}
                    >
                      <button
                        type="button"
                        className="aim-ses-select"
                        aria-current={active ? "true" : undefined}
                        onClick={() => void onSelect(summary)}
                      >
                        <span className="aim-ses-logo" data-source={sourceClass}>
                          {sourceMonogram(summary.source)}
                        </span>
                        <span className="aim-ses-tx">
                          <strong>{cleanTitle(summary.title)}</strong>
                          <span>
                            {sourceMeta(summary.source).label}
                            <i aria-hidden="true" />
                            {summary.messageCount} 条消息
                            {indexOnly && <em>仅索引</em>}
                          </span>
                        </span>
                        <span className="aim-ses-time">
                          {formatClock(summary.modifiedAt)}
                        </span>
                      </button>

                      {readonly ? (
                        <Tooltip title={summary.source === "antigravity-backup"
                          ? "备份会话保持只读，避免破坏恢复数据"
                          : "该来源目前只提供只读索引"}
                        >
                          <span className="aim-ses-lock"><LockOutlined /></span>
                        </Tooltip>
                      ) : (
                        <Popconfirm
                          title={indexOnly ? "清理残留索引" : "删除会话"}
                          description={indexOnly
                            ? "正文文件已经不存在。将清理 Codex 中残留的索引和数据库记录，此操作不可恢复。"
                            : "确定删除这条会话？此操作不可恢复。"}
                          okText={indexOnly ? "清理索引" : "删除"}
                          cancelText="取消"
                          okButtonProps={{ danger: true }}
                          onConfirm={() => onDelete(summary)}
                        >
                          <Button
                            className="aim-ses-del"
                            type="text"
                            size="small"
                            danger
                            loading={deletingSession}
                            disabled={hasMutation && !deletingSession}
                            aria-label={indexOnly ? "清理残留索引" : "删除会话"}
                            icon={<DeleteOutlined />}
                          />
                        </Popconfirm>
                      )}
                    </article>
  );
}
