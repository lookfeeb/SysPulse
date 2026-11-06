import { App, Card, Space, Typography } from "antd";
import { HolderOutlined, EyeOutlined } from "@ant-design/icons";
import { useState } from "react";
import { OverlayOrderEditor } from "@/components/OverlayOrderEditor";
import { useConfigStore } from "@/stores/configStore";
import type { OverlayConfig, OverlayItem } from "@/bindings";
import {
  overlayItemOptions,
  toConfigItems,
  toDisplayItems,
} from "@/utils/overlayItems";

const { Text } = Typography;

export function OverlaySettingsCard() {
  const { message } = App.useApp();
  const [saving, setSaving] = useState(false);
  const config = useConfigStore((s) => s.config);
  const patch = useConfigStore((s) => s.patch);

  if (!config) return null;
  const ov = config.overlay;

  const apply = async (changes: Partial<OverlayConfig>) => {
    setSaving(true);
    try {
      const latest = useConfigStore.getState().config?.overlay ?? ov;
      await patch({ overlay: { ...latest, ...changes } });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Space vertical size={16} style={{ width: "100%" }}>
      {/* 显示项选择 */}
      <Card
        size="small"
        title={
          <Space>
            <EyeOutlined style={{ color: "#3388cc" }} />
            <span style={{ fontWeight: 600, fontSize: 13, color: "#374151" }}>
              显示项
            </span>
            <Text type="secondary" style={{ fontSize: 12, fontWeight: 400 }}>
              选择悬浮窗显示哪些指标
            </Text>
          </Space>
        }
        style={{ borderRadius: 10 }}
        styles={{ body: { padding: "14px 16px" } }}
      >
        <ItemsPicker
          selected={toDisplayItems(ov.items)}
          disabled={saving}
          onChange={(items) => void apply({ items: toConfigItems(items) }).catch((error: unknown) => {
            void message.error(error instanceof Error ? error.message : String(error));
          })}
        />
      </Card>

      {/* 显示顺序 */}
      <Card
        size="small"
        title={
          <Space>
            <HolderOutlined style={{ color: "#3388cc" }} />
            <span style={{ fontWeight: 600, fontSize: 13, color: "#374151" }}>
              显示顺序
            </span>
            <Text type="secondary" style={{ fontSize: 12, fontWeight: 400 }}>
              拖动调整悬浮窗中的排列顺序
            </Text>
          </Space>
        }
        style={{ borderRadius: 10 }}
        styles={{ body: { padding: "14px 16px" } }}
      >
        <OverlayOrderEditor
          value={toDisplayItems(ov.items)}
          disabled={saving}
          onCommit={(items) => apply({ items: toConfigItems(items) })}
        />
      </Card>

    </Space>
  );
}

/* ------------------------------------------------------------------ */
/* 显示项选择（用可点击的 chip，选中高亮）                              */
/* ------------------------------------------------------------------ */

function ItemsPicker({
  selected,
  disabled: saving,
  onChange,
}: {
  selected: OverlayItem[];
  disabled: boolean;
  onChange: (items: OverlayItem[]) => void;
}) {
  const MAX_ITEMS = 6;
  const selectedSet = new Set(selected);
  const atLimit = selected.length >= MAX_ITEMS;

  const toggle = (item: OverlayItem) => {
    if (selectedSet.has(item)) {
      onChange(selected.filter((i) => i !== item));
    } else if (!atLimit) {
      onChange([...selected, item]);
    }
  };

  return (
    <div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
        {overlayItemOptions.map((opt) => {
          const active = selectedSet.has(opt.value);
          const disabled = saving || (!active && atLimit);
          return (
            <button
              key={opt.value}
              type="button"
              onClick={() => toggle(opt.value)}
              disabled={disabled}
              style={{
                border: `1px solid ${active ? "#3388cc" : "#e5e7eb"}`,
                background: active ? "#eff6ff" : disabled ? "#f9fafb" : "#ffffff",
                color: active ? "#1d4ed8" : disabled ? "#d1d5db" : "#6b7280",
                fontSize: 12,
                fontWeight: active ? 600 : 400,
                padding: "5px 12px",
                borderRadius: 20,
                cursor: disabled ? "not-allowed" : "pointer",
                transition: "all 120ms ease",
                userSelect: "none",
                lineHeight: 1.4,
                display: "inline-flex",
                alignItems: "center",
                gap: 6,
                opacity: disabled ? 0.5 : 1,
              }}
              onMouseEnter={(e) => {
                if (!active && !disabled) e.currentTarget.style.borderColor = "#93c5fd";
              }}
              onMouseLeave={(e) => {
                if (!active && !disabled) e.currentTarget.style.borderColor = "#e5e7eb";
              }}
            >
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: active ? "#3388cc" : "#d1d5db",
                  display: "inline-block",
                }}
              />
              {opt.label}
            </button>
          );
        })}
      </div>
      <div style={{ marginTop: 8, fontSize: 11, color: atLimit ? "#f97316" : "#9ca3af" }}>
        已选 {selected.length}/{MAX_ITEMS}
        {atLimit && " — 已达上限，取消已选项后才能添加新的"}
      </div>
    </div>
  );
}
