import { Card, Space, Typography } from "antd";
import { HolderOutlined, EyeOutlined } from "@ant-design/icons";
import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { useConfigStore } from "@/stores/configStore";
import type { OverlayConfig, OverlayItem } from "@/bindings";
import {
  overlayItemOptions,
  toConfigItems,
  toDisplayItems,
} from "@/utils/overlayItems";

const { Text } = Typography;

export default function OverlayPage() {
  return <OverlaySettingsCard />;
}

export function OverlaySettingsCard() {
  const config = useConfigStore((s) => s.config);
  const patch = useConfigStore((s) => s.patch);

  if (!config) return null;
  const ov = config.overlay;

  const apply = (changes: Partial<OverlayConfig>) =>
    void patch({ overlay: { ...ov, ...changes } });

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
          onChange={(items) => apply({ items: toConfigItems(items) })}
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
        <OrderEditor
          value={toDisplayItems(ov.items)}
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
  onChange,
}: {
  selected: OverlayItem[];
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
          const disabled = !active && atLimit;
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

/* ------------------------------------------------------------------ */
/* 顺序编辑器（拖拽排序，带手柄图标和序号）                             */
/* ------------------------------------------------------------------ */

function OrderEditor({
  value,
  onCommit,
}: {
  value: OverlayItem[];
  onCommit: (items: OverlayItem[]) => void;
}) {
  const [draftItems, setDraftItems] = useState<OverlayItem[]>(value);
  const draftRef = useRef<OverlayItem[]>(value);
  const dragIndexRef = useRef<number | null>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);

  useEffect(() => {
    if (dragIndexRef.current != null) return;
    draftRef.current = value;
    setDraftItems(value);
  }, [value]);

  useEffect(() => {
    if (dragIndex == null) return;
    const handler = (e: PointerEvent) => endDrag(e);
    window.addEventListener("pointerup", handler);
    window.addEventListener("pointercancel", handler);
    return () => {
      window.removeEventListener("pointerup", handler);
      window.removeEventListener("pointercancel", handler);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dragIndex]);

  const moveItem = (from: number, to: number) => {
    if (from === to) return draftRef.current;
    const next = [...draftRef.current];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    dragIndexRef.current = to;
    draftRef.current = next;
    setDraftItems(next);
    return next;
  };

  const beginDrag = (event: ReactPointerEvent<HTMLDivElement>, index: number) => {
    if (event.button !== 0) return;
    dragIndexRef.current = index;
    setDragIndex(index);
    // Capture pointer so move/up events keep firing even when pointer leaves the element.
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is a progressive enhancement; window pointerup still commits.
    }
    event.preventDefault();
  };

  const endDrag = (event?: PointerEvent | ReactPointerEvent<HTMLDivElement>) => {
    if (dragIndexRef.current == null) return;
    const changed = draftRef.current.join("|") !== value.join("|");
    dragIndexRef.current = null;
    setDragIndex(null);
    // Release capture if we still hold it.
    if (event && "pointerId" in event) {
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (target?.hasPointerCapture((event as PointerEvent).pointerId)) {
        target.releasePointerCapture((event as PointerEvent).pointerId);
      }
    }
    if (changed) onCommit(draftRef.current);
  };

  const dragMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (dragIndexRef.current == null) return;
    const target = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>("[data-order-index]");
    const to = Number(target?.dataset.orderIndex);
    if (Number.isInteger(to)) {
      const from = dragIndexRef.current;
      if (from != null) moveItem(from, to);
    }
  };

  if (draftItems.length === 0) {
    return (
      <div
        style={{
          padding: "24px 12px",
          textAlign: "center",
          background: "#f9fafb",
          borderRadius: 6,
          border: "1px dashed #d1d5db",
        }}
      >
        <Text type="secondary" style={{ fontSize: 12 }}>
          尚未选择显示项
        </Text>
      </div>
    );
  }

  return (
    <div
      data-order-container
      onPointerMove={dragMove}
      onPointerUp={(e) => endDrag(e)}
      onPointerCancel={(e) => endDrag(e)}
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(6, minmax(0, 1fr))",
        gap: 8,
      }}
    >
      {draftItems.map((item, index) => {
        const label =
          overlayItemOptions.find((o) => o.value === item)?.label ?? item;
        const active = dragIndex === index;
        return (
          <div
            key={item}
            data-order-index={index}
            onPointerDown={(e) => beginDrag(e, index)}
            style={{
              background: active ? "#dbeafe" : "#ffffff",
              border: `1px solid ${active ? "#3388cc" : "#e5e7eb"}`,
              borderRadius: 6,
              cursor: active ? "grabbing" : "grab",
              fontSize: 12,
              lineHeight: 1,
              padding: "7px 10px",
              touchAction: "none",
              userSelect: "none",
              display: "flex",
              alignItems: "center",
              gap: 8,
              minWidth: 0,
              boxShadow: active
                ? "0 4px 12px rgba(51, 136, 204, 0.2)"
                : "0 1px 0 rgba(0,0,0,0.02)",
              transition: "box-shadow 120ms ease",
            }}
            title={label}
          >
            <HolderOutlined style={{ color: "#9ca3af", fontSize: 12, pointerEvents: "none" }} />
            <span
              style={{
                background: active ? "#3388cc" : "#f3f4f6",
                color: active ? "#ffffff" : "#6b7280",
                borderRadius: 10,
                minWidth: 18,
                height: 18,
                fontSize: 10,
                fontWeight: 600,
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                padding: "0 5px",
                pointerEvents: "none",
              }}
            >
              {index + 1}
            </span>
            <span
              style={{
                color: "#374151",
                fontWeight: 500,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                pointerEvents: "none",
              }}
            >
              {label}
            </span>
          </div>
        );
      })}
    </div>
  );
}
