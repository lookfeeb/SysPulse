import { useEffect, useMemo, useState } from "react";
import {
  App as AntdApp,
  Button,
  Card,
  Checkbox,
  Modal,
  Popconfirm,
  Progress,
  Space,
  Typography,
} from "antd";
import {
  CodeOutlined,
  FolderOpenOutlined,
  ScanOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { commands } from "@/bindings";
import type { CleanupCategory, PathDetail } from "@/bindings";
import { listen } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";
import { fmtBytes } from "@/utils/format";
import { readStoredStringList, writeStoredStringList } from "@/utils/storageList";

const { Title, Text } = Typography;

const PROGRAMMING_CATEGORY_ID = "programming-cache";
const PROGRAMMING_CATEGORY_IDS = new Set(["rust-target", "go-cache", "python-cache", "node-cache"]);
const SELECTED_STORAGE_KEY = "syspulse.cleanup.selectedCategories.v3";
const EXCLUDED_PATHS_STORAGE_KEY = "syspulse.cleanup.excludedPaths.v1";

let cachedCategories: CleanupCategory[] | null = null;
let cachedScanId: string | null = null;

type DisplayCategory = CleanupCategory & {
  childCategories?: CleanupCategory[];
};

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

function restoreSelected(categories: CleanupCategory[]): Set<string> {
  const availableIds = new Set(categories.map((c) => c.id));
  const stored = readStoredStringList(SELECTED_STORAGE_KEY);
  if (stored !== null) {
    const safeDefaultIds = new Set(
      categories.filter((category) => category.defaultSelected).map((category) => category.id),
    );
    return new Set(stored.filter((id) => availableIds.has(id) && safeDefaultIds.has(id)));
  }
  return new Set(categories.filter((category) => category.defaultSelected).map((category) => category.id));
}

function restoreExcludedPaths(): Set<string> {
  return new Set(readStoredStringList(EXCLUDED_PATHS_STORAGE_KEY) ?? []);
}

function buildDisplayCategories(categories: CleanupCategory[]): DisplayCategory[] {
  const programmingCategories = categories.filter((cat) => PROGRAMMING_CATEGORY_IDS.has(cat.id));
  const otherCategories = categories.filter((cat) => !PROGRAMMING_CATEGORY_IDS.has(cat.id));

  if (programmingCategories.length === 0) return otherCategories;

  return [
    ...otherCategories,
    {
      id: PROGRAMMING_CATEGORY_ID,
      name: "编程缓存",
      description: programmingCategories.map((cat) => cat.name.replace(/\s*缓存$/, "")).join(" / "),
      sizeBytes: programmingCategories.reduce((sum, cat) => sum + cat.sizeBytes, 0),
      fileCount: programmingCategories.reduce((sum, cat) => sum + cat.fileCount, 0),
      paths: programmingCategories.flatMap((cat) => cat.paths),
      riskLevel: "advanced",
      defaultSelected: false,
      minAgeDays: null,
      childCategories: programmingCategories,
    },
  ];
}

function checkedPaths(paths: PathDetail[], excludedPaths: Set<string>): PathDetail[] {
  return paths.filter((path) => !excludedPaths.has(path.path));
}

function sumPathSize(paths: PathDetail[]): number {
  return paths.reduce((sum, path) => sum + path.sizeBytes, 0);
}

function sumPathFiles(paths: PathDetail[]): number {
  return paths.reduce((sum, path) => sum + path.fileCount, 0);
}

function cleanablePaths(category: CleanupCategory, selected: Set<string>, excludedPaths: Set<string>): PathDetail[] {
  if (!selected.has(category.id)) return [];
  return checkedPaths(category.paths, excludedPaths);
}

type SelectionState = {
  checked: boolean;
  indeterminate: boolean;
  checkedPathCount: number;
  totalPathCount: number;
  selectedCategoryCount: number;
  totalCategoryCount: number;
};

function cleanupCategorySelection(
  category: CleanupCategory,
  selected: Set<string>,
  excludedPaths: Set<string>,
): SelectionState {
  const categorySelected = selected.has(category.id);
  const totalPathCount = category.paths.length;
  const checkedPathCount = categorySelected
    ? checkedPaths(category.paths, excludedPaths).length
    : 0;

  return {
    checked: categorySelected && (totalPathCount === 0 || checkedPathCount === totalPathCount),
    indeterminate:
      categorySelected && checkedPathCount > 0 && checkedPathCount < totalPathCount,
    checkedPathCount,
    totalPathCount,
    selectedCategoryCount: categorySelected ? 1 : 0,
    totalCategoryCount: 1,
  };
}

function displayCategorySelection(
  category: DisplayCategory,
  selected: Set<string>,
  excludedPaths: Set<string>,
): SelectionState {
  const childStates = (category.childCategories ?? [category]).map((child) =>
    cleanupCategorySelection(child, selected, excludedPaths),
  );
  const checked = childStates.length > 0 && childStates.every((state) => state.checked);
  const hasSelection = childStates.some((state) => state.checked || state.indeterminate);

  return {
    checked,
    indeterminate: hasSelection && !checked,
    checkedPathCount: childStates.reduce((sum, state) => sum + state.checkedPathCount, 0),
    totalPathCount: childStates.reduce((sum, state) => sum + state.totalPathCount, 0),
    selectedCategoryCount: childStates.reduce((sum, state) => sum + state.selectedCategoryCount, 0),
    totalCategoryCount: childStates.reduce((sum, state) => sum + state.totalCategoryCount, 0),
  };
}

export default function CleanupPage() {
  const { message } = AntdApp.useApp();
  const [categories, setCategories] = useState<CleanupCategory[]>(cachedCategories ?? []);
  const [selected, setSelected] = useState<Set<string>>(() => restoreSelected(cachedCategories ?? []));
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [detailCat, setDetailCat] = useState<DisplayCategory | null>(null);
  const [expandedCats, setExpandedCats] = useState<Record<string, boolean>>({});
  const [excludedPaths, setExcludedPaths] = useState<Set<string>>(() => restoreExcludedPaths());
  const [cleanProgress, setCleanProgress] = useState<CleanupProgressEvent | null>(null);
  const [scanId, setScanId] = useState<string | null>(cachedScanId);

  const displayCategories = useMemo(() => buildDisplayCategories(categories), [categories]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void listen<CleanupProgressEvent>("cleanup:progress", (event) => {
      setCleanProgress(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, []);

  const totalSelected = categories
    .filter((cat) => selected.has(cat.id))
    .reduce((sum, cat) => sum + sumPathSize(cleanablePaths(cat, selected, excludedPaths)), 0);

  const selectedPathCount = categories
    .reduce((sum, cat) => sum + cleanablePaths(cat, selected, excludedPaths).length, 0);
  const selectionStates = categories.map((cat) => cleanupCategorySelection(cat, selected, excludedPaths));
  const allSelected = selectionStates.length > 0 && selectionStates.every((state) => state.checked);
  const partiallySelected = selectionStates.some((state) => state.checked || state.indeterminate) && !allSelected;

  const saveSelected = (next: Set<string>) => writeStoredStringList(SELECTED_STORAGE_KEY, next);
  const saveExcludedPaths = (next: Set<string>) => writeStoredStringList(EXCLUDED_PATHS_STORAGE_KEY, next);

  const commitSelection = (nextSelected: Set<string>, nextExcludedPaths: Set<string>) => {
    saveSelected(nextSelected);
    saveExcludedPaths(nextExcludedPaths);
    setSelected(nextSelected);
    setExcludedPaths(nextExcludedPaths);
  };

  const setCategoriesChecked = (targetCategories: CleanupCategory[], checked: boolean) => {
    const nextSelected = new Set(selected);
    const nextExcludedPaths = new Set(excludedPaths);

    targetCategories.forEach((category) => {
      if (checked) nextSelected.add(category.id);
      else nextSelected.delete(category.id);

      category.paths.forEach((path) => {
        if (checked) nextExcludedPaths.delete(path.path);
        else nextExcludedPaths.add(path.path);
      });
    });

    commitSelection(nextSelected, nextExcludedPaths);
  };

  const toggleCategory = (category: DisplayCategory) => {
    const state = displayCategorySelection(category, selected, excludedPaths);
    setCategoriesChecked(category.childCategories ?? [category], !state.checked);
  };

  const toggleAllCategories = () => {
    setCategoriesChecked(categories, !allSelected);
  };

  const selectedAdvanced = categories.some(
    (category) => category.riskLevel === "advanced" && selected.has(category.id),
  );
  const selectedCaution = categories.some(
    (category) => category.riskLevel === "caution" && selected.has(category.id),
  );

  const togglePath = (category: CleanupCategory, path: PathDetail) => {
    const nextSelected = new Set(selected);
    const nextExcludedPaths = new Set(excludedPaths);
    const pathChecked = selected.has(category.id) && !excludedPaths.has(path.path);

    if (pathChecked) {
      nextExcludedPaths.add(path.path);
      const hasCheckedPath = category.paths.some((item) =>
        item.path !== path.path && !nextExcludedPaths.has(item.path),
      );
      if (!hasCheckedPath) nextSelected.delete(category.id);
    } else {
      nextSelected.add(category.id);
      nextExcludedPaths.delete(path.path);
    }

    commitSelection(nextSelected, nextExcludedPaths);
  };

  const onScan = async () => {
    setScanning(true);
    setCleanProgress(null);
    try {
      const result = await commands.scanCleanup();
      setCategories(result.categories);
      cachedCategories = result.categories;
      cachedScanId = result.scanId;
      setScanId(result.scanId);
      setSelected(restoreSelected(result.categories));
      void message.success({ content: `扫描完成，发现 ${fmtBytes(result.totalSizeBytes)} 可清理`, key: "cleanup-scan", duration: 2 });
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  };

  const onClean = async () => {
    if (selectedPathCount === 0 || !scanId) return;
    setCleaning(true);
    setCleanProgress({
      percent: 0,
      processedItems: 0,
      totalItems: 0,
      currentCategory: "准备清理",
      currentPath: null,
      freedBytes: 0,
      deletedFiles: 0,
      done: false,
    });
    try {
      const result = await commands.cleanCategories({
        scanId,
        categoryIds: [...selected],
        excludedPaths: [...excludedPaths],
        confirmCaution: selectedCaution,
        confirmAdvanced: selectedAdvanced,
      });
      void message.success({ content: `已释放 ${fmtBytes(result.freedBytes)}，删除 ${result.deletedFiles} 个文件`, key: "cleanup-clean", duration: 3 });
      if (result.errors.length > 0) {
        void message.warning({ content: `${result.errors.length} 个路径清理失败`, duration: 3 });
      }
      const fresh = await commands.scanCleanup();
      setCategories(fresh.categories);
      cachedCategories = fresh.categories;
      cachedScanId = fresh.scanId;
      setScanId(fresh.scanId);
      setSelected(restoreSelected(fresh.categories));
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setCleaning(false);
    }
  };

  const categoryIcon = (id: string) => {
    const map: Record<string, string> = {
      "win-temp": "♻️",
      "prefetch": "⚡",
      "win-update": "🔄",
      "recycle-bin": "🗑️",
      "rust-target": "🦀",
      "node-cache": "📦",
      "go-cache": "🐹",
      "python-cache": "🐍",
      "browser-cache": "🌐",
      "webview-cache": "🧩",
      "app-cache": "🧰",
      "thumbnails": "🖼️",
      "chrome-update": "🔄",
      "notion-cache": "📝",
      "wer-cache": "📋",
      "shader-cache": "🎮",
      "installer-cache": "📥",
    };
    return map[id] ?? "📁";
  };

  const riskMeta = (risk: CleanupCategory["riskLevel"]) => {
    if (risk === "safe") return { label: "安全", color: "#16845b", background: "#e7f8f0" };
    if (risk === "caution") return { label: "谨慎", color: "#a96b00", background: "#fff3d8" };
    return { label: "高级", color: "#b42335", background: "#fdecef" };
  };

  const renderPathRows = (category: CleanupCategory) => (
    <div className="cleanup-path-list" style={{ maxHeight: 320, overflow: "auto", borderRadius: 8, border: "1px solid #edf0f5", background: "#fff" }}>
      {category.paths.map((path, index) => (
        <div
          key={path.path}
          style={{
            display: "grid",
            gridTemplateColumns: "24px 30px minmax(0, 1fr) auto",
            alignItems: "center",
            gap: 8,
            padding: "10px 12px",
            borderBottom: index < category.paths.length - 1 ? "1px solid #f1f3f7" : "none",
          }}
        >
          <Checkbox
            checked={selected.has(category.id) && !excludedPaths.has(path.path)}
            onChange={() => togglePath(category, path)}
          />
          <span
            style={{
              width: 30,
              height: 30,
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              borderRadius: 6,
              background: "#f5f7fb",
              color: "#64748b",
              fontSize: 15,
            }}
          >
            <FolderOpenOutlined />
          </span>
          <button
            type="button"
            title={path.path}
            style={{
              minWidth: 0,
              padding: 0,
              border: 0,
              background: "transparent",
              color: "#1677ff",
              cursor: "pointer",
              fontFamily: "Consolas, JetBrains Mono, monospace",
              fontSize: 12,
              lineHeight: 1.45,
              overflow: "hidden",
              textAlign: "left",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            onClick={() => { void openPath(path.path).catch(() => {}); }}
          >
            {path.path}
          </button>
          <div style={{ textAlign: "right", whiteSpace: "nowrap", marginLeft: 8 }}>
            <div style={{ fontSize: 12, fontWeight: 600, color: "#cf1322" }}>{fmtBytes(path.sizeBytes)}</div>
            <div style={{ fontSize: 10, color: "#8c8c8c" }}>{path.fileCount.toLocaleString()} 文件</div>
          </div>
        </div>
      ))}
    </div>
  );

  return (
    <div style={{ padding: "0 4px" }}>
      <Title level={4} style={{ marginBottom: 4 }}>磁盘清理</Title>
      <Text type="secondary" style={{ display: "block", marginBottom: 16 }}>
        默认只选择超过保留期的安全垃圾；程序缓存、编译产物和系统维护项均需手动确认
      </Text>

      {categories.length > 0 && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 12, marginBottom: 16 }}>
          {(["safe", "caution", "advanced"] as const).map((risk) => {
            const matching = categories.filter((category) => category.riskLevel === risk);
            const size = matching.reduce((sum, category) => sum + category.sizeBytes, 0);
            const meta = riskMeta(risk);
            const title = risk === "safe" ? "安全垃圾" : risk === "caution" ? "可再生成缓存" : "高级维护项";
            return (
              <div key={risk} style={{ background: "#fff", border: "1px solid #e5e7eb", borderRadius: 10, padding: "12px 14px" }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <Text type="secondary" style={{ fontSize: 12 }}>{title}</Text>
                  <span style={{ fontSize: 11, fontWeight: 600, color: meta.color, background: meta.background, borderRadius: 10, padding: "2px 8px" }}>{meta.label}</span>
                </div>
                <div style={{ fontSize: 22, fontWeight: 700, marginTop: 4 }}>{fmtBytes(size)}</div>
                <Text type="secondary" style={{ fontSize: 11 }}>{matching.length} 个类别</Text>
              </div>
            );
          })}
        </div>
      )}

      <Space style={{ marginBottom: 16 }} wrap>
        {categories.length > 0 && (
          <Checkbox
            checked={allSelected}
            indeterminate={partiallySelected}
            onChange={toggleAllCategories}
          >
            <Text type="secondary" style={{ fontSize: 12 }}>全选当前项目</Text>
          </Checkbox>
        )}
        <Button
          type="primary"
          icon={<ScanOutlined />}
          loading={scanning}
          disabled={cleaning}
          onClick={onScan}
        >
          扫描
        </Button>
        <Popconfirm
          title="确认清理"
          description={
            selectedAdvanced
              ? `包含高级维护项，将清理 ${fmtBytes(totalSelected)}，请确认已查看路径明细`
              : selectedCaution
                ? `包含可再生成缓存，将清理 ${fmtBytes(totalSelected)}；相关程序运行时会自动跳过`
                : `将安全清理 ${fmtBytes(totalSelected)}，此操作不可撤销`
          }
          onConfirm={() => {
            void onClean();
          }}
          disabled={selectedPathCount === 0 || cleaning || !scanId}
        >
          <Button
            danger
            icon={<ThunderboltOutlined />}
            loading={cleaning}
            disabled={selectedPathCount === 0 || cleaning || !scanId}
          >
            清理选中 ({fmtBytes(totalSelected)})
          </Button>
        </Popconfirm>
      </Space>

      {cleanProgress && (
        <div style={{ border: "1px solid #edf0f5", borderRadius: 8, padding: "12px 14px", marginBottom: 16, background: "#fafcff" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12, marginBottom: 8 }}>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontWeight: 600 }}>
                {cleanProgress.done ? "清理完成" : `正在清理：${cleanProgress.currentCategory || "准备中"}`}
              </div>
              {cleanProgress.currentPath && (
                <Text type="secondary" style={{ display: "block", fontSize: 12 }} ellipsis title={cleanProgress.currentPath}>
                  {cleanProgress.currentPath}
                </Text>
              )}
            </div>
            <div style={{ textAlign: "right", whiteSpace: "nowrap" }}>
              <div style={{ fontWeight: 700, color: "#cf1322" }}>{fmtBytes(cleanProgress.freedBytes)}</div>
              <Text type="secondary" style={{ fontSize: 11 }}>{cleanProgress.deletedFiles.toLocaleString()} 个文件</Text>
            </div>
          </div>
          <Progress
            percent={cleanProgress.percent}
            status={cleanProgress.done ? "success" : "active"}
            size="small"
          />
        </div>
      )}

      {categories.length > 0 && (
        <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr)", gap: 10 }}>
          {[...displayCategories].sort((a, b) => ({ safe: 0, caution: 1, advanced: 2 }[a.riskLevel] - { safe: 0, caution: 1, advanced: 2 }[b.riskLevel])).map((cat) => {
            const selection = displayCategorySelection(cat, selected, excludedPaths);
            const checked = selection.checked;
            const indeterminate = selection.indeterminate;
            const visiblePaths = cat.childCategories
              ? cat.childCategories.flatMap((child) => cleanablePaths(child, selected, excludedPaths))
              : cleanablePaths(cat, selected, excludedPaths);
            const visibleSize = sumPathSize(visiblePaths);
            const visibleFiles = sumPathFiles(visiblePaths);
            const risk = riskMeta(cat.riskLevel);

            return (
              <Card
                key={cat.id}
                size="small"
                hoverable
                onClick={() => setDetailCat(cat)}
                style={{
                  border: checked || indeterminate ? "1px solid #1677ff" : "1px solid #e5e7eb",
                  background: checked || indeterminate ? "#f6faff" : "#fff",
                  cursor: "pointer",
                }}
              >
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <Checkbox
                    checked={checked}
                    indeterminate={indeterminate}
                    onClick={(event) => event.stopPropagation()}
                    onChange={() => toggleCategory(cat)}
                  />
                  <span
                    style={{
                      width: 34,
                      height: 34,
                      display: "inline-flex",
                      alignItems: "center",
                      justifyContent: "center",
                      borderRadius: 8,
                      background: checked || indeterminate ? "#e6f4ff" : "#f5f7fb",
                      color: cat.id === PROGRAMMING_CATEGORY_ID ? "#1677ff" : undefined,
                      fontSize: 20,
                    }}
                  >
                    {cat.id === PROGRAMMING_CATEGORY_ID ? <CodeOutlined /> : categoryIcon(cat.id)}
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontWeight: 600, display: "flex", alignItems: "center", gap: 8 }}>
                      <span>{cat.name}</span>
                      <span style={{ fontSize: 11, fontWeight: 600, color: risk.color, background: risk.background, borderRadius: 10, padding: "1px 7px" }}>{risk.label}</span>
                      {cat.minAgeDays != null && <span style={{ fontSize: 11, color: "#16845b" }}>仅 {cat.minAgeDays} 天以上</span>}
                    </div>
                    <Text type="secondary" style={{ display: "block", fontSize: 12 }} ellipsis>
                      {cat.childCategories
                        ? `${cat.description} · ${selection.checkedPathCount}/${selection.totalPathCount} 路径`
                        : cat.description}
                    </Text>
                  </div>
                  <div style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                    <div style={{ fontWeight: 700, color: "#cf1322" }}>{fmtBytes(visibleSize)}</div>
                    <Text type="secondary" style={{ fontSize: 11 }}>{visibleFiles.toLocaleString()} 个文件</Text>
                  </div>
                </div>
              </Card>
            );
          })}
        </div>
      )}

      <Modal
        title={detailCat ? (
          <Space size={8}>
            {detailCat.id === PROGRAMMING_CATEGORY_ID ? <CodeOutlined /> : <span>{categoryIcon(detailCat.id)}</span>}
            <span>{detailCat.name}</span>
          </Space>
        ) : ""}
        open={!!detailCat}
        onCancel={() => setDetailCat(null)}
        footer={null}
        width={720}
        styles={{ body: { height: 500, display: "flex", flexDirection: "column", padding: "12px 24px" } }}
      >
        {detailCat && (() => {
          const detailCategories = detailCat.childCategories ?? [detailCat];
          const activePaths = detailCategories.flatMap((cat) => cleanablePaths(cat, selected, excludedPaths));
          const detailSelection = displayCategorySelection(detailCat, selected, excludedPaths);
          const allPathsChecked = detailSelection.checked;

          return (
            <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0, overflow: "hidden" }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", background: "#f7f9fc", border: "1px solid #edf0f5", borderRadius: 8, padding: "12px 16px", marginBottom: 16, flexShrink: 0 }}>
                <div>
                  <Text type="secondary" style={{ fontSize: 12 }}>占用空间</Text>
                  <div style={{ fontSize: 20, fontWeight: 700, color: "#cf1322" }}>{fmtBytes(sumPathSize(activePaths))}</div>
                </div>
                <div style={{ textAlign: "right" }}>
                  <Text type="secondary" style={{ fontSize: 12 }}>文件数量</Text>
                  <div style={{ fontSize: 20, fontWeight: 700 }}>{sumPathFiles(activePaths).toLocaleString()}</div>
                </div>
              </div>

              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8, flexShrink: 0 }}>
                <Text type="secondary" style={{ fontSize: 12 }}>扫描路径（取消勾选的路径不会被清理）</Text>
                <Button
                  size="small"
                  type="link"
                  onClick={() => setCategoriesChecked(detailCategories, !allPathsChecked)}
                >
                  {allPathsChecked ? "取消全选" : "全选"}
                </Button>
              </div>

              {detailCat.childCategories ? (
                <div className="cleanup-detail-list" style={{ border: "1px solid #edf0f5", borderRadius: 8, overflowY: "auto", paddingRight: 6, minHeight: 0, flex: "0 1 auto" }}>
                  {detailCat.childCategories.map((cat, idx) => {
                    const catSelection = cleanupCategorySelection(cat, selected, excludedPaths);
                    const activeCatPaths = cleanablePaths(cat, selected, excludedPaths);
                    const isExpanded = expandedCats[cat.id] !== false;
                    return (
                      <div key={cat.id} style={{ borderTop: idx > 0 ? "1px solid #edf0f5" : "none" }}>
                        <div
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 10,
                            padding: "10px 12px",
                            cursor: "pointer",
                            userSelect: "none",
                          }}
                            onClick={() => setExpandedCats(prev => ({ ...prev, [cat.id]: !prev[cat.id] }))}
                        >
                          <Checkbox
                            checked={catSelection.checked}
                            indeterminate={catSelection.indeterminate}
                            onClick={(e) => e.stopPropagation()}
                            onChange={() => toggleCategory(cat)}
                          />
                          <span style={{ fontSize: 18 }}>{categoryIcon(cat.id)}</span>
                          <div style={{ flex: 1, minWidth: 0 }}>
                            <div style={{ fontWeight: 600, display: "flex", alignItems: "center", gap: 6 }}>
                              <span>{cat.name}</span>
                              <span style={{ fontSize: 11, color: "#8c8c8c", transition: "transform 0.2s", transform: isExpanded ? "rotate(0deg)" : "rotate(-90deg)" }}>▼</span>
                            </div>
                            <Text type="secondary" style={{ display: "block", fontSize: 12 }} ellipsis>
                              {cat.description} · {catSelection.checkedPathCount}/{catSelection.totalPathCount} 路径
                            </Text>
                          </div>
                          <div style={{ textAlign: "right", whiteSpace: "nowrap" }} onClick={(e) => e.stopPropagation()}>
                            <div style={{ fontSize: 13, fontWeight: 700, color: "#cf1322" }}>{fmtBytes(sumPathSize(activeCatPaths))}</div>
                            <Text type="secondary" style={{ fontSize: 11 }}>{sumPathFiles(activeCatPaths).toLocaleString()} 文件</Text>
                          </div>
                        </div>
                        {isExpanded && renderPathRows(cat)}
                      </div>
                    );
                  })}
                </div>
              ) : (
                renderPathRows(detailCat)
              )}
            </div>
          );
        })()}
      </Modal>

      {categories.length === 0 && !scanning && (
        <div style={{ textAlign: "center", padding: 40, color: "#999" }}>
          点击「扫描」开始检测可清理的文件
        </div>
      )}
    </div>
  );
}
