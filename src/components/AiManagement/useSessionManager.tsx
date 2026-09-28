import type { AiHistoryItem, AiSessionPage, AiSessionSummary, AiSessionTree, } from "@/bindings";
import { commands } from "@/bindings";
import { save } from "@tauri-apps/plugin-dialog";
import { App as AntdApp } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bindPanelWheelRouting } from "./panelWheel";
import { canRevealSession, isCodexIndexSession, isReadOnlySession, } from "./sessionCapabilities";
import "./SessionManager.css";
const EMPTY_TREE: AiSessionTree = {
    workspaces: [],
    sessionsByWorkspace: {},
};
const PAGE_SIZE = 25;
import { cleanTitle, dateGroup, errorMessage, historyText, safeFileName, sameSession, SESSION_GROUP_META, sourceMeta, workspaceName, workspacePath, type SessionGroupView, type WorkspaceView } from "./sessionPresentation";
export function useSessionManager() {
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
    const sessionSidebarRef = useRef<HTMLElement | null>(null);
    const conversationPanelRef = useRef<HTMLElement | null>(null);
    useEffect(() => () => {
        if (copyTimer.current !== null)
            window.clearTimeout(copyTimer.current);
    }, []);
    useEffect(() => {
        // 侧栏与会话面板整体接管滚轮，指针停在标题、筛选或分页上时也滚动对应列表。
        const cleanups: Array<() => void> = [];
        if (sessionSidebarRef.current && sessionListScrollRef.current) {
            cleanups.push(bindPanelWheelRouting(sessionSidebarRef.current, sessionListScrollRef.current));
        }
        if (conversationPanelRef.current && conversationScrollRef.current) {
            cleanups.push(bindPanelWheelRouting(conversationPanelRef.current, conversationScrollRef.current));
        }
        return () => cleanups.forEach((cleanup) => cleanup());
    }, []);
    const loadSessionPage = useCallback(async (summary: AiSessionSummary, page: number, clearCurrent: boolean): Promise<boolean> => {
        const sequence = ++loadSequence.current;
        selectedRef.current = summary;
        setActiveWorkspaceHash(summary.workspaceHash);
        setSelectedSummary(summary);
        if (clearCurrent)
            setSession(null);
        setSessionLoading(true);
        setMessagePage(page);
        try {
            const loaded = await commands.aiLoadSession({
                workspaceHash: summary.workspaceHash,
                sessionId: summary.sessionId,
                page,
                pageSize: PAGE_SIZE,
            });
            if (sequence !== loadSequence.current)
                return false;
            setSession(loaded);
            setMessagePage(loaded.page);
            return true;
        }
        catch (error: unknown) {
            if (sequence !== loadSequence.current)
                return false;
            void message.error(errorMessage(error));
            return false;
        }
        finally {
            if (sequence === loadSequence.current)
                setSessionLoading(false);
        }
    }, [message]);
    const selectSession = useCallback(async (summary: AiSessionSummary) => {
        await loadSessionPage(summary, 1, true);
    }, [loadSessionPage]);
    const loadTree = useCallback(async (invalidateCache: boolean): Promise<boolean> => {
        const sequence = ++treeLoadSequence.current;
        setTreeLoading(true);
        try {
            if (invalidateCache)
                await commands.aiRefreshSessionCache();
            const nextTree = await commands.aiListSessionTree();
            if (sequence !== treeLoadSequence.current)
                return false;
            setTree(nextTree);
            setActiveWorkspaceHash((current) => current && nextTree.workspaces.includes(current)
                ? current
                : nextTree.workspaces[0] ?? null);
            const summaries = nextTree.workspaces.flatMap((hash) => nextTree.sessionsByWorkspace[hash] ?? []);
            const current = selectedRef.current;
            const retained = current
                ? summaries.find((item) => sameSession(current, item))
                : undefined;
            const target = retained ?? summaries[0];
            if (target) {
                // 树列表加载完成后立即解除列表 loading；详情解析使用独立状态，
                // 避免大会话阻塞工作区和会话列表交互。
                void selectSession(target);
            }
            else {
                ++loadSequence.current;
                selectedRef.current = null;
                setSelectedSummary(null);
                setSession(null);
                setSessionLoading(false);
            }
            return true;
        }
        catch (error: unknown) {
            if (sequence === treeLoadSequence.current) {
                void message.error(errorMessage(error));
            }
            return false;
        }
        finally {
            if (sequence === treeLoadSequence.current)
                setTreeLoading(false);
        }
    }, [message, selectSession]);
    useEffect(() => {
        void loadTree(false);
    }, [loadTree]);
    const workspaces = useMemo<WorkspaceView[]>(() => tree.workspaces.map((hash) => {
        const sessions = [...(tree.sessionsByWorkspace[hash] ?? [])].sort((left, right) => (right.modifiedAt ?? 0) - (left.modifiedAt ?? 0));
        const path = workspacePath(hash, sessions);
        return { hash, path, name: workspaceName(path), sessions };
    }), [tree]);
    const totalSessions = useMemo(() => workspaces.reduce((total, workspace) => total + workspace.sessions.length, 0), [workspaces]);
    const activeWorkspace = useMemo(() => workspaces.find((workspace) => workspace.hash === activeWorkspaceHash)
        ?? workspaces[0]
        ?? null, [activeWorkspaceHash, workspaces]);
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
            if (sourceFilter !== "all" && summary.source !== sourceFilter)
                return false;
            if (!query)
                return true;
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
        if (!session)
            return [];
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
            }
            else {
                void message.warning("会话已删除，但列表刷新失败");
            }
        }
        catch (error: unknown) {
            void message.error(errorMessage(error));
        }
        finally {
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
            }
            else {
                void message.warning("工作区会话已删除，但列表刷新失败");
            }
        }
        catch (error: unknown) {
            void message.error(errorMessage(error));
        }
        finally {
            setMutatingKey(null);
        }
    };
    const exportSession = async (format: "json" | "markdown") => {
        if (!selectedSummary)
            return;
        const extension = format === "json" ? "json" : "md";
        const path = await save({
            title: format === "json" ? "导出 AI 会话 JSON" : "导出 AI 会话 Markdown",
            defaultPath: safeFileName(selectedSummary.title) + "." + extension,
            filters: [{
                    name: format === "json" ? "JSON" : "Markdown",
                    extensions: [extension],
                }],
        });
        if (!path)
            return;
        setMutatingKey("export");
        try {
            const savedTo = await commands.aiExportSession({
                workspaceHash: selectedSummary.workspaceHash,
                sessionId: selectedSummary.sessionId,
                format,
                path,
            });
            void message.success("会话已导出到 " + savedTo);
        }
        catch (error: unknown) {
            void message.error(errorMessage(error));
        }
        finally {
            setMutatingKey(null);
        }
    };
    const revealSession = async () => {
        if (!selectedSummary || !canRevealSession(selectedSummary))
            return;
        setMutatingKey("reveal");
        try {
            await commands.aiRevealSessionFile({
                workspaceHash: selectedSummary.workspaceHash,
                sessionId: selectedSummary.sessionId,
            });
        }
        catch (error: unknown) {
            void message.error(errorMessage(error));
        }
        finally {
            setMutatingKey(null);
        }
    };
    const copyMessage = async (item: AiHistoryItem, key: string) => {
        try {
            await navigator.clipboard.writeText(historyText(item));
            setCopiedMessageKey(key);
            if (copyTimer.current !== null)
                window.clearTimeout(copyTimer.current);
            copyTimer.current = window.setTimeout(() => setCopiedMessageKey(null), 1400);
        }
        catch (error: unknown) {
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
        if (!selectedSummary)
            return;
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
    const totalPages = session ? Math.ceil(session.totalMessages / session.pageSize) : 1;
    return { activeWorkspace, activeWorkspaceHash, canDeleteWorkspace, canRevealSelected, changeMessagePage, conversationPanelRef, conversationScrollRef, copiedMessageKey, copyMessage, deleteSession, deleteWorkspace, exportSession, groupedSessions, hasMutation, indexOnlySelected, messagePage, mutatingKey, openWorkspace, pagedMessages, readonlySelected, refreshTree, revealSession, selectSession, selectedSummary, session, sessionListScrollRef, sessionLoading, sessionQuery, sessionSidebarRef, setSessionQuery, setSourceFilter, sourceFilter, sourceOptions, totalMessageCount, totalPages, totalSessions, treeLoading, workspaceForDelete, workspaces };
}
export type SessionModel = ReturnType<typeof useSessionManager>;
