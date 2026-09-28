import { CheckOutlined, CopyOutlined, DatabaseOutlined, DeleteOutlined, DownloadOutlined, FileMarkdownOutlined, FileSearchOutlined, FolderOpenOutlined, LockOutlined, MessageOutlined, RobotOutlined, UserOutlined } from "@ant-design/icons";
import { Button, Dropdown, Empty, Pagination, Popconfirm, Spin, Tooltip } from "antd";
import MarkdownContent from "./MarkdownContent";
import "./SessionManager.css";
import { cleanTitle, formatFileSize, formatTime, roleLabel, sourceMeta } from "./sessionPresentation";
import type { SessionModel } from "./useSessionManager";
export default function SessionDetails({ model }: {
    model: SessionModel;
}) {
    const { canRevealSelected, changeMessagePage, conversationPanelRef, conversationScrollRef, copiedMessageKey, copyMessage, deleteSession, exportSession, hasMutation, indexOnlySelected, messagePage, mutatingKey, pagedMessages, readonlySelected, revealSession, selectedSummary, session, sessionLoading, totalMessageCount, totalPages } = model;
    return (<section className="aim-card aim-conv" ref={conversationPanelRef} aria-labelledby="aim-conv-title">
        <header className="aim-conv-hd">
          <div className="aim-conv-l1">
            <h2 id="aim-conv-title">
              {selectedSummary ? cleanTitle(selectedSummary.title) : "选择一条会话"}
            </h2>
            {selectedSummary && (<>
                <Tooltip title={!canRevealSelected
                ? (indexOnlySelected
                    ? "正文文件已不存在，只能查看恢复内容或清理残留索引"
                    : "该索引会话没有独立正文文件")
                : "在资源管理器中定位会话文件"}>
                  <span>
                    <Button className="aim-act aim-act--sm" aria-label="定位文件" icon={<FolderOpenOutlined />} disabled={!canRevealSelected || (hasMutation && mutatingKey !== "reveal")} loading={mutatingKey === "reveal"} onClick={() => void revealSession()}>
                      定位文件
                    </Button>
                  </span>
                </Tooltip>
                <Dropdown trigger={["click"]} classNames={{ root: "aim-menu" }} menu={{
                items: [
                    {
                        key: "markdown",
                        icon: <FileMarkdownOutlined />,
                        label: "导出 Markdown",
                        disabled: hasMutation,
                        onClick: () => void exportSession("markdown"),
                    },
                    {
                        key: "json",
                        icon: <DownloadOutlined />,
                        label: "导出 JSON",
                        disabled: hasMutation,
                        onClick: () => void exportSession("json"),
                    },
                ],
            }}>
                  <Button className="aim-act aim-act--sm" aria-label="导出会话" icon={<DownloadOutlined />} loading={mutatingKey === "export"} disabled={hasMutation && mutatingKey !== "export"}>
                    导出
                  </Button>
                </Dropdown>
                {!readonlySelected && (<Popconfirm title={indexOnlySelected ? "清理残留索引" : "删除会话"} description={indexOnlySelected
                    ? "正文文件已经不存在。将清理 Codex 中残留的索引和数据库记录，此操作不可恢复。"
                    : "确定删除当前会话？此操作不可恢复。"} okText={indexOnlySelected ? "清理索引" : "删除"} cancelText="取消" okButtonProps={{ danger: true }} disabled={hasMutation} onConfirm={() => deleteSession(selectedSummary)}>
                    <Button className="aim-act aim-act--sm aim-act--icon aim-act--danger" danger disabled={hasMutation} aria-label={indexOnlySelected ? "清理残留索引" : "删除当前会话"} icon={<DeleteOutlined />}/>
                  </Popconfirm>)}
              </>)}
          </div>

          {selectedSummary && (<div className="aim-conv-l2">
              <span className="aim-chip aim-chip--teal">
                {sourceMeta(selectedSummary.source).label}
              </span>
              {readonlySelected && (<span className="aim-chip aim-chip--mute"><LockOutlined />只读</span>)}
              {indexOnlySelected && (<span className="aim-chip aim-chip--warn"><DatabaseOutlined />仅索引</span>)}
              <span className="aim-conv-meta">
                <MessageOutlined /><b>{totalMessageCount}</b> 条消息
              </span>
              <i className="aim-conv-dot" aria-hidden="true"/>
              <span className="aim-conv-meta">
                <DatabaseOutlined /><b>{formatFileSize(selectedSummary.fileSize)}</b>
              </span>
              <i className="aim-conv-dot" aria-hidden="true"/>
              <span className="aim-conv-meta">更新于 {formatTime(selectedSummary.modifiedAt)}</span>
            </div>)}
        </header>

        <div className="aim-conv-scroll" ref={conversationScrollRef}>
          {!selectedSummary ? (<div className="aim-empty">
              <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="从左侧选择一条会话"/>
            </div>) : sessionLoading ? (<div className="aim-empty">
              <Spin size="large" description="正在解析会话内容…"/>
            </div>) : !session ? (<div className="aim-empty">
              <Empty description="会话内容加载失败，请刷新后重试"/>
            </div>) : (<div className="aim-msg-stream">
              {session.conversationSummary?.trim() && (<article className="aim-msg is-summary">
                  <div className="aim-msg-av"><FileSearchOutlined /></div>
                  <div className="aim-msg-bd">
                    <div className="aim-msg-hd">
                      <strong>上下文摘要</strong>
                      <span>会话持续记忆</span>
                    </div>
                    <div className="aim-bubble">
                      <MarkdownContent text={session.conversationSummary}/>
                    </div>
                  </div>
                </article>)}

              {pagedMessages.length === 0 ? (<div className="aim-empty">
                  <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无消息"/>
                </div>) : pagedMessages.map(({ item, index }) => {
                const role = item.message.role || "system";
                const key = (item.message.id || "message") + ":" + index;
                const copied = copiedMessageKey === key;
                return (<article key={key} className={"aim-msg is-" + role}>
                    <div className="aim-msg-av">
                      {role === "user"
                        ? <UserOutlined />
                        : role === "assistant"
                            ? <RobotOutlined />
                            : <FileSearchOutlined />}
                    </div>
                    <div className="aim-msg-bd">
                      <div className="aim-msg-hd">
                        <strong>{roleLabel(role)}</strong>
                        <span>消息 {String(index + 1).padStart(2, "0")}</span>
                        {item.message.isHidden && (<span className="aim-chip aim-chip--mute">隐藏消息</span>)}
                        <Tooltip title={copied ? "已复制" : "复制此条消息"}>
                          <Button className={"aim-msg-copy" + (copied ? " is-copied" : "")} type="text" size="small" icon={copied ? <CheckOutlined /> : <CopyOutlined />} aria-label="复制此条消息" onClick={() => void copyMessage(item, key)}/>
                        </Tooltip>
                      </div>
                      <div className="aim-bubble">
                        {item.message.content.length === 0 ? (<span className="aim-msg-void">空消息</span>) : item.message.content.map((content, contentIndex) => (<div key={key + ":" + contentIndex}>
                            {content.type && content.type !== "text" && (<span className="aim-chip aim-chip--violet aim-msg-type">
                                {content.type}
                              </span>)}
                            <MarkdownContent text={content.text}/>
                          </div>))}
                      </div>
                    </div>
                  </article>);
            })}
            </div>)}
        </div>

        {session && session.totalMessages > session.pageSize && (<footer className="aim-conv-page">
            <span>第 {messagePage} / {totalPages} 页 · 共 {session.totalMessages} 条消息</span>
            <Pagination size="small" current={messagePage} pageSize={session.pageSize} total={session.totalMessages} showSizeChanger={false} disabled={sessionLoading || hasMutation} onChange={(page) => void changeMessagePage(page)}/>
          </footer>)}
      </section>);
}
