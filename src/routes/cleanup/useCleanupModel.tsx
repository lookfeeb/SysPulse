import { useCleanupStore } from "@/stores/cleanupStore";
import type { CleanupCategory, PathDetail } from "@/bindings";
import type { DisplayCategory } from "@/routes/cleanupSelection";
import { MAX_PROJECT_ROOTS, PROGRAMMING_CATEGORY_ID, addProjectRoot, buildDisplayCategories, cleanablePaths, cleanupCategorySelection, displayCategorySelection, normalizePathKey, normalizeProjectRoots, replaceProjectRoot, restoreSelectedCategories, scanButtonState, setCategoriesCheckedState, sumPathFiles, sumPathSize, togglePathState, validExcludedPaths } from "@/routes/cleanupSelection";
import { fmtBytes } from "@/utils/format";
import { readStoredStringList, writeStoredStringList } from "@/utils/storageList";
import { CodeOutlined } from "@ant-design/icons";
import { open } from "@tauri-apps/plugin-dialog";
import { App as AntdApp, Checkbox, Space, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useMemo, useState } from "react";
const { Text } = Typography;
const SELECTED_STORAGE_KEY = "syspulse.cleanup.selectedCategories.v4";
const EXCLUDED_PATHS_STORAGE_KEY = "syspulse.cleanup.excludedPaths.v1";
const PROJECT_ROOTS_STORAGE_KEY = "syspulse.cleanup.projectRoots.v1";
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
        return { label: "发现目录", color: "blue" };
    if (stage === "engineer")
        return { label: "检查缓存", color: "cyan" };
    if (stage === "done")
        return { label: "已完成", color: "green" };
    return { label: "准备扫描", color: "purple" };
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
    const { message } = AntdApp.useApp();
    const { scan: cleanupCache, phase, consumed, cancelRequested, scanProgress, cleanProgress, lastCleanResult } = useCleanupStore();
    const scanning = phase === "scanning" || phase === "refreshing";
    const cleaning = phase === "cleaning";
    const categories = cleanupCache?.categories ?? [];
    const [selected, setSelected] = useState<Set<string>>(() => restoreSelected(cleanupCache?.categories ?? []));
    const [excludedPaths, setExcludedPaths] = useState<Set<string>>(() => restoreExcludedPaths());
    const [projectRoots, setProjectRoots] = useState<string[]>(() => normalizeProjectRoots(readStoredStringList(PROJECT_ROOTS_STORAGE_KEY) ?? []));
    const [detailCat, setDetailCat] = useState<DisplayCategory | null>(null);
    const [activeToolchainId, setActiveToolchainId] = useState<string | null>(null);
    const [projectManagerOpen, setProjectManagerOpen] = useState(false);
    const scanId = !consumed ? cleanupCache?.scanId ?? null : null;
    const [nowMs, setNowMs] = useState(Date.now());
    const displayCategories = useMemo(() => buildDisplayCategories(categories), [categories]);
    const sortedDisplayCategories = useMemo(() => [...displayCategories].sort((left, right) => {
        const order = { safe: 0, caution: 1, advanced: 2 } as const;
        return order[left.riskLevel] - order[right.riskLevel];
    }), [displayCategories]);
    const busy = scanning || cleaning;
    const scanIncomplete = cleanupCache !== null && (!cleanupCache.complete || cleanupCache.cancelled);
    useEffect(() => {
        if (!cleanupCache)
            return;
        setNowMs(Date.now());
        const timer = window.setInterval(() => setNowMs(Date.now()), 15000);
        return () => window.clearInterval(timer);
    }, [cleanupCache]);
    useEffect(() => {
        if (!cleanupCache) return;
        const restored = restoreSelected(cleanupCache.categories);
        const excluded = validExcludedPaths(cleanupCache.categories, restoreExcludedPaths());
        setSelected(restored);
        setExcludedPaths(excluded);
        writeStoredStringList(SELECTED_STORAGE_KEY, restored);
        writeStoredStringList(EXCLUDED_PATHS_STORAGE_KEY, excluded);
        setNowMs(Date.now());
    }, [cleanupCache]);
    const selectedAdvanced = categories.some((category) => category.riskLevel === "advanced" && selected.has(category.id));
    const selectedCaution = categories.some((category) => category.riskLevel === "caution" && selected.has(category.id));
    const totalSelected = categories.reduce((sum, category) => sum + sumPathSize(cleanablePaths(category, selected, excludedPaths)), 0);
    const selectedFiles = categories.reduce((sum, category) => sum + sumPathFiles(cleanablePaths(category, selected, excludedPaths)), 0);
    const selectedPathCount = categories.reduce((sum, category) => sum + cleanablePaths(category, selected, excludedPaths).length, 0);
    const totalPathCount = categories.reduce((sum, category) => sum + category.paths.length, 0);
    const canClean = selectedPathCount > 0 && !!scanId && !scanIncomplete && !busy;
    const scanButton = scanButtonState({ scanning, cancelRequested, cleaning, exporting: false });
    const saveSelected = (next: Set<string>) => writeStoredStringList(SELECTED_STORAGE_KEY, next);
    const saveExcludedPaths = (next: Set<string>) => writeStoredStringList(EXCLUDED_PATHS_STORAGE_KEY, next);
    const invalidateScan = () => {
        useCleanupStore.getState().invalidate();
        setDetailCat(null);
    };
    const persistProjectRoots = (next: string[], notice?: string) => {
        if (useCleanupStore.getState().phase !== "idle") return;
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
    const onScanButton = () => {
        if (scanButton.action === "stop") void useCleanupStore.getState().cancelScan();
        else if (scanButton.action === "start") {
            setDetailCat(null);
            void useCleanupStore.getState().startScan(projectRoots);
        }
    };
    const onClean = async () => {
        if (!canClean || !scanId) return;
        await useCleanupStore.getState().clean({
            scanId,
            categoryIds: [...selected],
            excludedPaths: categories.flatMap((category) => category.paths)
                .filter((path) => excludedPaths.has(normalizePathKey(path.path)))
                .map((path) => path.path),
            confirmCaution: selectedCaution,
            confirmAdvanced: selectedAdvanced,
        }, projectRoots);
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
        </div>),
            key: "category",
            render: (_, row) => {
                const meta = riskMeta(row.category.riskLevel);
                return (<div className="cleanup-category-cell">
            <Checkbox checked={row.selection.checked} indeterminate={row.selection.indeterminate} aria-label={`选择${row.category.name}`} disabled={busy} onClick={(event) => event.stopPropagation()} onChange={() => toggleCategory(row.category)}/>
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
        </div>),
            dataIndex: ["selection", "totalPathCount"],
            width: 68,
            align: "right",
            render: (_, row) => `${row.selection.checkedPathCount}/${row.selection.totalPathCount}`,
        },
        {
            title: (<div className="cleanup-table-column-title is-right">
          <strong>占用空间</strong>
        </div>),
            dataIndex: ["category", "sizeBytes"],
            width: 102,
            align: "right",
            render: (_, row) => <Text strong className="cleanup-size-total">{fmtBytes(row.category.sizeBytes)}</Text>,
        },
        {
            title: (<div className="cleanup-table-column-title is-right">
          <strong>已选空间</strong>
        </div>),
            key: "selected",
            width: 116,
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
    const heroScanState = cleaning
        ? "正在后台清理，切换页面不会中断"
        : scanning
        ? `${progressMeta.label} · ${scanProgress?.phase ?? "正在建立扫描任务"}`
        : cleanupCache
            ? `${relativeScanTime(cleanupCache.scannedAtMs, nowMs)} · ${cleanupCache.complete && !cleanupCache.cancelled ? "结果完整" : "仅供审核"}`
            : "尚未扫描 · 当前处于只读审核模式";
    const heroScanDetail = cleaning
        ? cleanProgress?.currentPath ?? "正在校验清理路径"
        : scanning
        ? scanProgress?.currentPath ?? "正在分派侦察任务"
        : cleanupCache
            ? `检查 ${compactNumber(cleanupCache.scannedPaths)} 项 · 热点 ${compactNumber(cleanupCache.hotspotCount)} · 侦察 ${cleanupCache.scoutWorkers} · 工兵 ${cleanupCache.engineerWorkers}`
            : "默认只选择超过保留期的安全项；清理由你最后确认。";
    const heroScoutTasks = scanning ? scanProgress?.scoutTasks ?? 0 : cleanupCache?.scoutTasks ?? 0;
    const heroEngineerTasks = scanning ? scanProgress?.engineerTasks ?? 0 : cleanupCache?.engineerTasks ?? 0;
    return { lastCleanResult, phase, Text, activeDetailCategory, activeDetailSelection, busy, canClean, categories, categoryColumns, categoryIcon, categoryRows, cleanProgress, cleaning, cleanupCache, closeDetail, compactNumber, detailAllSelection, detailCat, detailCategories, detailSelectedFiles, detailSelectedPaths, detailSelectedSize, detailToolchains, discoveredToolchains, excludedPaths, heroEngineerTasks, heroScanDetail, heroScanState, heroScoutTasks, onAddProjectRoot, onClean, onReplaceProjectRoot, onScanButton, openDetail, persistProjectRoots, progressMeta, projectManagerOpen, projectRoots, riskMeta, scanButton, scanIncomplete, scanProgress, scanning, selected, selectedAdvanced, selectedCaution, selectedFiles, selectedPathCount, setActiveToolchainId, setCategoriesChecked, setProjectManagerOpen, sourceLabel, toggleCategory, togglePath, totalPathCount, totalSelected, visibleDetailPaths };
}
export type CleanupModel = ReturnType<typeof useCleanupModel>;
