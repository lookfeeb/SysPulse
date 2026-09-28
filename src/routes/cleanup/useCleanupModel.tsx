import { disposeAsyncListeners } from "@/utils/disposeAsyncListeners";
import type { CleanupCategory, PathDetail, ScanResult } from "@/bindings";
import { commands } from "@/bindings";
import type { DisplayCategory } from "@/routes/cleanupSelection";
import { MAX_PROJECT_ROOTS, PROGRAMMING_CATEGORY_ID, addProjectRoot, buildDisplayCategories, cleanablePaths, cleanupCategorySelection, displayCategorySelection, normalizePathKey, normalizeProjectRoots, replaceProjectRoot, restoreSelectedCategories, scanButtonState, setCategoriesCheckedState, sumPathFiles, sumPathSize, togglePathState, validExcludedPaths } from "@/routes/cleanupSelection";
import { fmtBytes } from "@/utils/format";
import { readStoredStringList, writeStoredStringList } from "@/utils/storageList";
import { CodeOutlined } from "@ant-design/icons";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { App as AntdApp, Checkbox, Space, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useMemo, useState } from "react";
const { Text } = Typography;
const SELECTED_STORAGE_KEY = "syspulse.cleanup.selectedCategories.v4";
const EXCLUDED_PATHS_STORAGE_KEY = "syspulse.cleanup.excludedPaths.v1";
const PROJECT_ROOTS_STORAGE_KEY = "syspulse.cleanup.projectRoots.v1";
type CleanupCache = ScanResult;
let cleanupCache: CleanupCache | null = null;
type CleanupProgressEvent = {
    percent: number;
    processedItems: number;
    totalItems: number;
    currentCategory: string;
    currentPath: string | null;
    freedBytes: number;
    deletedFiles: number;
    done: boolean;
};
type CleanupScanProgressEvent = {
    stage: "queen" | "scout" | "engineer" | "done" | string;
    phase: string;
    scannedPaths: number;
    skippedPaths: number;
    ignoredPaths: number;
    scoutTasks: number;
    engineerTasks: number;
    currentPath: string | null;
    done: boolean;
    cancelled: boolean;
};
type RiskMeta = {
    label: string;
    color: string;
    background: string;
};
function restoreSelected(categories: CleanupCategory[]): Set<string> {
    return restoreSelectedCategories(categories, readStoredStringList(SELECTED_STORAGE_KEY));
}
function restoreExcludedPaths(): Set<string> {
    return new Set((readStoredStringList(EXCLUDED_PATHS_STORAGE_KEY) ?? []).map(normalizePathKey));
}
function riskMeta(risk: CleanupCategory["riskLevel"]): RiskMeta {
    if (risk === "safe")
        return { label: "安全", color: "#16845b", background: "#e7f8f0" };
    if (risk === "caution")
        return { label: "谨慎", color: "#a96b00", background: "#fff3d8" };
    return { label: "高级", color: "#b42335", background: "#fdecef" };
}
function sourceLabel(source: string): string {
    const labels: Record<string, string> = {
        "project-discovery": "项目发现",
        "user-directory": "用户目录",
        "system-directory": "系统目录",
        "tool-global-cache": "工具全局缓存",
        "hotspot-index": "热点索引",
    };
    return labels[source] ?? (source || "规则扫描");
}
function stageMeta(stage: string): {
    label: string;
    color: string;
} {
    if (stage === "scout")
        return { label: "侦察蚁", color: "blue" };
    if (stage === "engineer")
        return { label: "工兵蚁", color: "cyan" };
    if (stage === "done")
        return { label: "已完成", color: "green" };
    return { label: "蚁后", color: "purple" };
}
function categoryIcon(id: string): string {
    const icons: Record<string, string> = {
        "win-temp": "♻️",
        "win-update": "🔄",
        "rust-target": "🦀",
        "node-cache": "📦",
        "go-cache": "🐹",
        "python-cache": "🐍",
        "cpp-cache": "C++",
        "dotnet-cache": ".NET",
        "java-cache": "☕",
        "browser-cache": "🌐",
        "webview-cache": "🧩",
        "app-cache": "🧰",
        thumbnails: "🖼️",
        "notion-cache": "📝",
        "wer-cache": "📋",
        "shader-cache": "🎮",
        "installer-cache": "📥",
    };
    return icons[id] ?? "📁";
}
function compactNumber(value: number): string {
    return value.toLocaleString();
}
function relativeScanTime(scannedAtMs: number, nowMs: number): string {
    const elapsedMs = Math.max(0, nowMs - scannedAtMs);
    const elapsedMinutes = Math.floor(elapsedMs / 60000);
    if (elapsedMinutes < 1)
        return "刚刚";
    if (elapsedMinutes < 60)
        return `${elapsedMinutes} 分钟前`;
    const elapsedHours = Math.floor(elapsedMinutes / 60);
    if (elapsedHours < 24)
        return `${elapsedHours} 小时前`;
    return new Date(scannedAtMs).toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}
export function useCleanupModel() {
    const { message, modal } = AntdApp.useApp();
    const initialCache = cleanupCache && cleanupCache.expiresAtMs > Date.now() ? cleanupCache : null;
    const [categories, setCategories] = useState<CleanupCategory[]>(initialCache?.categories ?? []);
    const [selected, setSelected] = useState<Set<string>>(() => restoreSelected(initialCache?.categories ?? []));
    const [excludedPaths, setExcludedPaths] = useState<Set<string>>(() => restoreExcludedPaths());
    const [projectRoots, setProjectRoots] = useState<string[]>(() => normalizeProjectRoots(readStoredStringList(PROJECT_ROOTS_STORAGE_KEY) ?? []));
    const [scanning, setScanning] = useState(false);
    const [cancelRequested, setCancelRequested] = useState(false);
    const [cleaning, setCleaning] = useState(false);
    const [exporting, setExporting] = useState(false);
    const [detailCat, setDetailCat] = useState<DisplayCategory | null>(null);
    const [activeToolchainId, setActiveToolchainId] = useState<string | null>(null);
    const [projectManagerOpen, setProjectManagerOpen] = useState(false);
    const [scanProgress, setScanProgress] = useState<CleanupScanProgressEvent | null>(null);
    const [cleanProgress, setCleanProgress] = useState<CleanupProgressEvent | null>(null);
    const [scanId, setScanId] = useState<string | null>(initialCache?.scanId ?? null);
    const [scanExpiresAt, setScanExpiresAt] = useState<number | null>(initialCache?.expiresAtMs ?? null);
    const [nowMs, setNowMs] = useState(Date.now());
    const displayCategories = useMemo(() => buildDisplayCategories(categories), [categories]);
    const sortedDisplayCategories = useMemo(() => [...displayCategories].sort((left, right) => {
        const order = { safe: 0, caution: 1, advanced: 2 } as const;
        return order[left.riskLevel] - order[right.riskLevel];
    }), [displayCategories]);
    const busy = scanning || cleaning || exporting;
    const scanExpired = scanExpiresAt !== null && nowMs >= scanExpiresAt;
    const scanIncomplete = cleanupCache !== null && (!cleanupCache.complete || cleanupCache.cancelled);
    useEffect(() => {
        if (scanExpiresAt === null)
            return;
        setNowMs(Date.now());
        const timer = window.setInterval(() => setNowMs(Date.now()), 15000);
        return () => window.clearInterval(timer);
    }, [scanExpiresAt]);
    useEffect(() => disposeAsyncListeners([
        listen<CleanupProgressEvent>("cleanup:progress", (event) => setCleanProgress(event.payload)),
        listen<CleanupScanProgressEvent>("cleanup:scan-progress", (event) => {
            setScanProgress(event.payload);
            if (event.payload.done) setCancelRequested(false);
        }),
    ], (error) => console.error("注册或释放清理事件失败", error)), []);
    const safeCategories = categories.filter((category) => category.riskLevel === "safe");
    const safeSelectionStates = safeCategories.map((category) => cleanupCategorySelection(category, selected, excludedPaths));
    const allSafeSelected = safeSelectionStates.length > 0 && safeSelectionStates.every((state) => state.checked);
    const partiallySafeSelected = safeSelectionStates.some((state) => state.checked || state.indeterminate) && !allSafeSelected;
    const selectedAdvanced = categories.some((category) => category.riskLevel === "advanced" && selected.has(category.id));
    const selectedCaution = categories.some((category) => category.riskLevel === "caution" && selected.has(category.id));
    const totalSelected = categories.reduce((sum, category) => sum + sumPathSize(cleanablePaths(category, selected, excludedPaths)), 0);
    const selectedFiles = categories.reduce((sum, category) => sum + sumPathFiles(cleanablePaths(category, selected, excludedPaths)), 0);
    const selectedPathCount = categories.reduce((sum, category) => sum + cleanablePaths(category, selected, excludedPaths).length, 0);
    const totalPathCount = categories.reduce((sum, category) => sum + category.paths.length, 0);
    const canClean = selectedPathCount > 0 && !!scanId && !scanExpired && !scanIncomplete && !busy;
    const canExport = !!cleanupCache?.scanId && selectedPathCount > 0 && !scanning && !cleaning && !exporting;
    const scanButton = scanButtonState({ scanning, cancelRequested, cleaning, exporting });
    const saveSelected = (next: Set<string>) => writeStoredStringList(SELECTED_STORAGE_KEY, next);
    const saveExcludedPaths = (next: Set<string>) => writeStoredStringList(EXCLUDED_PATHS_STORAGE_KEY, next);
    const invalidateScan = () => {
        cleanupCache = null;
        setCategories([]);
        setScanId(null);
        setScanExpiresAt(null);
        setScanProgress(null);
        setDetailCat(null);
    };
    const persistProjectRoots = (next: string[], notice?: string) => {
        const normalized = normalizeProjectRoots(next);
        writeStoredStringList(PROJECT_ROOTS_STORAGE_KEY, normalized);
        setProjectRoots(normalized);
        if (cleanupCache || categories.length > 0) {
            invalidateScan();
            void message.info("项目目录已更新，原扫描快照已失效，请重新扫描");
        }
        else if (notice) {
            void message.success(notice);
        }
    };
    const applyScanResult = (result: ScanResult) => {
        const nextExcludedPaths = validExcludedPaths(result.categories, excludedPaths);
        cleanupCache = result;
        setCategories(result.categories);
        setScanId(result.complete && !result.cancelled ? result.scanId : null);
        setScanExpiresAt(result.expiresAtMs);
        setNowMs(Date.now());
        const restored = restoreSelected(result.categories);
        saveSelected(restored);
        saveExcludedPaths(nextExcludedPaths);
        setSelected(restored);
        setExcludedPaths(nextExcludedPaths);
    };
    const commitSelection = (nextSelected: Set<string>, nextExcludedPaths: Set<string>) => {
        saveSelected(nextSelected);
        saveExcludedPaths(nextExcludedPaths);
        setSelected(nextSelected);
        setExcludedPaths(nextExcludedPaths);
    };
    const setCategoriesChecked = (targetCategories: CleanupCategory[], checked: boolean) => {
        const next = setCategoriesCheckedState(targetCategories, selected, excludedPaths, checked);
        commitSelection(next.selected, next.excludedPaths);
    };
    const toggleCategory = (category: DisplayCategory | CleanupCategory) => {
        const display = category as DisplayCategory;
        const state = displayCategorySelection(display, selected, excludedPaths);
        setCategoriesChecked(display.childCategories ?? [display], !state.checked);
    };
    const togglePath = (category: CleanupCategory, path: PathDetail) => {
        const next = togglePathState(category, path, selected, excludedPaths);
        commitSelection(next.selected, next.excludedPaths);
    };
    const onStartScan = async () => {
        if (busy)
            return;
        invalidateScan();
        setScanning(true);
        setCancelRequested(false);
        setCleanProgress(null);
        setScanProgress({
            stage: "queen",
            phase: "准备扫描",
            scannedPaths: 0,
            skippedPaths: 0,
            ignoredPaths: 0,
            scoutTasks: 0,
            engineerTasks: 0,
            currentPath: null,
            done: false,
            cancelled: false,
        });
        try {
            const result = await commands.scanCleanup({ projectRoots });
            applyScanResult(result);
            if (result.cancelled) {
                void message.warning({ content: "扫描已停止，当前结果仅供审核，不能用于清理", key: "cleanup-scan", duration: 4 });
            }
            else if (!result.complete) {
                void message.warning({ content: "扫描因时间或数量预算被截断，当前结果仅供审核", key: "cleanup-scan", duration: 5 });
            }
            else {
                void message.success({ content: `扫描完成，发现 ${fmtBytes(result.totalSizeBytes)}`, key: "cleanup-scan", duration: 2 });
            }
        }
        catch (error: unknown) {
            void message.error(error instanceof Error ? error.message : String(error));
        }
        finally {
            setScanning(false);
            setCancelRequested(false);
        }
    };
    const onStopScan = async () => {
        if (!scanning || cancelRequested)
            return;
        setCancelRequested(true);
        const accepted = await commands.cancelCleanupScan();
        if (!accepted) {
            setCancelRequested(false);
            return;
        }
        setScanProgress((current) => current ? { ...current, phase: "正在停止扫描…", cancelled: true } : current);
    };
    const onScanButton = () => {
        if (scanButton.action === "stop")
            void onStopScan();
        else if (scanButton.action === "start")
            void onStartScan();
    };
    const onClean = async () => {
        if (!canClean || !scanId)
            return;
        setScanId(null);
        setCleaning(true);
        setCleanProgress({
            percent: 0,
            processedItems: 0,
            totalItems: 0,
            currentCategory: "战争蚁正在校验扫描快照",
            currentPath: null,
            freedBytes: 0,
            deletedFiles: 0,
            done: false,
        });
        try {
            const result = await commands.cleanCategories({
                scanId,
                categoryIds: [...selected],
                excludedPaths: categories
                    .flatMap((category) => category.paths)
                    .filter((path) => excludedPaths.has(normalizePathKey(path.path)))
                    .map((path) => path.path),
                confirmCaution: selectedCaution,
                confirmAdvanced: selectedAdvanced,
            });
            if (result.errors.length === 0) {
                void message.success(`已释放 ${fmtBytes(result.freedBytes)}，删除 ${compactNumber(result.deletedFiles)} 个文件`);
            }
            else if (result.deletedFiles > 0) {
                void message.warning(`部分完成：已释放 ${fmtBytes(result.freedBytes)}，${result.errors.length} 项失败`);
            }
            else {
                void message.error(`清理未完成：${result.errors[0] ?? "未知错误"}`);
            }
            if (result.errors.length > 0) {
                modal.warning({
                    title: "部分项目未能清理",
                    width: 680,
                    content: (<div style={{ maxHeight: 320, overflow: "auto", marginTop: 12 }}>
              {result.errors.map((error, index) => (<div key={`${index}-${error}`} style={{ marginBottom: 8, wordBreak: "break-all" }}>{error}</div>))}
            </div>),
                });
            }
            const fresh = await commands.scanCleanup({ projectRoots });
            applyScanResult(fresh);
            setCleanProgress(null);
        }
        catch (error: unknown) {
            void message.error(error instanceof Error ? error.message : String(error));
        }
        finally {
            setCleaning(false);
        }
    };
    const onExport = async () => {
        if (!canExport || !cleanupCache)
            return;
        if (selectedPathCount === 0) {
            void message.info("请先勾选需要导出的清理分组或具体路径");
            return;
        }
        const selectedCategoryIds = categories
            .filter((category) => selected.has(category.id)
            && category.paths.some((item) => !excludedPaths.has(normalizePathKey(item.path))))
            .map((category) => category.id);
        const path = await save({
            title: "导出磁盘清理审核清单",
            defaultPath: `syspulse-cleanup-${new Date().toISOString().slice(0, 10)}.json`,
            filters: [{ name: "JSON", extensions: ["json"] }],
        });
        if (!path)
            return;
        setExporting(true);
        try {
            const result = await commands.exportCleanupScan({
                scanId: cleanupCache.scanId,
                path,
                selectedCategoryIds,
                excludedPaths: [...excludedPaths],
            });
            void message.success(`已导出 ${compactNumber(result.records)} 条路径 → ${result.savedTo}`);
        }
        catch (error: unknown) {
            void message.error(error instanceof Error ? error.message : String(error));
        }
        finally {
            setExporting(false);
        }
    };
    const chooseProjectRoot = async (): Promise<string | null> => {
        const chosen = await open({ directory: true, multiple: false, title: "选择项目工作区" });
        return typeof chosen === "string" ? chosen : null;
    };
    const onAddProjectRoot = async () => {
        if (projectRoots.length >= MAX_PROJECT_ROOTS) {
            void message.warning(`最多添加 ${MAX_PROJECT_ROOTS} 个项目目录`);
            return;
        }
        const chosen = await chooseProjectRoot();
        if (!chosen)
            return;
        const next = addProjectRoot(projectRoots, chosen);
        if (next.length === projectRoots.length) {
            void message.info("该目录已在扫描范围内");
            return;
        }
        persistProjectRoots(next, "已加入项目扫描范围");
    };
    const onReplaceProjectRoot = async (index: number) => {
        const chosen = await chooseProjectRoot();
        if (!chosen)
            return;
        persistProjectRoots(replaceProjectRoot(projectRoots, index, chosen), "项目目录已替换");
    };
    const openDetail = (category: DisplayCategory) => {
        setDetailCat(category);
        setActiveToolchainId(category.childCategories?.find((child) => child.paths.length > 0)?.id ?? category.id);
    };
    const closeDetail = () => {
        setDetailCat(null);
    };
    const categoryRows = sortedDisplayCategories.map((category) => {
        const selection = displayCategorySelection(category, selected, excludedPaths);
        const activePaths = category.childCategories
            ? category.childCategories.flatMap((child) => cleanablePaths(child, selected, excludedPaths))
            : cleanablePaths(category, selected, excludedPaths);
        return {
            category,
            selection,
            selectedSize: sumPathSize(activePaths),
            selectedFiles: sumPathFiles(activePaths),
        };
    });
    const categoryColumns: ColumnsType<(typeof categoryRows)[number]> = [
        {
            title: (<div className="cleanup-table-column-title">
          <strong>清理项目</strong>
          <span>缓存类别与风险</span>
        </div>),
            key: "category",
            render: (_, row) => {
                const meta = riskMeta(row.category.riskLevel);
                return (<div className="cleanup-category-cell">
            <Checkbox checked={row.selection.checked} indeterminate={row.selection.indeterminate} disabled={busy} onClick={(event) => event.stopPropagation()} onChange={() => toggleCategory(row.category)}/>
            <span className="cleanup-category-icon">
              {row.category.id === PROGRAMMING_CATEGORY_ID ? <CodeOutlined /> : categoryIcon(row.category.id)}
            </span>
            <div className="cleanup-category-copy">
              <Space size={6} wrap>
                <Text strong>{row.category.name}</Text>
                <span className="cleanup-risk-pill" style={{ color: meta.color, background: meta.background }}>{meta.label}</span>
                {row.category.minAgeDays != null && <Tag color="green">仅 {row.category.minAgeDays} 天以上</Tag>}
              </Space>
              <Text type="secondary" ellipsis title={row.category.description}>{row.category.description}</Text>
            </div>
          </div>);
            },
        },
        {
            title: (<div className="cleanup-table-column-title is-right">
          <strong>路径</strong>
          <span>已选 / 总数</span>
        </div>),
            dataIndex: ["selection", "totalPathCount"],
            width: 90,
            align: "right",
            render: (_, row) => `${row.selection.checkedPathCount}/${row.selection.totalPathCount}`,
        },
        {
            title: (<div className="cleanup-table-column-title is-right">
          <strong>占用</strong>
          <span>发现总量</span>
        </div>),
            dataIndex: ["category", "sizeBytes"],
            width: 120,
            align: "right",
            render: (_, row) => <Text strong style={{ color: "#cf1322" }}>{fmtBytes(row.category.sizeBytes)}</Text>,
        },
        {
            title: (<div className="cleanup-table-column-title is-right">
          <strong>清理队列</strong>
          <span>容量与文件</span>
        </div>),
            key: "selected",
            width: 150,
            align: "right",
            render: (_, row) => (<div>
          <Text>{fmtBytes(row.selectedSize)}</Text>
          <Text type="secondary" style={{ display: "block", fontSize: 11 }}>{compactNumber(row.selectedFiles)} 文件</Text>
        </div>),
        },
    ];
    const detailToolchains = detailCat?.childCategories?.filter((category) => category.paths.length > 0) ?? [];
    const activeDetailCategory = detailToolchains.find((category) => category.id === activeToolchainId)
        ?? detailToolchains[0]
        ?? detailCat;
    const activeDetailSelection = activeDetailCategory
        ? cleanupCategorySelection(activeDetailCategory, selected, excludedPaths)
        : null;
    const visibleDetailPaths = activeDetailCategory?.paths ?? [];
    const detailCategories = detailCat?.childCategories ?? (detailCat ? [detailCat] : []);
    const detailSelectedPaths = detailCategories.flatMap((category) => cleanablePaths(category, selected, excludedPaths));
    const detailSelectedSize = sumPathSize(detailSelectedPaths);
    const detailSelectedFiles = sumPathFiles(detailSelectedPaths);
    const detailAllSelection = detailCat
        ? displayCategorySelection(detailCat, selected, excludedPaths)
        : null;
    const discoveredToolchains = detailToolchains.length;
    const progressMeta = stageMeta(scanProgress?.stage ?? "queen");
    const heroScanState = scanning
        ? `${progressMeta.label} · ${scanProgress?.phase ?? "正在建立扫描任务"}`
        : cleanupCache
            ? `${relativeScanTime(cleanupCache.scannedAtMs, nowMs)} · ${cleanupCache.complete && !cleanupCache.cancelled ? "结果完整" : "仅供审核"}`
            : "尚未扫描 · 当前处于只读审核模式";
    const heroScanDetail = scanning
        ? scanProgress?.currentPath ?? "正在分派侦察任务"
        : cleanupCache
            ? `检查 ${compactNumber(cleanupCache.scannedPaths)} 项 · 热点 ${compactNumber(cleanupCache.hotspotCount)} · 侦察 ${cleanupCache.scoutWorkers} · 工兵 ${cleanupCache.engineerWorkers}`
            : "默认只选择超过保留期的安全项；清理由你最后确认。";
    const heroScoutTasks = scanProgress?.scoutTasks ?? cleanupCache?.scoutTasks ?? 0;
    const heroEngineerTasks = scanProgress?.engineerTasks ?? cleanupCache?.engineerTasks ?? 0;
    return { Text, activeDetailCategory, activeDetailSelection, allSafeSelected, busy, canClean, canExport, categories, categoryColumns, categoryIcon, categoryRows, cleanProgress, cleaning, cleanupCache, closeDetail, compactNumber, detailAllSelection, detailCat, detailCategories, detailSelectedFiles, detailSelectedPaths, detailSelectedSize, detailToolchains, discoveredToolchains, excludedPaths, exporting, heroEngineerTasks, heroScanDetail, heroScanState, heroScoutTasks, onAddProjectRoot, onClean, onExport, onReplaceProjectRoot, onScanButton, openDetail, partiallySafeSelected, persistProjectRoots, progressMeta, projectManagerOpen, projectRoots, riskMeta, safeCategories, scanButton, scanExpired, scanIncomplete, scanProgress, scanning, selected, selectedAdvanced, selectedCaution, selectedFiles, selectedPathCount, setActiveToolchainId, setCategoriesChecked, setProjectManagerOpen, sourceLabel, toggleCategory, togglePath, totalPathCount, totalSelected, visibleDetailPaths };
}
export type CleanupModel = ReturnType<typeof useCleanupModel>;
