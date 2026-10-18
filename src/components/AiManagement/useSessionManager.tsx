import type { AiHistoryItem, AiSessionPage, AiSessionSummary, AiSessionTree, } from "@/bindings";
import { commands } from "@/bindings";
import { save } from "@tauri-apps/plugin-dialog";
import { App as AntdApp } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bindPanelWheelRouting } from "./panelWheel";
import { buildWorkspaceViews, filterSessions, nextSessionAfterRemoval, resolveTreeSelection } from "./sessionNavigation";
import { canRevealSession, isCodexIndexSession, isReadOnlySession, } from "./sessionCapabilities";
import "./SessionManager.css";
const EMPTY_TREE: AiSessionTree = {
    workspaces: [],
    sessionsByWorkspace: {},
};
const PAGE_SIZE = 25;
import { dateGroup, errorMessage, historyText, safeFileName, sameSession, SESSION_GROUP_META, sourceMeta, type SessionGroupView, type WorkspaceView } from "./sessionPresentation";
export function useSessionManager() {
    const { message } = AntdApp.useApp();
    const [tree, setTree] = useState<AiSessionTree>(EMPTY_TREE);
    const treeRef = useRef(tree);
    treeRef.current = tree;
    const [treeLoading, setTreeLoading] = useState(false);
    const treeLoadingRef = useRef(false);
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
    const activeWorkspaceRef = useRef<string | null>(null);
    const messagePageRef = useRef(1);
    const workspaceSelections = useRef(new Map<string, { summary: AiSessionSummary; page: number }>());
    const mutationRef = useRef<string | null>(null);
    const mountedRef = useRef(true);
    const loadSequence = useRef(0);
    const treeLoadSequence = useRef(0);
    const copyTimer = useRef<number | null>(null);
    const sessionListScrollRef = useRef<HTMLDivElement | null>(null);
    const conversationScrollRef = useRef<HTMLDivElement | null>(null);
    const sessionSidebarRef = useRef<HTMLElement | null>(null);
    const conversationPanelRef = useRef<HTMLElement | null>(null);
    const beginMutation = (key: string) => {
        if (mutationRef.current) return false;
        mutationRef.current = key;
        setMutatingKey(key);
        return true;
    };
    const endMutation = () => { mutationRef.current = null; setMutatingKey(null); };
    const clearSelection = useCallback(() => {
        ++loadSequence.current;
        selectedRef.current = null;
        messagePageRef.current = 1;
        setSelectedSummary(null);
        setSession(null);
        setSessionLoading(false);
        setMessagePage(1);
    }, []);
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
        activeWorkspaceRef.current = summary.workspaceHash;
        messagePageRef.current = page;
        workspaceSelections.current.set(summary.workspaceHash, { summary, page });
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
            messagePageRef.current = loaded.page;
            workspaceSelections.current.set(summary.workspaceHash, { summary, page: loaded.page });
            setMessagePage(loaded.page);
            return true;
        }
        catch (error: unknown) {
            if (sequence !== loadSequence.current)
                return false;
            setSession(null);
            void message.error(errorMessage(error));
            return false;
        }
        finally {
            if (sequence === loadSequence.current)
                setSessionLoading(false);
        }
    }, [message]);
    const selectSession = useCallback(async (summary: AiSessionSummary) => {
        if (mutationRef.current || sameSession(selectedRef.current, summary)) return;
        if (await loadSessionPage(summary, 1, true)) conversationScrollRef.current?.scrollTo({ top: 0 });
    }, [loadSessionPage]);
    const loadTree = useCallback(async (invalidateCache: boolean, refreshCurrent = true): Promise<boolean> => {
        const sequence = ++treeLoadSequence.current;
        treeLoadingRef.current = true;
        setTreeLoading(true);
        try {
            if (invalidateCache)
                await commands.aiRefreshSessionCache();
            const nextTree = await commands.aiListSessionTree();
            if (sequence !== treeLoadSequence.current)
                return false;
            setTree(nextTree);
            treeRef.current = nextTree;
            for (const hash of workspaceSelections.current.keys()) {
                if (!nextTree.workspaces.includes(hash)) workspaceSelections.current.delete(hash);
            }
            const current = selectedRef.current;
            const { workspace, summary: target } = resolveTreeSelection(buildWorkspaceViews(nextTree), activeWorkspaceRef.current, current);
            const nextHash = workspace?.hash ?? null;
            if (activeWorkspaceRef.current !== nextHash) { setSessionQuery(""); setSourceFilter("all"); }
            activeWorkspaceRef.current = nextHash;
            setActiveWorkspaceHash(nextHash);
            if (target) {
                // 树列表加载完成后立即解除列表 loading；详情解析使用独立状态，
                // 避免大会话阻塞工作区和会话列表交互。
                const retained = sameSession(current, target);
                if (!retained || refreshCurrent) void loadSessionPage(target, retained ? messagePageRef.current : 1, !retained);
            }
            else {
                clearSelection();
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
            if (sequence === treeLoadSequence.current) {
                treeLoadingRef.current = false;
                setTreeLoading(false);
            }
        }
    }, [message, loadSessionPage, clearSelection]);
    useEffect(() => {
        mountedRef.current = true;
        void loadTree(false);
        return () => { mountedRef.current = false; ++treeLoadSequence.current; ++loadSequence.current; };
    }, [loadTree]);
    const workspaces = useMemo(() => buildWorkspaceViews(tree), [tree]);
    const totalSessions = useMemo(() => workspaces.reduce((total, workspace) => total + workspace.sessions.length, 0), [workspaces]);
    const activeWorkspace = useMemo(() => workspaces.find((workspace) => workspace.hash === activeWorkspaceHash)
        ?? workspaces[0]
        ?? null, [activeWorkspaceHash, workspaces]);
    const sourceOptions = useMemo(() => {
        const sources = new Map<string, number>();
        for (const summary of activeWorkspace?.sessions ?? []) sources.set(summary.source, (sources.get(summary.source) ?? 0) + 1);
        return [
            { value: "all", label: `全部来源 · ${activeWorkspace?.sessions.length ?? 0}` },
            ...[...sources.keys()]
                .sort((left, right) => sourceMeta(left).label.localeCompare(sourceMeta(right).label, "zh-CN"))
                .map((source) => ({ value: source, label: `${sourceMeta(source).label} · ${sources.get(source)}` })),
        ];
    }, [activeWorkspace]);
    useEffect(() => {
        if (!sourceOptions.some((option) => option.value === sourceFilter)) setSourceFilter("all");
    }, [sourceOptions, sourceFilter]);
    const visibleSessions = useMemo(() => filterSessions(activeWorkspace?.sessions ?? [], sessionQuery, sourceFilter), [activeWorkspace, sessionQuery, sourceFilter]);
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
        if (mutationRef.current || treeLoadingRef.current) return;
        if (await loadTree(true)) {
            void message.success({ content: "会话索引已刷新", duration: 1.5 });
        }
    };
    const deleteSession = async (summary: AiSessionSummary) => {
        const key = "session:" + summary.workspaceHash + ":" + summary.sessionId;
        if (isReadOnlySession(summary) || !beginMutation(key)) return;
        ++treeLoadSequence.current;
        treeLoadingRef.current = false;
        setTreeLoading(false);
        try {
            await commands.aiDeleteSession({
                workspaceHash: summary.workspaceHash,
                sessionId: summary.sessionId,
            });
            if (!mountedRef.current) return;
            const previous = treeRef.current;
            const remaining = (previous.sessionsByWorkspace[summary.workspaceHash] ?? []).filter((s) => !sameSession(summary, s));
            const nextTree = { ...previous, sessionsByWorkspace: { ...previous.sessionsByWorkspace, [summary.workspaceHash]: remaining } };
            treeRef.current = nextTree;
            setTree(nextTree);
            const remembered = workspaceSelections.current.get(summary.workspaceHash);
            if (remembered && sameSession(remembered.summary, summary)) workspaceSelections.current.delete(summary.workspaceHash);
            if (sameSession(selectedRef.current, summary)) {
                const neighbor = nextSessionAfterRemoval(visibleSessions, summary) ?? filterSessions(remaining, sessionQuery, sourceFilter)[0] ?? remaining[0];
                clearSelection();
                if (neighbor) void loadSessionPage(neighbor, 1, true);
            }
            if (await loadTree(true, false)) {
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
            endMutation();
        }
    };
    const deleteWorkspace = async (workspace: WorkspaceView) => {
        const key = "workspace:" + workspace.hash;
        const currentSessions = treeRef.current.sessionsByWorkspace[workspace.hash] ?? [];
        if (!currentSessions.length || currentSessions.some(isReadOnlySession) || !beginMutation(key)) return;
        const confirmedIds = new Set(workspace.sessions.map((s) => s.sessionId));
        if (currentSessions.length !== workspace.sessions.length || currentSessions.some((s) => !confirmedIds.has(s.sessionId))) {
            endMutation();
            void message.warning("会话列表已变化，请重新打开删除确认");
            return;
        }
        ++treeLoadSequence.current;
        treeLoadingRef.current = false;
        setTreeLoading(false);
        try {
            await commands.aiDeleteWorkspace({ workspaceHash: workspace.hash });
            if (!mountedRef.current) return;
            const previous = treeRef.current;
            const sessionsByWorkspace = { ...previous.sessionsByWorkspace };
            delete sessionsByWorkspace[workspace.hash];
            const nextTree = { workspaces: previous.workspaces.filter((hash) => hash !== workspace.hash), sessionsByWorkspace };
            treeRef.current = nextTree;
            setTree(nextTree);
            workspaceSelections.current.delete(workspace.hash);
            if (activeWorkspaceRef.current === workspace.hash) {
                clearSelection();
                const next = resolveTreeSelection(buildWorkspaceViews(nextTree), null, null);
                activeWorkspaceRef.current = next.workspace?.hash ?? null;
                setActiveWorkspaceHash(activeWorkspaceRef.current);
                setSessionQuery("");
                setSourceFilter("all");
                if (next.summary) void loadSessionPage(next.summary, 1, true);
            }
            if (await loadTree(true, false)) {
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
            endMutation();
        }
    };
    const exportSession = async (format: "json" | "markdown") => {
        const target = selectedRef.current;
        if (!target || !beginMutation("export")) return;
        try {
            const extension = format === "json" ? "json" : "md";
            const path = await save({
                title: format === "json" ? "导出 AI 会话 JSON" : "导出 AI 会话 Markdown",
                defaultPath: safeFileName(target.title) + "." + extension,
                filters: [{
                        name: format === "json" ? "JSON" : "Markdown",
                        extensions: [extension],
                    }],
            });
            if (!path)
                return;
            const savedTo = await commands.aiExportSession({
                workspaceHash: target.workspaceHash,
                sessionId: target.sessionId,
                format,
                path,
            });
            void message.success("会话已导出到 " + savedTo);
        }
        catch (error: unknown) {
            void message.error(errorMessage(error));
        }
        finally {
            endMutation();
        }
    };
    const revealSession = async () => {
        const target = selectedRef.current;
        if (!target || !canRevealSession(target) || !beginMutation("reveal"))
            return;
        try {
            await commands.aiRevealSessionFile({
                workspaceHash: target.workspaceHash,
                sessionId: target.sessionId,
            });
        }
        catch (error: unknown) {
            void message.error(errorMessage(error));
        }
        finally {
            endMutation();
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
        if (mutationRef.current || activeWorkspaceRef.current === workspace.hash) return;
        activeWorkspaceRef.current = workspace.hash;
        setActiveWorkspaceHash(workspace.hash);
        setSourceFilter("all");
        setSessionQuery("");
        const remembered = workspaceSelections.current.get(workspace.hash);
        const target = workspace.sessions.find((s) => remembered && sameSession(remembered.summary, s)) ?? workspace.sessions[0];
        clearSelection();
        if (target) void loadSessionPage(target, remembered && sameSession(remembered.summary, target) ? remembered.page : 1, true);
        conversationScrollRef.current?.scrollTo({ top: 0 });
    };
    const changeMessagePage = async (page: number) => {
        const target = selectedRef.current;
        if (!target || mutationRef.current)
            return;
        if (await loadSessionPage(target, page, true)) {
            window.requestAnimationFrame(() => {
                if (sameSession(selectedRef.current, target)) conversationScrollRef.current?.scrollTo({ top: 0 });
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
        ? workspaceForDelete.sessions.length > 0 && workspaceForDelete.sessions.every((summary) => !isReadOnlySession(summary))
        : false;
    const hasMutation = mutatingKey !== null;
    const totalPages = session ? Math.ceil(session.totalMessages / session.pageSize) : 1;
    const clearFilters = () => { setSessionQuery(""); setSourceFilter("all"); };
    const selectedOutsideFilter = !!selectedSummary && selectedSummary.workspaceHash === activeWorkspace?.hash && !visibleSessions.some((s) => sameSession(selectedSummary, s));
    const retrySession = () => selectedRef.current ? loadSessionPage(selectedRef.current, messagePageRef.current, true) : Promise.resolve(false);
    return { activeWorkspace, activeWorkspaceHash, canDeleteWorkspace, canRevealSelected, changeMessagePage, conversationPanelRef, conversationScrollRef, copiedMessageKey, copyMessage, deleteSession, deleteWorkspace, exportSession, groupedSessions, hasMutation, indexOnlySelected, messagePage, mutatingKey, openWorkspace, pagedMessages, readonlySelected, refreshTree, revealSession, selectSession, selectedSummary, session, sessionListScrollRef, sessionLoading, sessionQuery, sessionSidebarRef, setSessionQuery, setSourceFilter, sourceFilter, sourceOptions, totalMessageCount, totalPages, totalSessions, treeLoading, workspaceForDelete, workspaces, visibleSessions, clearFilters, selectedOutsideFilter, retrySession };
}
export type SessionModel = ReturnType<typeof useSessionManager>;
