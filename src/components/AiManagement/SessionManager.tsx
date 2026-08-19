import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  App as AntdApp,
  Button,
  Dropdown,
  Empty,
  Input,
  Pagination,
  Popconfirm,
  Select,
  Spin,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import {
  CheckOutlined,
  CopyOutlined,
  DatabaseOutlined,
  DeleteOutlined,
  DownOutlined,
  DownloadOutlined,
  FileMarkdownOutlined,
  FileSearchOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  LockOutlined,
  MessageOutlined,
  ReloadOutlined,
  RobotOutlined,
  SearchOutlined,
  UserOutlined,
} from "@ant-design/icons";
import { save } from "@tauri-apps/plugin-dialog";
import { commands } from "@/bindings";
import type {
  AiHistoryItem,
  AiSessionPage,
  AiSessionSummary,
  AiSessionTree,
} from "@/bindings";
import MarkdownContent from "./MarkdownContent";
import {
  canRevealSession,
  isCodexIndexSession,
  isReadOnlySession,
} from "./sessionCapabilities";
import { bindPanelWheelRouting } from "./panelWheel";
import "./SessionManager.css";

const EMPTY_TREE: AiSessionTree = {
  workspaces: [],
  sessionsByWorkspace: {},
};

const PAGE_SIZE = 25;

const SOURCE_META: Record<string, { label: string; color: string }> = {
  ide: { label: "Kiro IDE", color: "blue" },
  cli: { label: "Kiro CLI", color: "purple" },
  codex: { label: "Codex", color: "green" },
  claude: { label: "Claude", color: "orange" },
  antigravity: { label: "Gemini", color: "cyan" },
  "antigravity-backup": { label: "AG 备份", color: "geekblue" },
  "antigravity-ide": { label: "AG IDE", color: "processing" },
  gemini: { label: "Gemini CLI", color: "magenta" },
};

function errorMessage(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return error instanceof Error ? error.message : String(error);
}

function sourceMeta(source: string): { label: string; color: string } {
  return SOURCE_META[source] ?? { label: source || "未知来源", color: "default" };
}

function cleanTitle(value?: string): string {
  const clean = (value ?? "")
    .replace(/<\/?[a-zA-Z][^>]*>?/g, " ")
    .replace(/&[a-zA-Z#0-9]+;/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!clean) return "未命名会话";
  return clean.length > 100 ? clean.slice(0, 100) + "…" : clean;
}

function timestampDate(timestamp: number | null): Date | null {
  if (!timestamp) return null;
  const milliseconds = timestamp > 10_000_000_000 ? timestamp : timestamp * 1000;
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? null : date;
}

function formatTime(timestamp: number | null): string {
  const date = timestampDate(timestamp);
  if (!date) return "时间未知";
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

function formatClock(timestamp: number | null): string {
  const date = timestampDate(timestamp);
  if (!date) return "--:--";
  return date.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

function decodeWorkspacePath(hash: string): string {
  const knownPrefix = Object.keys(SOURCE_META).find((source) => hash.startsWith(source + ":"));
  if (knownPrefix) return hash.slice(knownPrefix.length + 1);
  try {
    return atob(hash.replace(/_+$/, ""));
  } catch {
    return hash;
  }
}

function workspacePath(hash: string, sessions: AiSessionSummary[]): string {
  return sessions.find((session) => session.workspaceDirectory)?.workspaceDirectory
    || decodeWorkspacePath(hash);
}

function workspaceName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || path || "未知工作区";
}

function historyText(item: AiHistoryItem): string {
  return item.message.content.map((content) => content.text).filter(Boolean).join("\n\n");
}

function safeFileName(value: string): string {
  const clean = cleanTitle(value).replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").trim();
  return (clean || "ai-session").slice(0, 80);
}

function sameSession(left: AiSessionSummary | null, right: AiSessionSummary): boolean {
  return !!left
    && left.workspaceHash === right.workspaceHash
    && left.sessionId === right.sessionId;
}

function roleLabel(role: string): string {
  if (role === "user") return "用户";
  if (role === "assistant") return "助手";
  if (role === "system") return "系统";
  return role || "系统";
}

function sourceMonogram(source: string): string {
  const label = sourceMeta(source).label;
  const words = label.match(/[A-Za-z]+/g) ?? [];
  if (words.length >= 2) {
    return ((words[0]?.[0] ?? "") + (words[1]?.[0] ?? "")).toUpperCase();
  }
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return label.slice(0, 2).toUpperCase();
}

function dateGroup(timestamp: number | null): "today" | "yesterday" | "earlier" {
  const date = timestampDate(timestamp);
  if (!date) return "earlier";
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const value = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  if (value === today) return "today";
  if (value === today - 86_400_000) return "yesterday";
  return "earlier";
}

const SESSION_GROUP_META = {
  today: "今天",
  yesterday: "昨天",
  earlier: "更早",
} as const;

interface WorkspaceView {
  hash: string;
  path: string;
  name: string;
  sessions: AiSessionSummary[];
}

interface SessionGroupView {
  key: keyof typeof SESSION_GROUP_META;
  label: string;
  sessions: AiSessionSummary[];
}

export default function SessionManager() {
  const { message } = AntdApp.useApp();
  const [tree, setTree] = useState<AiSessionTree>(EMPTY_TREE);
  const [treeLoading, setTreeLoading] = useState(false);
  const [selectedSummary, setSelectedSummary] = useState<AiSessionSummary | null>(null);
  const [session, setSession] = useState<AiSessionPage | null>(null);
  const [sessionLoading, setSessionLoading] = useState(false);
  const [mutatingKey, setMutatingKey] = useState<string | null>(null);
  const [activeWorkspaceHash, setActiveWorkspaceHash] = useState<string | null>(null);
  const [messagePage, setMessagePage] = useState(1);
  const [sessionQuery, setSessionQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState("all");
  const [copiedMessageKey, setCopiedMessageKey] = useState<string | null>(null);
  const selectedRef = useRef<AiSessionSummary | null>(null);
  const loadSequence = useRef(0);
  const treeLoadSequence = useRef(0);
  const copyTimer = useRef<number | null>(null);
  const sessionListScrollRef = useRef<HTMLDivElement | null>(null);
  const conversationScrollRef = useRef<HTMLDivElement | null>(null);
  const sessionSidebarHeaderRef = useRef<HTMLElement | null>(null);
  const conversationHeaderRef = useRef<HTMLElement | null>(null);
  const sessionSidebarRef = useRef<HTMLElement | null>(null);
  const conversationPanelRef = useRef<HTMLElement | null>(null);

  useEffect(() => () => {
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
  }, []);

  useEffect(() => {
    const cleanups: Array<() => void> = [];
    if (sessionSidebarRef.current && sessionListScrollRef.current) {
      cleanups.push(bindPanelWheelRouting(
        sessionSidebarRef.current,
        sessionListScrollRef.current,
      ));
    }
    if (sessionSidebarHeaderRef.current && sessionListScrollRef.current) {
      cleanups.push(bindPanelWheelRouting(
        sessionSidebarHeaderRef.current,
        sessionListScrollRef.current,
      ));
    }
    if (conversationPanelRef.current && conversationScrollRef.current) {
      cleanups.push(bindPanelWheelRouting(
        conversationPanelRef.current,
        conversationScrollRef.current,
      ));
    }
    if (conversationHeaderRef.current && conversationScrollRef.current) {
      cleanups.push(bindPanelWheelRouting(
        conversationHeaderRef.current,
        conversationScrollRef.current,
      ));
    }
    return () => cleanups.forEach((cleanup) => cleanup());
  }, []);

  const loadSessionPage = useCallback(async (
    summary: AiSessionSummary,
    page: number,
    clearCurrent: boolean,
  ): Promise<boolean> => {
    const sequence = ++loadSequence.current;
    selectedRef.current = summary;
    setActiveWorkspaceHash(summary.workspaceHash);
    setSelectedSummary(summary);
    if (clearCurrent) setSession(null);
    setSessionLoading(true);
    setMessagePage(page);
    try {
      const loaded = await commands.aiLoadSession({
        workspaceHash: summary.workspaceHash,
        sessionId: summary.sessionId,
        page,
        pageSize: PAGE_SIZE,
      });
      if (sequence !== loadSequence.current) return false;
      setSession(loaded);
      setMessagePage(loaded.page);
      return true;
    } catch (error: unknown) {
      if (sequence !== loadSequence.current) return false;
      void message.error(errorMessage(error));
      return false;
    } finally {
      if (sequence === loadSequence.current) setSessionLoading(false);
    }
  }, [message]);

  const selectSession = useCallback(async (summary: AiSessionSummary) => {
    await loadSessionPage(summary, 1, true);
  }, [loadSessionPage]);

  const loadTree = useCallback(async (invalidateCache: boolean): Promise<boolean> => {
    const sequence = ++treeLoadSequence.current;
    setTreeLoading(true);
    try {
      if (invalidateCache) await commands.aiRefreshSessionCache();
      const nextTree = await commands.aiListSessionTree();
      if (sequence !== treeLoadSequence.current) return false;
      setTree(nextTree);
      setActiveWorkspaceHash((current) =>
        current && nextTree.workspaces.includes(current)
          ? current
          : nextTree.workspaces[0] ?? null
      );

      const summaries = nextTree.workspaces.flatMap(
        (hash) => nextTree.sessionsByWorkspace[hash] ?? [],
      );
      const current = selectedRef.current;
      const retained = current
        ? summaries.find((item) => sameSession(current, item))
        : undefined;
      const target = retained ?? summaries[0];
      if (target) {
        // 树列表加载完成后立即解除列表 loading；详情解析使用独立状态，
        // 避免大会话阻塞工作区和会话列表交互。
        void selectSession(target);
      } else {
        ++loadSequence.current;
        selectedRef.current = null;
        setSelectedSummary(null);
        setSession(null);
        setSessionLoading(false);
      }
      return true;
    } catch (error: unknown) {
      if (sequence === treeLoadSequence.current) {
        void message.error(errorMessage(error));
      }
      return false;
    } finally {
      if (sequence === treeLoadSequence.current) setTreeLoading(false);
    }
  }, [message, selectSession]);

  useEffect(() => {
    void loadTree(false);
  }, [loadTree]);

  const workspaces = useMemo<WorkspaceView[]>(() => tree.workspaces.map((hash) => {
    const sessions = [...(tree.sessionsByWorkspace[hash] ?? [])].sort(
      (left, right) => (right.modifiedAt ?? 0) - (left.modifiedAt ?? 0),
    );
    const path = workspacePath(hash, sessions);
    return { hash, path, name: workspaceName(path), sessions };
  }), [tree]);

  const totalSessions = useMemo(
    () => workspaces.reduce((total, workspace) => total + workspace.sessions.length, 0),
    [workspaces],
  );

  const activeWorkspace = useMemo(
    () => workspaces.find((workspace) => workspace.hash === activeWorkspaceHash)
      ?? workspaces[0]
      ?? null,
    [activeWorkspaceHash, workspaces],
  );

  const sourceOptions = useMemo(() => {
    const sources = new Set((activeWorkspace?.sessions ?? []).map((summary) => summary.source));
    return [
      { value: "all", label: "全部来源" },
      ...[...sources]
        .sort((left, right) => sourceMeta(left).label.localeCompare(sourceMeta(right).label, "zh-CN"))
        .map((source) => ({ value: source, label: sourceMeta(source).label })),
    ];
  }, [activeWorkspace]);

  const visibleSessions = useMemo(() => {
    const query = sessionQuery.trim().toLocaleLowerCase("zh-CN");
    return (activeWorkspace?.sessions ?? []).filter((summary) => {
      if (sourceFilter !== "all" && summary.source !== sourceFilter) return false;
      if (!query) return true;
      return [
        cleanTitle(summary.title),
        summary.workspaceDirectory,
        summary.sessionType,
        summary.source,
        sourceMeta(summary.source).label,
      ].some((value) => value.toLocaleLowerCase("zh-CN").includes(query));
    });
  }, [activeWorkspace, sessionQuery, sourceFilter]);

  const groupedSessions = useMemo<SessionGroupView[]>(() => {
    const buckets: Record<SessionGroupView["key"], AiSessionSummary[]> = {
      today: [],
      yesterday: [],
      earlier: [],
    };
    for (const summary of visibleSessions) {
      buckets[dateGroup(summary.modifiedAt)].push(summary);
    }
    return (Object.keys(buckets) as SessionGroupView["key"][])
      .filter((key) => buckets[key].length > 0)
      .map((key) => ({ key, label: SESSION_GROUP_META[key], sessions: buckets[key] }));
  }, [visibleSessions]);

  const pagedMessages = useMemo(() => {
    if (!session) return [];
    const start = (session.page - 1) * session.pageSize;
    return session.history
      .map((item, offset) => ({ item, index: start + offset }));
  }, [session]);

  const refreshTree = async () => {
    if (await loadTree(true)) {
      void message.success({ content: "会话索引已刷新", duration: 1.5 });
    }
  };

  const deleteSession = async (summary: AiSessionSummary) => {
    const key = "session:" + summary.workspaceHash + ":" + summary.sessionId;
    setMutatingKey(key);
    try {
      await commands.aiDeleteSession({
        workspaceHash: summary.workspaceHash,
        sessionId: summary.sessionId,
      });
      if (sameSession(selectedRef.current, summary)) {
        selectedRef.current = null;
        setSelectedSummary(null);
        setSession(null);
      }
      if (await loadTree(true)) {
        void message.success(isCodexIndexSession(summary) ? "残留索引已清理" : "会话已删除");
      } else {
        void message.warning("会话已删除，但列表刷新失败");
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setMutatingKey(null);
    }
  };

  const deleteWorkspace = async (workspace: WorkspaceView) => {
    const key = "workspace:" + workspace.hash;
    setMutatingKey(key);
    try {
      await commands.aiDeleteWorkspace({ workspaceHash: workspace.hash });
      if (selectedRef.current?.workspaceHash === workspace.hash) {
        selectedRef.current = null;
        setSelectedSummary(null);
        setSession(null);
      }
      if (await loadTree(true)) {
        void message.success("工作区会话已删除");
      } else {
        void message.warning("工作区会话已删除，但列表刷新失败");
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setMutatingKey(null);
    }
  };

  const exportSession = async (format: "json" | "markdown") => {
    if (!selectedSummary) return;
    const extension = format === "json" ? "json" : "md";
    const path = await save({
      title: format === "json" ? "导出 AI 会话 JSON" : "导出 AI 会话 Markdown",
      defaultPath: safeFileName(selectedSummary.title) + "." + extension,
      filters: [{
        name: format === "json" ? "JSON" : "Markdown",
        extensions: [extension],
      }],
    });
    if (!path) return;
    setMutatingKey("export");
    try {
      const savedTo = await commands.aiExportSession({
        workspaceHash: selectedSummary.workspaceHash,
        sessionId: selectedSummary.sessionId,
        format,
        path,
      });
      void message.success("会话已导出到 " + savedTo);
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setMutatingKey(null);
    }
  };

  const revealSession = async () => {
    if (!selectedSummary || !canRevealSession(selectedSummary)) return;
    setMutatingKey("reveal");
    try {
      await commands.aiRevealSessionFile({
        workspaceHash: selectedSummary.workspaceHash,
        sessionId: selectedSummary.sessionId,
      });
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setMutatingKey(null);
    }
  };

  const copyMessage = async (item: AiHistoryItem, key: string) => {
    try {
      await navigator.clipboard.writeText(historyText(item));
      setCopiedMessageKey(key);
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopiedMessageKey(null), 1400);
    } catch (error: unknown) {
      void message.error("复制失败：" + errorMessage(error));
    }
  };

  const openWorkspace = (workspace: WorkspaceView) => {
    setActiveWorkspaceHash(workspace.hash);
    setSourceFilter("all");
    if (selectedSummary?.workspaceHash !== workspace.hash && workspace.sessions[0]) {
      void selectSession(workspace.sessions[0]);
    }
  };

  const changeMessagePage = async (page: number) => {
    if (!selectedSummary) return;
    if (await loadSessionPage(selectedSummary, page, false)) {
      window.requestAnimationFrame(() => {
        conversationScrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
      });
    }
  };

  const readonlySelected = selectedSummary ? isReadOnlySession(selectedSummary) : false;
  const indexOnlySelected = selectedSummary ? isCodexIndexSession(selectedSummary) : false;
  const canRevealSelected = selectedSummary ? canRevealSession(selectedSummary) : false;
  const totalMessageCount = session?.totalMessages ?? selectedSummary?.messageCount ?? 0;
  const workspaceForDelete = activeWorkspace
    ? workspaces.find((workspace) => workspace.hash === activeWorkspace.hash) ?? activeWorkspace
    : null;
  const canDeleteWorkspace = workspaceForDelete
    ? workspaceForDelete.sessions.every((summary) => !isReadOnlySession(summary))
    : false;
  const hasMutation = mutatingKey !== null;

  return (
    <div className="ai-session-library">
      <header className="session-sidebar-header" ref={sessionSidebarHeaderRef}>
        <div className="session-sidebar-title">
          <h2 id="session-library-title">AI 会话</h2>
          <span className="session-total">{totalSessions}</span>
        </div>
        <p>本机全部项目与 AI 客户端记录</p>
      </header>

      <aside
        className="session-sidebar"
        ref={sessionSidebarRef}
        aria-labelledby="session-library-title"
      >
        <div className="session-workspace-row">
          <Dropdown
            trigger={["click"]}
            classNames={{ root: "session-workspace-dropdown" }}
            menu={{
              items: workspaces.map((workspace) => ({
                key: workspace.hash,
                label: (
                  <div className="session-workspace-menu-item">
                    <span><FolderOutlined /></span>
                    <div>
                      <strong>{workspace.name}</strong>
                      <small title={workspace.path}>{workspace.path}</small>
                    </div>
                    <b>{workspace.sessions.length}</b>
                  </div>
                ),
              })),
              onClick: ({ key }) => {
                const workspace = workspaces.find((item) => item.hash === key);
                if (workspace) openWorkspace(workspace);
              },
            }}
          >
            <button
              type="button"
              className="session-workspace-selector"
              disabled={workspaces.length === 0 || hasMutation}
              aria-label="切换项目工作区"
            >
              <span className="session-workspace-symbol"><FolderOutlined /></span>
              <span className="session-workspace-copy">
                <small>当前项目</small>
                <strong>{activeWorkspace?.name ?? "暂无项目"}</strong>
              </span>
              <span className="session-workspace-count">
                {workspaces.length}
              </span>
              <DownOutlined />
            </button>
          </Dropdown>
        </div>

        <div className="session-list-toolbar">
          <Input
            allowClear
            size="small"
            value={sessionQuery}
            prefix={<SearchOutlined />}
            placeholder="搜索会话"
            onChange={(event) => setSessionQuery(event.target.value)}
          />
          <Select
            size="small"
            value={sourceFilter}
            options={sourceOptions}
            aria-label="按会话来源筛选"
            onChange={setSourceFilter}
          />
        </div>

        <div className="session-list-scroll" ref={sessionListScrollRef}>
          <Spin spinning={treeLoading}>
            {!activeWorkspace || visibleSessions.length === 0 ? (
              <div className="session-list-empty">
                <Empty
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                  description={activeWorkspace?.sessions.length
                    ? "没有匹配的会话"
                    : "当前项目没有会话"}
                />
              </div>
            ) : groupedSessions.map((group) => (
              <section className="session-group" key={group.key}>
                <div className="session-group-title">
                  <span>{group.label}</span>
                  <span>{group.sessions.length} 条</span>
                </div>
                {group.sessions.map((summary) => {
                  const active = sameSession(selectedSummary, summary);
                  const readonly = isReadOnlySession(summary);
                  const indexOnly = isCodexIndexSession(summary);
                  const deletingSession = mutatingKey
                    === "session:" + summary.workspaceHash + ":" + summary.sessionId;
                  const sourceClass = summary.source.replace(/[^a-zA-Z0-9-]/g, "-");
                  return (
                    <article
                      key={summary.workspaceHash + ":" + summary.sessionId}
                      className={"session-sidebar-item" + (active ? " is-active" : "")}
                    >
                      <button
                        type="button"
                        className="session-sidebar-item__select"
                        aria-current={active ? "true" : undefined}
                        onClick={() => void selectSession(summary)}
                      >
                        <span className={"session-source-logo source-" + sourceClass}>
                          {sourceMonogram(summary.source)}
                        </span>
                        <span className="session-sidebar-item__copy">
                          <strong>{cleanTitle(summary.title)}</strong>
                          <span>
                            {sourceMeta(summary.source).label}
                            <i aria-hidden="true" />
                            {summary.messageCount} 条消息
                            {indexOnly && <em>仅索引</em>}
                          </span>
                        </span>
                        <span className="session-sidebar-item__time">
                          {formatClock(summary.modifiedAt)}
                        </span>
                      </button>

                      {readonly ? (
                        <Tooltip title={summary.source === "antigravity-backup"
                          ? "备份会话保持只读，避免破坏恢复数据"
                          : "该来源目前只提供只读索引"}
                        >
                          <LockOutlined className="session-sidebar-item__readonly" />
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
                          onConfirm={() => deleteSession(summary)}
                        >
                          <Button
                            className={"session-sidebar-item__delete"
                              + (indexOnly ? " is-index-cleanup" : "")}
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
                })}
              </section>
            ))}
          </Spin>
        </div>

        <footer className="session-sidebar-footer">
          <span>
            {activeWorkspace
              ? (visibleSessions.length === activeWorkspace.sessions.length
                  ? activeWorkspace.sessions.length + " 条项目会话"
                  : visibleSessions.length + " / " + activeWorkspace.sessions.length + " 条会话")
              : "未选择项目"}
          </span>
          <div>
            {workspaceForDelete && (
              <Tooltip
                title={canDeleteWorkspace
                  ? "删除当前项目的全部会话"
                  : "项目包含只读索引或备份会话，不能整组删除"}
              >
                <span>
                  <Popconfirm
                    title="删除项目会话"
                    description={"确定删除 “" + workspaceForDelete.name + "” 下的全部会话？此操作不可恢复。"}
                    okText="全部删除"
                    cancelText="取消"
                    okButtonProps={{ danger: true }}
                    disabled={!canDeleteWorkspace || hasMutation}
                    onConfirm={() => deleteWorkspace(workspaceForDelete)}
                  >
                    <Button
                      type="text"
                      size="small"
                      danger
                      disabled={!canDeleteWorkspace || (hasMutation
                        && mutatingKey !== "workspace:" + workspaceForDelete.hash)}
                      loading={mutatingKey === "workspace:" + workspaceForDelete.hash}
                      icon={<DeleteOutlined />}
                    >
                      删除项目
                    </Button>
                  </Popconfirm>
                </span>
              </Tooltip>
            )}
            <Button
              type="text"
              size="small"
              icon={<ReloadOutlined />}
              loading={treeLoading}
              disabled={hasMutation}
              onClick={() => void refreshTree()}
            >
              重新扫描
            </Button>
          </div>
        </footer>
      </aside>

      <header className="conversation-header" ref={conversationHeaderRef}>
        <div className="conversation-heading">
          <div className="conversation-heading-line">
            <h2 id="conversation-title">
              {selectedSummary ? cleanTitle(selectedSummary.title) : "选择一条会话"}
            </h2>
            {selectedSummary && (
              <div className="conversation-controls">
                <span className="conversation-source-badge">
                  {sourceMeta(selectedSummary.source).label}
                </span>
                {readonlySelected && <span className="conversation-state-badge"><LockOutlined />只读</span>}
                {indexOnlySelected && <span className="conversation-state-badge"><DatabaseOutlined />仅索引</span>}
                <div className="conversation-info">
                  <span><MessageOutlined /><strong>{totalMessageCount}</strong> 条消息</span>
                  <span><DatabaseOutlined /><strong>{formatFileSize(selectedSummary.fileSize)}</strong></span>
                  <span>更新于 {formatTime(selectedSummary.modifiedAt)}</span>
                </div>
                <div className="conversation-actions">
                  <Tooltip
                    title={!canRevealSelected
                      ? (indexOnlySelected
                          ? "正文文件已不存在，只能查看恢复内容或清理残留索引"
                          : "该索引会话没有独立正文文件")
                      : "在资源管理器中定位会话文件"}
                  >
                    <span>
                      <Button
                        aria-label="定位文件"
                        icon={<FolderOpenOutlined />}
                        disabled={!canRevealSelected || (hasMutation && mutatingKey !== "reveal")}
                        loading={mutatingKey === "reveal"}
                        onClick={() => void revealSession()}
                      >
                        定位文件
                      </Button>
                    </span>
                  </Tooltip>
                  <Dropdown
                    trigger={["click"]}
                    menu={{
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
                    }}
                  >
                    <Button
                      aria-label="导出会话"
                      icon={<DownloadOutlined />}
                      loading={mutatingKey === "export"}
                      disabled={hasMutation && mutatingKey !== "export"}
                    >
                      导出
                    </Button>
                  </Dropdown>
                  {!readonlySelected && (
                    <Popconfirm
                      title={indexOnlySelected ? "清理残留索引" : "删除会话"}
                      description={indexOnlySelected
                        ? "正文文件已经不存在。将清理 Codex 中残留的索引和数据库记录，此操作不可恢复。"
                        : "确定删除当前会话？此操作不可恢复。"}
                      okText={indexOnlySelected ? "清理索引" : "删除"}
                      cancelText="取消"
                      okButtonProps={{ danger: true }}
                      disabled={hasMutation}
                      onConfirm={() => deleteSession(selectedSummary)}
                    >
                      <Button
                        className="conversation-danger-action"
                        danger
                        disabled={hasMutation}
                        aria-label={indexOnlySelected ? "清理残留索引" : "删除当前会话"}
                        icon={<DeleteOutlined />}
                      />
                    </Popconfirm>
                  )}
                </div>
              </div>
            )}
          </div>
        </div>
      </header>

      <section
        className="conversation-panel"
        ref={conversationPanelRef}
        aria-labelledby="conversation-title"
      >
        <div className="conversation-scroll" ref={conversationScrollRef}>
          {!selectedSummary ? (
            <div className="conversation-empty">
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description="从左侧选择一条会话"
              />
            </div>
          ) : sessionLoading ? (
            <div className="conversation-empty">
              <Spin size="large" description="正在解析会话内容…" />
            </div>
          ) : !session ? (
            <div className="conversation-empty">
              <Empty description="会话内容加载失败，请刷新后重试" />
            </div>
          ) : (
            <div className="message-stream">
              {session.conversationSummary?.trim() && (
                <article className="conversation-message is-summary">
                  <div className="conversation-message__avatar"><FileSearchOutlined /></div>
                  <div className="conversation-message__main">
                    <header className="conversation-message__header">
                      <div><strong>上下文摘要</strong><span>会话持续记忆</span></div>
                    </header>
                    <div className="conversation-message__content">
                      <MarkdownContent text={session.conversationSummary} />
                    </div>
                  </div>
                </article>
              )}

              {pagedMessages.length === 0 ? (
                <div className="conversation-empty is-inline">
                  <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无消息" />
                </div>
              ) : pagedMessages.map(({ item, index }) => {
                const role = item.message.role || "system";
                const key = (item.message.id || "message") + ":" + index;
                return (
                  <article
                    key={key}
                    className={"conversation-message role-" + role
                      + (role === "user" ? " is-user" : "")}
                  >
                    <div className="conversation-message__avatar">
                      {role === "user"
                        ? <UserOutlined />
                        : role === "assistant"
                          ? <RobotOutlined />
                          : <FileSearchOutlined />}
                    </div>
                    <div className="conversation-message__main">
                      <header className="conversation-message__header">
                        <div>
                          <strong>{roleLabel(role)}</strong>
                          <span>消息 {String(index + 1).padStart(2, "0")}</span>
                          {item.message.isHidden && <Tag>隐藏消息</Tag>}
                        </div>
                        <Tooltip title={copiedMessageKey === key ? "已复制" : "复制此条消息"}>
                          <Button
                            className={"conversation-message__copy"
                              + (copiedMessageKey === key ? " is-copied" : "")}
                            type="text"
                            size="small"
                            icon={copiedMessageKey === key ? <CheckOutlined /> : <CopyOutlined />}
                            aria-label="复制此条消息"
                            onClick={() => void copyMessage(item, key)}
                          />
                        </Tooltip>
                      </header>
                      <div className="conversation-message__content">
                        {item.message.content.length === 0 ? (
                          <Typography.Text type="secondary">空消息</Typography.Text>
                        ) : item.message.content.map((content, contentIndex) => (
                          <div key={key + ":" + contentIndex}>
                            {content.type && content.type !== "text" && (
                              <Tag className="conversation-message__type" variant="filled">
                                {content.type}
                              </Tag>
                            )}
                            <MarkdownContent text={content.text} />
                          </div>
                        ))}
                      </div>
                    </div>
                  </article>
                );
              })}

              {session.totalMessages > session.pageSize && (
                <footer className="conversation-pagination">
                  <span>
                    第 {messagePage} / {Math.ceil(session.totalMessages / session.pageSize)} 页
                  </span>
                  <Pagination
                    size="small"
                    current={messagePage}
                    pageSize={session.pageSize}
                    total={session.totalMessages}
                    showSizeChanger={false}
                    disabled={sessionLoading || hasMutation}
                    onChange={(page) => void changeMessagePage(page)}
                  />
                </footer>
              )}
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
