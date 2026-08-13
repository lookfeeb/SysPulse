import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  App as AntdApp,
  Button,
  Card,
  Checkbox,
  Empty,
  Modal,
  Popconfirm,
  Progress,
  Space,
  Spin,
  Table,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import {
  CheckCircleOutlined,
  CloseOutlined,
  CodeOutlined,
  DeleteOutlined,
  EditOutlined,
  ExportOutlined,
  FolderAddOutlined,
  FolderOpenOutlined,
  RadarChartOutlined,
  ScanOutlined,
  StopOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { commands } from "@/bindings";
import type { CleanupCategory, PathDetail, ScanResult } from "@/bindings";
import { fmtBytes } from "@/utils/format";
import { readStoredStringList, writeStoredStringList } from "@/utils/storageList";
import {
  MAX_PROJECT_ROOTS,
  PROGRAMMING_CATEGORY_ID,
  addProjectRoot,
  buildDisplayCategories,
  cleanablePaths,
  cleanupCategorySelection,
  displayCategorySelection,
  normalizePathKey,
  normalizeProjectRoots,
  removeProjectRoot,
  replaceProjectRoot,
  restoreSelectedCategories,
  scanButtonState,
  setCategoriesCheckedState,
  sumPathFiles,
  sumPathSize,
  togglePathState,
  validExcludedPaths,
} from "@/routes/cleanupSelection";
import type { DisplayCategory } from "@/routes/cleanupSelection";

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
  if (risk === "safe") return { label: "安全", color: "#16845b", background: "#e7f8f0" };
  if (risk === "caution") return { label: "谨慎", color: "#a96b00", background: "#fff3d8" };
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

function stageMeta(stage: string): { label: string; color: string } {
  if (stage === "scout") return { label: "侦察蚁", color: "blue" };
  if (stage === "engineer") return { label: "工兵蚁", color: "cyan" };
  if (stage === "done") return { label: "已完成", color: "green" };
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
  const elapsedMinutes = Math.floor(elapsedMs / 60_000);
  if (elapsedMinutes < 1) return "刚刚";
  if (elapsedMinutes < 60) return `${elapsedMinutes} 分钟前`;
  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return `${elapsedHours} 小时前`;
  return new Date(scannedAtMs).toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}

export default function CleanupPage() {
  const { message, modal } = AntdApp.useApp();
  const initialCache = cleanupCache && cleanupCache.expiresAtMs > Date.now() ? cleanupCache : null;
  const [categories, setCategories] = useState<CleanupCategory[]>(initialCache?.categories ?? []);
  const [selected, setSelected] = useState<Set<string>>(() => restoreSelected(initialCache?.categories ?? []));
  const [excludedPaths, setExcludedPaths] = useState<Set<string>>(() => restoreExcludedPaths());
  const [projectRoots, setProjectRoots] = useState<string[]>(() =>
    normalizeProjectRoots(readStoredStringList(PROJECT_ROOTS_STORAGE_KEY) ?? []),
  );
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
  const sortedDisplayCategories = useMemo(
    () => [...displayCategories].sort((left, right) => {
      const order = { safe: 0, caution: 1, advanced: 2 } as const;
      return order[left.riskLevel] - order[right.riskLevel];
    }),
    [displayCategories],
  );
  const busy = scanning || cleaning || exporting;
  const scanExpired = scanExpiresAt !== null && nowMs >= scanExpiresAt;
  const scanIncomplete = cleanupCache !== null && (!cleanupCache.complete || cleanupCache.cancelled);

  useEffect(() => {
    if (scanExpiresAt === null) return;
    setNowMs(Date.now());
    const timer = window.setInterval(() => setNowMs(Date.now()), 15_000);
    return () => window.clearInterval(timer);
  }, [scanExpiresAt]);

  useEffect(() => {
    let unlistenCleanup: (() => void) | null = null;
    let unlistenScan: (() => void) | null = null;
    void listen<CleanupProgressEvent>("cleanup:progress", (event) => setCleanProgress(event.payload)).then((fn) => {
      unlistenCleanup = fn;
    });
    void listen<CleanupScanProgressEvent>("cleanup:scan-progress", (event) => {
      setScanProgress(event.payload);
      if (event.payload.done) setCancelRequested(false);
    }).then((fn) => {
      unlistenScan = fn;
    });
    return () => {
      unlistenCleanup?.();
      unlistenScan?.();
    };
  }, []);

  const safeCategories = categories.filter((category) => category.riskLevel === "safe");
  const safeSelectionStates = safeCategories.map((category) => cleanupCategorySelection(category, selected, excludedPaths));
  const allSafeSelected = safeSelectionStates.length > 0 && safeSelectionStates.every((state) => state.checked);
  const partiallySafeSelected = safeSelectionStates.some((state) => state.checked || state.indeterminate) && !allSafeSelected;
  const selectedAdvanced = categories.some((category) => category.riskLevel === "advanced" && selected.has(category.id));
  const selectedCaution = categories.some((category) => category.riskLevel === "caution" && selected.has(category.id));
  const totalSelected = categories.reduce(
    (sum, category) => sum + sumPathSize(cleanablePaths(category, selected, excludedPaths)),
    0,
  );
  const selectedFiles = categories.reduce(
    (sum, category) => sum + sumPathFiles(cleanablePaths(category, selected, excludedPaths)),
    0,
  );
  const selectedPathCount = categories.reduce(
    (sum, category) => sum + cleanablePaths(category, selected, excludedPaths).length,
    0,
  );
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
    } else if (notice) {
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
    if (busy) return;
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
      } else if (!result.complete) {
        void message.warning({ content: "扫描因时间或数量预算被截断，当前结果仅供审核", key: "cleanup-scan", duration: 5 });
      } else {
        void message.success({ content: `扫描完成，发现 ${fmtBytes(result.totalSizeBytes)}`, key: "cleanup-scan", duration: 2 });
      }
    } catch (error: unknown) {
      void message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setScanning(false);
      setCancelRequested(false);
    }
  };

  const onStopScan = async () => {
    if (!scanning || cancelRequested) return;
    setCancelRequested(true);
    const accepted = await commands.cancelCleanupScan();
    if (!accepted) {
      setCancelRequested(false);
      return;
    }
    setScanProgress((current) => current ? { ...current, phase: "正在停止扫描…", cancelled: true } : current);
  };

  const onScanButton = () => {
    if (scanButton.action === "stop") void onStopScan();
    else if (scanButton.action === "start") void onStartScan();
  };

  const onClean = async () => {
    if (!canClean || !scanId) return;
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
      } else if (result.deletedFiles > 0) {
        void message.warning(`部分完成：已释放 ${fmtBytes(result.freedBytes)}，${result.errors.length} 项失败`);
      } else {
        void message.error(`清理未完成：${result.errors[0] ?? "未知错误"}`);
      }
      if (result.errors.length > 0) {
        modal.warning({
          title: "部分项目未能清理",
          width: 680,
          content: (
            <div style={{ maxHeight: 320, overflow: "auto", marginTop: 12 }}>
              {result.errors.map((error, index) => (
                <div key={`${index}-${error}`} style={{ marginBottom: 8, wordBreak: "break-all" }}>{error}</div>
              ))}
            </div>
          ),
        });
      }
      const fresh = await commands.scanCleanup({ projectRoots });
      applyScanResult(fresh);
      setCleanProgress(null);
    } catch (error: unknown) {
      void message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setCleaning(false);
    }
  };

  const onExport = async () => {
    if (!canExport || !cleanupCache) return;
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
    if (!path) return;
    setExporting(true);
    try {
      const result = await commands.exportCleanupScan({
        scanId: cleanupCache.scanId,
        path,
        selectedCategoryIds,
        excludedPaths: [...excludedPaths],
      });
      void message.success(`已导出 ${compactNumber(result.records)} 条路径 → ${result.savedTo}`);
    } catch (error: unknown) {
      void message.error(error instanceof Error ? error.message : String(error));
    } finally {
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
    if (!chosen) return;
    const next = addProjectRoot(projectRoots, chosen);
    if (next.length === projectRoots.length) {
      void message.info("该目录已在扫描范围内");
      return;
    }
    persistProjectRoots(next, "已加入项目扫描范围");
  };

  const onReplaceProjectRoot = async (index: number) => {
    const chosen = await chooseProjectRoot();
    if (!chosen) return;
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
      title: (
        <div className="cleanup-table-column-title">
          <strong>清理项目</strong>
          <span>缓存类别与风险</span>
        </div>
      ),
      key: "category",
      render: (_, row) => {
        const meta = riskMeta(row.category.riskLevel);
        return (
          <div className="cleanup-category-cell">
            <Checkbox
              checked={row.selection.checked}
              indeterminate={row.selection.indeterminate}
              disabled={busy}
              onClick={(event) => event.stopPropagation()}
              onChange={() => toggleCategory(row.category)}
            />
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
          </div>
        );
      },
    },
    {
      title: (
        <div className="cleanup-table-column-title is-right">
          <strong>路径</strong>
          <span>已选 / 总数</span>
        </div>
      ),
      dataIndex: ["selection", "totalPathCount"],
      width: 90,
      align: "right",
      render: (_, row) => `${row.selection.checkedPathCount}/${row.selection.totalPathCount}`,
    },
    {
      title: (
        <div className="cleanup-table-column-title is-right">
          <strong>占用</strong>
          <span>发现总量</span>
        </div>
      ),
      dataIndex: ["category", "sizeBytes"],
      width: 120,
      align: "right",
      render: (_, row) => <Text strong style={{ color: "#cf1322" }}>{fmtBytes(row.category.sizeBytes)}</Text>,
    },
    {
      title: (
        <div className="cleanup-table-column-title is-right">
          <strong>清理队列</strong>
          <span>容量与文件</span>
        </div>
      ),
      key: "selected",
      width: 150,
      align: "right",
      render: (_, row) => (
        <div>
          <Text>{fmtBytes(row.selectedSize)}</Text>
          <Text type="secondary" style={{ display: "block", fontSize: 11 }}>{compactNumber(row.selectedFiles)} 文件</Text>
        </div>
      ),
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
  const detailSelectedPaths = detailCategories.flatMap((category) =>
    cleanablePaths(category, selected, excludedPaths),
  );
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

  return (
    <Space vertical size={16} style={{ width: "100%" }}>
      {(scanExpired || scanIncomplete) && categories.length > 0 && (
        <Alert
          type="warning"
          showIcon
          title={scanExpired ? "扫描结果已过期" : "扫描结果不完整"}
          description="当前结果可导出审核，但不能直接清理；请重新完成扫描。"
          style={{ borderRadius: 8 }}
        />
      )}

      <section className="cleanup-hero">
        <div className="cleanup-hero-copy">
          <span className="cleanup-hero-eyebrow">DISK HYGIENE · READ ONLY</span>
          <h1>先看清楚，再释放空间</h1>
          <p>深度检查用户目录、固定磁盘和开发工具热点，只展示已验证、可解释的清理路径。扫描结果默认保持只读，清理由你最后确认。</p>

          <div className={`cleanup-hero-status${scanning ? " is-scanning" : ""}`}>
            <div className="cleanup-hero-status-primary">
              <span className={`cleanup-hero-dot${scanning ? " is-scanning" : cleanupCache && (!cleanupCache.complete || cleanupCache.cancelled) ? " is-warning" : ""}`} />
              <div>
                <span>{scanning ? "扫描进行中" : cleanupCache ? "扫描快照" : "等待扫描"}</span>
                <strong>{heroScanState}</strong>
              </div>
              {scanning && <Spin size="small" />}
            </div>
            <div className="cleanup-hero-status-path" title={heroScanDetail}>{heroScanDetail}</div>
            <div className="cleanup-hero-status-counts">
              <span><b>{compactNumber(heroScoutTasks)}</b> 侦察任务</span>
              <span><b>{compactNumber(heroEngineerTasks)}</b> 工兵任务</span>
            </div>
          </div>

          <div className="cleanup-hero-metrics">
            <div><span>已选择</span><strong>{fmtBytes(totalSelected)}</strong><small>{compactNumber(selectedPathCount)} 条路径</small></div>
            <div><span>验证路径</span><strong>{compactNumber(totalPathCount)}</strong><small>{compactNumber(selectedFiles)} 个已选文件</small></div>
            <div><span>扫描用时</span><strong>{cleanupCache ? `${(cleanupCache.durationMs / 1000).toFixed(1)} s` : "—"}</strong><small>{cleanupCache ? `${compactNumber(cleanupCache.scannedPaths)} 项` : "等待扫描"}</small></div>
            <button
              type="button"
              className="cleanup-hero-metric-action"
              disabled={scanning || cleaning}
              onClick={() => setProjectManagerOpen(true)}
              title="管理项目扫描根；所有固定磁盘分区仍会自动进入快速侦察"
            >
              <span>扫描范围</span><strong>{projectRoots.length} / {MAX_PROJECT_ROOTS}</strong><small>管理扫描根 · 全固定盘</small>
            </button>
          </div>
        </div>

        <div className="cleanup-hero-visual">
          <div className={`cleanup-hero-gauge${scanning ? " is-scanning" : ""}`}>
            <div className="cleanup-hero-gauge-inner">
              <strong>{fmtBytes(cleanupCache?.totalSizeBytes ?? 0)}</strong>
              <span>可释放空间</span>
            </div>
          </div>
          <Tooltip title={scanButton.label}>
            <Button
              className="cleanup-hero-scan"
              type="primary"
              danger={scanButton.danger}
              shape="circle"
              icon={scanning ? <StopOutlined /> : <ScanOutlined />}
              disabled={scanButton.disabled}
              onClick={onScanButton}
              aria-label={scanButton.label}
            />
          </Tooltip>
          <span className="cleanup-hero-scan-label">{scanButton.label}</span>
        </div>
      </section>

      {cleanProgress && (
        <Card size="small" style={{ borderRadius: 10 }} styles={{ body: { padding: "12px 16px" } }}>
          <div className="cleanup-clean-status">
            <div style={{ minWidth: 0 }}>
              <Text strong>{cleanProgress.done ? "清理完成" : cleanProgress.currentCategory || "战争蚁正在清理"}</Text>
              {cleanProgress.currentPath && <Text type="secondary" ellipsis title={cleanProgress.currentPath} style={{ display: "block", fontSize: 11 }}>{cleanProgress.currentPath}</Text>}
            </div>
            <div style={{ textAlign: "right", whiteSpace: "nowrap" }}>
              <Text strong style={{ color: "#cf1322" }}>{fmtBytes(cleanProgress.freedBytes)}</Text>
              <Text type="secondary" style={{ display: "block", fontSize: 11 }}>{compactNumber(cleanProgress.deletedFiles)} 文件</Text>
            </div>
          </div>
          <Progress percent={cleanProgress.percent} status={cleanProgress.done ? "success" : "active"} size="small" style={{ marginTop: 8 }} />
        </Card>
      )}

      {(scanning || categories.length > 0) && (
        <Card
          className={`cleanup-results-card${scanning ? " is-scanning" : ""}`}
          size="small"
          title={
            <div className="cleanup-results-heading">
              <span className="cleanup-results-heading-icon"><RadarChartOutlined /></span>
              <div>
                <strong>{scanning ? "实时扫描结果" : "扫描结果"}</strong>
                <span>
                  {scanning
                    ? `已检查 ${compactNumber(scanProgress?.scannedPaths ?? 0)} 项 · 已派发 ${compactNumber(heroScoutTasks + heroEngineerTasks)} 个任务`
                    : `${categories.length} 个类别 · ${totalPathCount} 条已验证路径`}
                </span>
              </div>
            </div>
          }
          extra={(
            <div className="cleanup-results-toolbar">
              <div className="cleanup-results-toolbar-group cleanup-results-toolbar-scope">
                <Button
                  className="cleanup-results-scope-button"
                  size="small"
                  icon={<FolderAddOutlined />}
                  disabled={scanning || cleaning}
                  onClick={() => setProjectManagerOpen(true)}
                >
                  项目目录 <b>{projectRoots.length}/{MAX_PROJECT_ROOTS}</b>
                </Button>
                <Tooltip title={canExport ? "仅导出当前勾选的具体缓存路径" : "扫描并勾选路径后可导出 JSON 明细"}>
                  <Button
                    className="cleanup-results-export-button"
                    size="small"
                    icon={<ExportOutlined />}
                    loading={exporting}
                    disabled={!canExport}
                    onClick={() => void onExport()}
                  >
                    导出明细
                  </Button>
                </Tooltip>
              </div>
              {categories.length > 0 && (
                <div className="cleanup-results-toolbar-group cleanup-results-toolbar-selection">
                  <Checkbox
                    checked={allSafeSelected}
                    indeterminate={partiallySafeSelected}
                    disabled={busy || safeCategories.length === 0}
                    onChange={() => setCategoriesChecked(safeCategories, !allSafeSelected)}
                  >
                    全选安全项
                  </Checkbox>
                  <Popconfirm
                    title="确认清理选中路径"
                    description={selectedAdvanced
                      ? `包含高级维护项，将清理 ${fmtBytes(totalSelected)}；请先查看路径和命中规则。`
                      : selectedCaution
                        ? `包含可再生成缓存，将清理 ${fmtBytes(totalSelected)}；运行中的工具链会被跳过。`
                        : `将清理 ${fmtBytes(totalSelected)}，操作不可撤销。`}
                    onConfirm={() => void onClean()}
                    disabled={!canClean}
                  >
                    <Button type="primary" danger size="small" icon={<ThunderboltOutlined />} loading={cleaning} disabled={!canClean}>
                      清理 {fmtBytes(totalSelected)}
                    </Button>
                  </Popconfirm>
                </div>
              )}
            </div>
          )}
          style={{ borderRadius: 10 }}
          styles={{ body: { padding: categories.length > 0 ? 0 : 16 } }}
        >
          {categories.length > 0 ? (
          <Table
            className="cleanup-result-table"
            rowKey={(row) => row.category.id}
            columns={categoryColumns}
            dataSource={categoryRows}
            pagination={false}
            size="small"
            tableLayout="fixed"
            scroll={{ x: 720 }}
            onRow={(row) => ({ onClick: () => openDetail(row.category), style: { cursor: "pointer" } })}
          />
          ) : (
            <div className="cleanup-live-results">
              <div className="cleanup-live-results-stage">
                <span className="cleanup-live-results-pulse" />
                <div>
                  <strong>{progressMeta.label}</strong>
                  <span>{scanProgress?.phase ?? "正在建立扫描任务"}</span>
                </div>
                <b>{compactNumber(scanProgress?.scannedPaths ?? 0)}</b>
                <small>已检查项</small>
              </div>
              <div className="cleanup-live-results-path" title={scanProgress?.currentPath ?? undefined}>
                {scanProgress?.currentPath ?? "正在分派侦察任务，发现的缓存分类会逐步出现在这里"}
              </div>
              <div className="cleanup-live-results-tasks">
                <span><b>{compactNumber(heroScoutTasks)}</b><small>侦察任务</small></span>
                <span><b>{compactNumber(heroEngineerTasks)}</b><small>工兵任务</small></span>
                <span><b>{compactNumber(scanProgress?.skippedPaths ?? 0)}</b><small>跳过异常</small></span>
              </div>
            </div>
          )}
        </Card>
      )}

      <Modal
        title={null}
        closable={false}
        open={!!detailCat}
        onCancel={closeDetail}
        footer={null}
        width={1100}
        centered
        className="cleanup-review-modal"
        styles={{ container: { padding: 0, overflow: "hidden" }, body: { padding: 0 } }}
      >
        {detailCat && activeDetailCategory && (
          <div className="cleanup-review-shell">
            <header className="cleanup-review-head">
              <div className="cleanup-review-heading">
                <span className="cleanup-review-eyebrow">CACHE REVIEW · READ ONLY</span>
                <div className="cleanup-review-title-row">
                  <span className="cleanup-review-title-icon">
                    {detailCat.id === PROGRAMMING_CATEGORY_ID ? <CodeOutlined /> : categoryIcon(detailCat.id)}
                  </span>
                  <div>
                    <strong>{detailCat.name}</strong>
                    <span>
                      {detailCat.childCategories
                        ? "仅展示本次扫描实际发现缓存的工具链 · 保留可复用依赖目录"
                        : detailCat.description}
                    </span>
                  </div>
                </div>
              </div>
              <div className="cleanup-review-head-actions">
                <Button
                  className="cleanup-select-all-button"
                  size="small"
                  disabled={busy || detailCategories.every((category) => category.paths.length === 0)}
                  onClick={() => setCategoriesChecked(detailCategories, !detailAllSelection?.checked)}
                >
                  {detailAllSelection?.checked ? "清除全部勾选" : "勾选全部工具链"}
                </Button>
                <Button type="text" className="cleanup-review-close" icon={<CloseOutlined />} onClick={closeDetail} aria-label="关闭" />
              </div>
            </header>

            <div className="cleanup-review-overview">
              <div className="cleanup-review-stat cleanup-review-stat--primary">
                <span>可审核缓存</span>
                <strong>{fmtBytes(detailCat.sizeBytes)}</strong>
              </div>
              <div className="cleanup-review-stat">
                <span>{detailCat.childCategories ? "已发现工具链" : "风险等级"}</span>
                <strong>{detailCat.childCategories ? `${discoveredToolchains} 个` : riskMeta(detailCat.riskLevel).label}</strong>
              </div>
              <div className="cleanup-review-stat">
                <span>已验证路径</span>
                <strong>{detailCat.paths.length} 条</strong>
              </div>
              <div className="cleanup-review-stat">
                <span>当前已选</span>
                <strong>{fmtBytes(detailSelectedSize)}</strong>
              </div>
            </div>

            <div className={detailCat.childCategories ? "cleanup-review-body" : "cleanup-review-body cleanup-review-body--single"}>
              {detailCat.childCategories && (
                <aside className="cleanup-review-rail">
                  <div className="cleanup-review-rail-heading"><span>工具链</span><span>大小</span></div>
                  <div className="cleanup-review-rail-list">
                    {detailToolchains.map((category) => {
                      const selection = cleanupCategorySelection(category, selected, excludedPaths);
                      const active = category.id === activeDetailCategory.id;
                      return (
                        <button
                          type="button"
                          key={category.id}
                          className={`cleanup-review-tool${active ? " is-active" : ""}`}
                          onClick={() => {
                            setActiveToolchainId(category.id);
                          }}
                        >
                          <Checkbox
                            checked={selection.checked}
                            indeterminate={selection.indeterminate}
                            disabled={busy}
                            onClick={(event) => event.stopPropagation()}
                            onChange={() => toggleCategory(category)}
                          />
                          <span className="cleanup-review-tool-icon">{categoryIcon(category.id)}</span>
                          <span className="cleanup-review-tool-copy">
                            <strong>{category.name}</strong>
                            <small>{selection.checkedPathCount}/{selection.totalPathCount} 路径</small>
                          </span>
                          <b>{fmtBytes(category.sizeBytes)}</b>
                        </button>
                      );
                    })}
                  </div>
                </aside>
              )}

              <section className="cleanup-review-content">
                <div className="cleanup-review-content-head">
                  <div className="cleanup-review-content-heading">
                    <div>
                      <div className="cleanup-review-content-titleline">
                        <strong>{activeDetailCategory.name}</strong>
                        <span className="cleanup-risk-pill" style={{ color: riskMeta(activeDetailCategory.riskLevel).color, background: riskMeta(activeDetailCategory.riskLevel).background }}>
                          {riskMeta(activeDetailCategory.riskLevel).label}
                        </span>
                      </div>
                      <span>{activeDetailCategory.description}</span>
                    </div>
                  </div>
                  <div className="cleanup-review-content-tools">
                    <Button
                      className="cleanup-select-current-button"
                      type={activeDetailSelection?.checked ? "default" : "primary"}
                      danger={!!activeDetailSelection?.checked}
                      size="small"
                      disabled={busy || activeDetailCategory.paths.length === 0}
                      onClick={() => toggleCategory(activeDetailCategory)}
                    >
                      {activeDetailSelection?.checked ? "清除当前工具链" : "勾选当前工具链"}
                    </Button>
                  </div>
                </div>

                <div className="cleanup-review-paths">
                  {visibleDetailPaths.length > 0 ? visibleDetailPaths.map((path) => {
                    const checked = selected.has(activeDetailCategory.id)
                      && !excludedPaths.has(normalizePathKey(path.path));
                    return (
                      <article className={`cleanup-review-path${checked ? " is-selected" : ""}`} key={path.path}>
                        <Checkbox checked={checked} disabled={busy} onChange={() => togglePath(activeDetailCategory, path)} />
                        <div className="cleanup-review-path-main">
                          <button type="button" title={path.path} onClick={() => void openPath(path.path).catch(() => undefined)}>
                            <span className="cleanup-review-folder"><FolderOpenOutlined /></span>
                            <span>{path.path}</span>
                          </button>
                          <small title={path.matchedRule}>命中规则：{path.matchedRule}</small>
                        </div>
                        <div className="cleanup-review-path-source">
                          <Tag variant="filled">{sourceLabel(path.source)}</Tag>
                          <span style={{ color: riskMeta(activeDetailCategory.riskLevel).color }}>● {riskMeta(activeDetailCategory.riskLevel).label}</span>
                        </div>
                        <div className="cleanup-review-path-size">
                          <strong>{fmtBytes(path.sizeBytes)}</strong>
                          <small>{compactNumber(path.fileCount)} 文件</small>
                        </div>
                      </article>
                    );
                  }) : (
                    <Empty
                      image={Empty.PRESENTED_IMAGE_SIMPLE}
                      description="未发现该工具链缓存"
                    />
                  )}
                </div>
              </section>
            </div>

            <footer className="cleanup-review-foot">
              <div>
                当前已选 <strong>{fmtBytes(detailSelectedSize)}</strong>
                <span> · {detailSelectedPaths.length} 条路径 · {compactNumber(detailSelectedFiles)} 文件</span>
              </div>
              <Button type="primary" danger icon={<ThunderboltOutlined />} disabled={detailSelectedPaths.length === 0} onClick={closeDetail}>
                加入清理队列
              </Button>
            </footer>
          </div>
        )}
      </Modal>

      <Modal
        title={null}
        closable={false}
        open={projectManagerOpen}
        onCancel={() => setProjectManagerOpen(false)}
        footer={null}
        width={920}
        centered
        className="cleanup-review-modal cleanup-project-modal"
        styles={{ container: { padding: 0, overflow: "hidden" }, body: { padding: 0 } }}
      >
        <div className="cleanup-review-shell cleanup-project-shell">
          <header className="cleanup-review-head">
            <div className="cleanup-review-heading">
              <span className="cleanup-review-eyebrow">SCAN SCOPE · PERSISTED</span>
              <div className="cleanup-review-title-row">
                <span className="cleanup-review-title-icon"><FolderAddOutlined /></span>
                <div>
                  <strong>项目目录管理</strong>
                  <span>只发现符合规则的垃圾、缓存与编译产物，绝不会把项目根目录整体清空</span>
                </div>
              </div>
            </div>
            <div className="cleanup-review-head-actions">
              <Button
                type="text"
                className="cleanup-review-close"
                icon={<CloseOutlined />}
                onClick={() => setProjectManagerOpen(false)}
                aria-label="关闭"
              />
            </div>
          </header>

          <div className="cleanup-review-overview cleanup-project-overview">
            <div className="cleanup-review-stat cleanup-review-stat--primary">
              <span>自定义扫描根</span>
              <strong>{projectRoots.length} / {MAX_PROJECT_ROOTS}</strong>
            </div>
            <div className="cleanup-review-stat">
              <span>剩余名额</span>
              <strong>{MAX_PROJECT_ROOTS - projectRoots.length}</strong>
            </div>
            <div className="cleanup-review-stat">
              <span>自动扫描范围</span>
              <strong>用户目录 + 固定磁盘</strong>
            </div>
            <div className="cleanup-review-stat">
              <span>配置策略</span>
              <strong>本地持久化</strong>
            </div>
          </div>

          <div className="cleanup-project-body">
            <aside className="cleanup-project-guide">
              <span className="cleanup-project-guide-title">扫描规则</span>
              <div className="cleanup-project-guide-item is-active">
                <span><RadarChartOutlined /></span>
                <div><strong>优先侦察</strong><small>自定义根先于宽泛磁盘扫描处理</small></div>
              </div>
              <div className="cleanup-project-guide-item">
                <span><CheckCircleOutlined /></span>
                <div><strong>规范化去重</strong><small>大小写、长路径前缀和父子目录统一处理</small></div>
              </div>
              <div className="cleanup-project-guide-item is-warning">
                <span>!</span>
                <div><strong>不会整目录删除</strong><small>战争蚁只接收快照中的缓存子路径</small></div>
              </div>
            </aside>

            <section className="cleanup-project-content">
              <div className="cleanup-project-content-head">
                <div>
                  <strong>自定义工作区</strong>
                  <span>侦察蚁会深入子目录匹配规则；添加的工作区自身永远不是清理目标。</span>
                </div>
                <Tag color="blue" variant="filled">{projectRoots.length} 个目录</Tag>
              </div>

              <div className="cleanup-project-roots">
                {projectRoots.length > 0 ? projectRoots.map((root, index) => (
                  <article className="cleanup-project-root" key={normalizePathKey(root)}>
                    <span className="cleanup-project-root-index">{String(index + 1).padStart(2, "0")}</span>
                    <span className="cleanup-review-folder"><FolderOpenOutlined /></span>
                    <div className="cleanup-project-root-copy">
                      <Tooltip title={root} placement="topLeft">
                        <button type="button" onClick={() => void openPath(root).catch(() => undefined)}>{root}</button>
                      </Tooltip>
                      <span>优先扫描 · 配置已持久化</span>
                    </div>
                    <div className="cleanup-project-root-actions">
                      <Button type="text" size="small" icon={<EditOutlined />} onClick={() => void onReplaceProjectRoot(index)}>替换</Button>
                      <Popconfirm
                        title="移除此扫描目录？"
                        description="只移除扫描配置，不会删除磁盘上的目录。"
                        onConfirm={() => persistProjectRoots(removeProjectRoot(projectRoots, index))}
                      >
                        <Button type="text" danger size="small" icon={<DeleteOutlined />}>移除</Button>
                      </Popconfirm>
                    </div>
                  </article>
                )) : (
                  <Empty
                    image={Empty.PRESENTED_IMAGE_SIMPLE}
                    description="尚未添加项目目录，仍会自动扫描用户目录、固定磁盘候选目录和历史热点"
                  >
                    <Button type="primary" icon={<FolderAddOutlined />} onClick={() => void onAddProjectRoot()}>添加第一个目录</Button>
                  </Empty>
                )}
              </div>
            </section>
          </div>

          <footer className="cleanup-review-foot cleanup-project-foot">
            <div>
              已配置 <strong>{projectRoots.length}</strong>
              <span> 个优先扫描根 · 修改后立即保存</span>
            </div>
            <Space size={8}>
              <Popconfirm
                title="清空全部项目目录？"
                description="只移除扫描配置，不会删除磁盘上的任何文件。"
                onConfirm={() => persistProjectRoots([])}
                disabled={projectRoots.length === 0}
              >
                <Button danger disabled={projectRoots.length === 0}>清空全部</Button>
              </Popconfirm>
              <Button
                icon={<FolderAddOutlined />}
                disabled={projectRoots.length >= MAX_PROJECT_ROOTS}
                onClick={() => void onAddProjectRoot()}
              >
                添加扫描根
              </Button>
              <Button type="primary" onClick={() => setProjectManagerOpen(false)}>完成</Button>
            </Space>
          </footer>
        </div>
      </Modal>
    </Space>
  );
}
