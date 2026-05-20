import { useState } from "react";
import {
  Button,
  Card,
  Checkbox,
  App as AntdApp,
  Modal,
  Space,
  Typography,
  Popconfirm,
} from "antd";
import {
  ScanOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { commands } from "@/bindings";
import type { CleanupCategory } from "@/bindings";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

const { Title, Text } = Typography;

function fmtSize(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

let cachedCategories: CleanupCategory[] | null = null;

export default function CleanupPage() {
  const { message } = AntdApp.useApp();
  const [categories, setCategories] = useState<CleanupCategory[]>(cachedCategories ?? []);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [detailCat, setDetailCat] = useState<CleanupCategory | null>(null);
  // Per-category excluded paths
  const [excludedPaths, setExcludedPaths] = useState<Set<string>>(new Set());

  const totalSelected = categories
    .filter((c) => selected.has(c.id))
    .reduce((sum, c) => sum + c.paths.filter((p) => !excludedPaths.has(p.path)).reduce((s, p) => s + p.sizeBytes, 0), 0);

  const onScan = async () => {
    setScanning(true);
    try {
      const r = await commands.scanCleanup();
      setCategories(r.categories);
      cachedCategories = r.categories;
      setSelected(new Set(r.categories.map((c) => c.id)));
      void message.success({ content: `扫描完成，发现 ${fmtSize(r.totalSizeBytes)} 可清理`, key: "cleanup-scan", duration: 2 });
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  };

  const onClean = async () => {
    if (selected.size === 0) return;
    setCleaning(true);
    try {
      const r = await commands.cleanCategories({ categoryIds: [...selected], excludedPaths: [...excludedPaths] });
      void message.success({ content: `已释放 ${fmtSize(r.freedBytes)}，删除 ${r.deletedFiles} 个文件`, key: "cleanup-clean", duration: 3 });
      if (r.errors.length > 0) {
        void message.warning({ content: `${r.errors.length} 个路径清理失败`, duration: 3 });
      }
      const fresh = await commands.scanCleanup();
      setCategories(fresh.categories);
      cachedCategories = fresh.categories;
      setSelected(new Set());
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setCleaning(false);
    }
  };

  const toggleCategory = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
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
      "thumbnails": "🖼️",
      "chrome-update": "🔄",
      "notion-cache": "📝",
      "office-cache": "📎",
    };
    return map[id] ?? "📁";
  };

  return (
    <div style={{ padding: "0 4px" }}>
      <Title level={4} style={{ marginBottom: 16 }}>磁盘清理</Title>

      <Space style={{ marginBottom: 16 }}>
        <Button
          type="primary"
          icon={<ScanOutlined />}
          loading={scanning}
          onClick={onScan}
        >
          扫描
        </Button>
        <Popconfirm
          title="确认清理"
          description={`将清理 ${fmtSize(totalSelected)}，此操作不可撤销`}
          onConfirm={onClean}
          disabled={selected.size === 0}
        >
          <Button
            danger
            icon={<ThunderboltOutlined />}
            loading={cleaning}
            disabled={selected.size === 0}
          >
            清理选中 ({fmtSize(totalSelected)})
          </Button>
        </Popconfirm>
      </Space>

      {categories.length > 0 && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 12 }}>
          {categories.map((cat) => (
            <Card
              key={cat.id}
              size="small"
              hoverable
              style={{
                border: selected.has(cat.id) ? "1px solid #1677ff" : "1px solid #e5e7eb",
                background: selected.has(cat.id) ? "#f0f5ff" : "#fff",
                cursor: "pointer",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                <Checkbox checked={selected.has(cat.id)} onChange={() => toggleCategory(cat.id)} />
                <div style={{ display: "flex", alignItems: "center", gap: 10, flex: 1, minWidth: 0, cursor: "pointer" }} onClick={() => setDetailCat(cat)}>
                  <span style={{ fontSize: 20 }}>{categoryIcon(cat.id)}</span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontWeight: 500 }}>{cat.name}</div>
                    <Text type="secondary" style={{ fontSize: 12 }}>{cat.description}</Text>
                  </div>
                  <div style={{ textAlign: "right" }}>
                    <div style={{ fontWeight: 600, color: "#cf1322" }}>{fmtSize(cat.sizeBytes)}</div>
                    <Text type="secondary" style={{ fontSize: 11 }}>{cat.fileCount} 个文件</Text>
                  </div>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <Modal
        title={detailCat ? `${categoryIcon(detailCat.id)} ${detailCat.name}` : ""}
        open={!!detailCat}
        onCancel={() => setDetailCat(null)}
        footer={null}
        width={600}
      >
        {detailCat && (
          <div>
            {(() => {
              const checked = detailCat.paths.filter((p) => !excludedPaths.has(p.path));
              const checkedSize = checked.reduce((s, p) => s + p.sizeBytes, 0);
              const checkedCount = checked.reduce((s, p) => s + p.fileCount, 0);
              return (
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", background: "#fafafa", borderRadius: 8, padding: "12px 16px", marginBottom: 16 }}>
                  <div>
                    <Text type="secondary" style={{ fontSize: 12 }}>占用空间</Text>
                    <div style={{ fontSize: 20, fontWeight: 600, color: "#cf1322" }}>{fmtSize(checkedSize)}</div>
                  </div>
                  <div style={{ textAlign: "right" }}>
                    <Text type="secondary" style={{ fontSize: 12 }}>文件数量</Text>
                    <div style={{ fontSize: 20, fontWeight: 600 }}>{checkedCount.toLocaleString()}</div>
                  </div>
                </div>
              );
            })()}
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
              <Text type="secondary" style={{ fontSize: 12 }}>扫描路径（取消勾选的路径不会被清理）</Text>
              {(() => {
                const allPaths = detailCat.paths.map((p) => p.path);
                const checkedCount = allPaths.filter((p) => !excludedPaths.has(p)).length;
                const allChecked = checkedCount === allPaths.length;
                return (
                  <Button size="small" type="link" onClick={() => {
                    setExcludedPaths((prev) => {
                      const next = new Set(prev);
                      if (allChecked) {
                        allPaths.forEach((p) => next.add(p));
                      } else {
                        allPaths.forEach((p) => next.delete(p));
                      }
                      return next;
                    });
                  }}>
                    {allChecked ? "取消全选" : "全选"}
                  </Button>
                );
              })()}
            </div>
            <div style={{ maxHeight: 300, overflow: "auto", borderRadius: 8, border: "1px solid #f0f0f0" }}>
              {detailCat.paths.map((p, i) => (
                <div key={p.path} style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 12px", borderBottom: i < detailCat.paths.length - 1 ? "1px solid #f5f5f5" : "none" }}>
                  <Checkbox
                    checked={!excludedPaths.has(p.path)}
                    onChange={() => setExcludedPaths((prev) => {
                      const next = new Set(prev);
                      next.has(p.path) ? next.delete(p.path) : next.add(p.path);
                      return next;
                    })}
                  />
                  <span style={{ fontSize: 14 }}>📂</span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      style={{ fontFamily: "monospace", fontSize: 12, wordBreak: "break-all", color: "#1677ff", cursor: "pointer", textDecoration: "underline" }}
                      onClick={() => { void revealItemInDir(p.path).catch(() => {}); }}
                    >{p.path}</div>
                  </div>
                  <div style={{ textAlign: "right", whiteSpace: "nowrap", marginLeft: 8 }}>
                    <div style={{ fontSize: 12, fontWeight: 500, color: "#cf1322" }}>{fmtSize(p.sizeBytes)}</div>
                    <div style={{ fontSize: 10, color: "#999" }}>{p.fileCount.toLocaleString()} 文件</div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </Modal>

      {categories.length === 0 && !scanning && (
        <div style={{ textAlign: "center", padding: 40, color: "#999" }}>
          点击「扫描」开始检测可清理的文件
        </div>
      )}
    </div>
  );
}
