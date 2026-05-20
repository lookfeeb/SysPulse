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
const DEFAULT_UNSELECTED_CATEGORY_IDS = new Set(["recycle-bin"]);
const SELECTED_STORAGE_KEY = "syspulse.cleanup.selectedCategories.v2";
const EXCLUDED_PATHS_STORAGE_KEY = "syspulse.cleanup.excludedPaths.v1";

let cachedCategories: CleanupCategory[] | null = null;

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
  if (stored !== null) return new Set(stored.filter((id) => availableIds.has(id)));
  return new Set(categories.filter((c) => !DEFAULT_UNSELECTED_CATEGORY_IDS.has(c.id)).map((c) => c.id));
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
      childCategories: programmingCategories,
    },
  ];
}

function displayCategoryIds(category: DisplayCategory): string[] {
  return category.childCategories?.map((cat) => cat.id) ?? [category.id];
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

export default function CleanupPage() {
  const { message } = AntdApp.useApp();
  const [categories, setCategories] = useState<CleanupCategory[]>(cachedCategories ?? []);
  const [selected, setSelected] = useState<Set<string>>(() => restoreSelected(cachedCategories ?? []));
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [detailCat, setDetailCat] = useState<DisplayCategory | null>(null);
  const [excludedPaths, setExcludedPaths] = useState<Set<string>>(() => restoreExcludedPaths());
  const [cleanProgress, setCleanProgress] = useState<CleanupProgressEvent | null>(null);

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

  const selectedCategoryCount = categories.filter((cat) => selected.has(cat.id)).length;
  const allSelected = categories.length > 0 && selectedCategoryCount === categories.length;
  const partiallySelected = selectedCategoryCount > 0 && selectedCategoryCount < categories.length;

  const saveSelected = (next: Set<string>) => writeStoredStringList(SELECTED_STORAGE_KEY, next);
  const saveExcludedPaths = (next: Set<string>) => writeStoredStringList(EXCLUDED_PATHS_STORAGE_KEY, next);

  const updateSelected = (updater: (prev: Set<string>) => Set<string>) => {
    setSelected((prev) => {
      const next = updater(prev);
      saveSelected(next);
      return next;
    });
  };

  const updateExcludedPaths = (updater: (prev: Set<string>) => Set<string>) => {
    setExcludedPaths((prev) => {
      const next = updater(prev);
      saveExcludedPaths(next);
      return next;
    });
  };

  const toggleCategory = (category: DisplayCategory) => {
    const ids = displayCategoryIds(category);
    const checked = ids.every((id) => selected.has(id));
    updateSelected((prev) => {
      const next = new Set(prev);
      ids.forEach((id) => {
        if (checked) next.delete(id);
        else next.add(id);
      });
      return next;
    });
  };

  const toggleAllCategories = () => {
    updateSelected(() => (allSelected ? new Set() : new Set(categories.map((cat) => cat.id))));
  };

  const onScan = async () => {
    setScanning(true);
    setCleanProgress(null);
    try {
      const result = await commands.scanCleanup();
      setCategories(result.categories);
      cachedCategories = result.categories;
      setSelected(restoreSelected(result.categories));
      void message.success({ content: `扫描完成，发现 ${fmtBytes(result.totalSizeBytes)} 可清理`, key: "cleanup-scan", duration: 2 });
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  };

  const onClean = async () => {
    if (selected.size === 0) return;
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
      const result = await commands.cleanCategories({ categoryIds: [...selected], excludedPaths: [...excludedPaths] });
      void message.success({ content: `已释放 ${fmtBytes(result.freedBytes)}，删除 ${result.deletedFiles} 个文件`, key: "cleanup-clean", duration: 3 });
      if (result.errors.length > 0) {
        void message.warning({ content: `${result.errors.length} 个路径清理失败`, duration: 3 });
      }
      const fresh = await commands.scanCleanup();
      setCategories(fresh.categories);
      cachedCategories = fresh.categories;
      setSelected(restoreSelected(fresh.categories));
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setCleaning(false);
    }
  };

  const categoryIcon = (id: string) => {
    const map: Record<string, string> = {
      "win-temp": "🗑️",
      "prefetch": "⚡",
      "win-update": "🔄",
      "recycle-bin": "♻️",
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

  const renderPathRows = (paths: PathDetail[]) => (
    <div className="cleanup-path-list" style={{ maxHeight: 320, overflow: "auto", borderRadius: 8, border: "1px solid #edf0f5", background: "#fff" }}>
      {paths.map((path, index) => (
        <div
          key={path.path}
          style={{
            display: "grid",
            gridTemplateColumns: "24px 30px minmax(0, 1fr) auto",
            alignItems: "center",
            gap: 8,
            padding: "10px 12px",
            borderBottom: index < paths.length - 1 ? "1px solid #f1f3f7" : "none",
          }}
        >
          <Checkbox
            checked={!excludedPaths.has(path.path)}
            onChange={() => updateExcludedPaths((prev) => {
              const next = new Set(prev);
              if (next.has(path.path)) next.delete(path.path);
              else next.add(path.path);
              return next;
            })}
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
      <Title level={4} style={{ marginBottom: 16 }}>磁盘清理</Title>

      <Space style={{ marginBottom: 16 }} wrap>
        {categories.length > 0 && (
          <span style={{ display: "inline-flex" }}>
            <Checkbox
              checked={allSelected}
              indeterminate={partiallySelected}
              onChange={toggleAllCategories}
            />
          </span>
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
          description={`将清理 ${fmtBytes(totalSelected)}，此操作不可撤销`}
          onConfirm={onClean}
          disabled={selected.size === 0 || cleaning}
        >
          <Button
            danger
            icon={<ThunderboltOutlined />}
            loading={cleaning}
            disabled={selected.size === 0 || cleaning}
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
        <div style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 12 }}>
          {displayCategories.map((cat) => {
            const ids = displayCategoryIds(cat);
            const checkedCount = ids.filter((id) => selected.has(id)).length;
            const checked = checkedCount === ids.length;
            const indeterminate = checkedCount > 0 && checkedCount < ids.length;
            const visiblePaths = cat.childCategories
              ? cat.childCategories.flatMap((child) => cleanablePaths(child, selected, excludedPaths))
              : cleanablePaths(cat, selected, excludedPaths);
            const visibleSize = sumPathSize(visiblePaths);
            const visibleFiles = sumPathFiles(visiblePaths);

            return (
              <Card
                key={cat.id}
                size="small"
                hoverable
                onClick={() => setDetailCat(cat)}
                style={{
                  border: checked || indeterminate ? "1px solid #1677ff" : "1px solid #e5e7eb",
                  background: checked || indeterminate ? "#f0f5ff" : "#fff",
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
                    <div style={{ fontWeight: 600 }}>{cat.name}</div>
                    <Text type="secondary" style={{ display: "block", fontSize: 12 }} ellipsis>
                      {cat.childCategories ? `${cat.description} · ${checkedCount}/${ids.length} 已选` : cat.description}
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
        styles={{ body: { maxHeight: "calc(100vh - 180px)", overflow: "hidden" } }}
      >
        {detailCat && (() => {
          const detailCategories = detailCat.childCategories ?? [detailCat];
          const detailPaths = detailCategories.flatMap((cat) => cat.paths);
          const activePaths = detailCategories.flatMap((cat) => cleanablePaths(cat, selected, excludedPaths));
          const checkedPathCount = checkedPaths(detailPaths, excludedPaths).length;
          const allPathsChecked = detailPaths.length > 0 && checkedPathCount === detailPaths.length;

          return (
            <div style={{ display: "flex", flexDirection: "column", maxHeight: "calc(100vh - 180px)", minHeight: 0, overflow: "hidden" }}>
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
                  onClick={() => updateExcludedPaths((prev) => {
                    const next = new Set(prev);
                    detailPaths.forEach((path) => {
                      if (allPathsChecked) next.add(path.path);
                      else next.delete(path.path);
                    });
                    return next;
                  })}
                >
                  {allPathsChecked ? "取消全选" : "全选"}
                </Button>
              </div>

              {detailCat.childCategories ? (
                <div className="cleanup-detail-list" style={{ display: "grid", gap: 12, overflow: "auto", paddingRight: 2, minHeight: 0, flex: "1 1 auto" }}>
                  {detailCat.childCategories.map((cat) => {
                    const checkedCatPaths = checkedPaths(cat.paths, excludedPaths);
                    const activeCatPaths = cleanablePaths(cat, selected, excludedPaths);
                    return (
                      <div key={cat.id} style={{ border: "1px solid #edf0f5", borderRadius: 8, overflow: "hidden" }}>
                        <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "10px 12px", background: "#fafcff", borderBottom: "1px solid #edf0f5" }}>
                          <Checkbox checked={selected.has(cat.id)} onChange={() => toggleCategory(cat)} />
                          <span style={{ fontSize: 18 }}>{categoryIcon(cat.id)}</span>
                          <div style={{ flex: 1, minWidth: 0 }}>
                            <div style={{ fontWeight: 600 }}>{cat.name}</div>
                            <Text type="secondary" style={{ display: "block", fontSize: 12 }} ellipsis>
                              {cat.description} · {checkedCatPaths.length}/{cat.paths.length} 路径
                            </Text>
                          </div>
                          <div style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                            <div style={{ fontSize: 13, fontWeight: 700, color: "#cf1322" }}>{fmtBytes(sumPathSize(activeCatPaths))}</div>
                            <Text type="secondary" style={{ fontSize: 11 }}>{sumPathFiles(activeCatPaths).toLocaleString()} 文件</Text>
                          </div>
                        </div>
                        {renderPathRows(cat.paths)}
                      </div>
                    );
                  })}
                </div>
              ) : (
                renderPathRows(detailCat.paths)
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
