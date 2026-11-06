import { App } from "antd";
import { HolderOutlined } from "@ant-design/icons";
import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { createPortal } from "react-dom";
import type { OverlayItem } from "@/bindings";
import { overlayItemOptions } from "@/utils/overlayItems";
import { findOrderSlot, moveOrderItem } from "@/utils/overlayOrder";
import "./OverlayOrderEditor.css";

interface DragSession {
  item: OverlayItem;
  original: OverlayItem[];
  kind: "pointer" | "keyboard";
  active: boolean;
  cleanup?: () => void;
}
interface Preview { x: number; y: number; width: number; height: number; outside: boolean }
const labelOf = (item: OverlayItem) => overlayItemOptions.find((option) => option.value === item)?.label ?? item;

export function OverlayOrderEditor({ value, disabled = false, onCommit }: {
  value: OverlayItem[];
  disabled?: boolean;
  onCommit: (items: OverlayItem[]) => Promise<void>;
}) {
  const { message } = App.useApp();
  const helpId = useId();
  const gridRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef(value);
  const valueRef = useRef(value);
  const commitRef = useRef(onCommit);
  valueRef.current = value;
  commitRef.current = onCommit;
  const sessionRef = useRef<DragSession | null>(null);
  const savingRef = useRef(false);
  const positionsRef = useRef(new Map<OverlayItem, DOMRect>());
  const [items, setItems] = useState(value);
  const [active, setActive] = useState<{ item: OverlayItem; kind: DragSession["kind"] } | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [status, setStatus] = useState("");

  const updateItems = (next: OverlayItem[]) => {
    draftRef.current = next;
    setItems(next);
  };
  const slots = () => Array.from(gridRef.current?.querySelectorAll<HTMLElement>("[data-order-item]") ?? []);
  const focusItem = (item: OverlayItem) => slots().find((slot) => slot.dataset.orderItem === item)?.querySelector("button")?.focus({ preventScroll: true });

  const finish = (commit: boolean) => {
    const session = sessionRef.current;
    if (!session) return;
    sessionRef.current = null;
    session.cleanup?.();
    setActive(null);
    setPreview(null);
    const changed = draftRef.current.join("|") !== session.original.join("|");
    if (!commit || !session.active) {
      updateItems(valueRef.current);
      if (session.active) setStatus("已取消排序");
    } else if (changed) {
      savingRef.current = true;
      setStatus("正在保存顺序…");
      void commitRef.current([...draftRef.current]).then(() => {
        setStatus("显示顺序已保存");
      }).catch((error: unknown) => {
        updateItems(valueRef.current);
        setStatus("保存失败，已恢复原顺序");
        void message.error(`显示顺序保存失败：${error instanceof Error ? error.message : String(error)}`);
      }).finally(() => { savingRef.current = false; });
    } else {
      setStatus("顺序未改变");
    }
    focusItem(session.item);
  };

  // Compare contents rather than array identity: parent renders while saving must
  // not replace the optimistic order with the still-unacknowledged old config.
  const valueKey = value.join("|");
  useEffect(() => {
    const session = sessionRef.current;
    sessionRef.current = null;
    session?.cleanup?.();
    setActive(null);
    setPreview(null);
    draftRef.current = valueRef.current;
    setItems(valueRef.current);
  }, [valueKey]);

  useEffect(() => () => {
    const session = sessionRef.current;
    sessionRef.current = null;
    session?.cleanup?.();
  }, []);

  useLayoutEffect(() => {
    const next = new Map<OverlayItem, DOMRect>();
    for (const slot of slots()) {
      const item = slot.dataset.orderItem as OverlayItem;
      const rect = slot.getBoundingClientRect();
      const before = positionsRef.current.get(item);
      const chip = slot.firstElementChild as HTMLElement;
      if (before && active?.item !== item && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
        const x = before.left - rect.left;
        const y = before.top - rect.top;
        if (x || y) {
          chip.getAnimations().forEach((animation) => animation.cancel());
          chip.animate([{ transform: `translate(${x}px, ${y}px)` }, { transform: "translate(0, 0)" }], { duration: 160, easing: "ease-out" });
        }
      }
      next.set(item, rect);
    }
    positionsRef.current = next;
  }, [items, active?.item]);

  const moveTo = (session: DragSession, index: number) => {
    const from = draftRef.current.indexOf(session.item);
    const next = moveOrderItem(draftRef.current, from, index);
    if (next !== draftRef.current) {
      updateItems(next);
      setStatus(`${labelOf(session.item)}，第 ${index + 1} 项，共 ${next.length} 项`);
    }
  };

  const beginPointer = (event: ReactPointerEvent<HTMLButtonElement>, item: OverlayItem) => {
    if (disabled || savingRef.current || sessionRef.current || value.length < 2 || event.button !== 0 || !event.isPrimary) return;
    const grid = gridRef.current!;
    positionsRef.current = new Map(slots().map((slot) => [slot.dataset.orderItem as OverlayItem, slot.getBoundingClientRect()]));
    const rect = event.currentTarget.getBoundingClientRect();
    const { pointerId, clientX: startX, clientY: startY } = event;
    const offsetX = startX - rect.left;
    const offsetY = startY - rect.top;
    const session: DragSession = { item, original: [...draftRef.current], kind: "pointer", active: false };
    sessionRef.current = session;
    event.currentTarget.focus({ preventScroll: true });
    event.preventDefault();
    let frame = 0;
    let lastEvent: PointerEvent | null = null;
    let outside = false;
    const update = (point: PointerEvent) => {
      if (!session.active) {
        if (Math.hypot(point.clientX - startX, point.clientY - startY) < 6) return;
        session.active = true;
        setActive({ item, kind: "pointer" });
        setStatus("松手保存 · Esc 取消");
      }
      const bounds = grid.getBoundingClientRect();
      outside = point.clientX < bounds.left - 12 || point.clientX > bounds.right + 12 || point.clientY < bounds.top - 12 || point.clientY > bounds.bottom + 12;
      const x = point.clientX - offsetX;
      const y = point.clientY - offsetY;
      setPreview({ x, y, width: rect.width, height: rect.height, outside });
      if (!outside) moveTo(session, findOrderSlot(slots().map((slot) => slot.getBoundingClientRect()), x + rect.width / 2, y + rect.height / 2, draftRef.current.indexOf(item)));
    };
    const move = (point: PointerEvent) => {
      if (point.pointerId !== pointerId) return;
      point.preventDefault();
      lastEvent = point;
      if (!frame) frame = requestAnimationFrame(() => { frame = 0; if (lastEvent) update(lastEvent); });
    };
    const up = (point: PointerEvent) => {
      if (point.pointerId !== pointerId) return;
      if (frame) cancelAnimationFrame(frame);
      // Do not lose the final pointer position when releasing between frames.
      update(point);
      finish(!outside);
    };
    const cancelPointer = (point: PointerEvent) => { if (point.pointerId === pointerId) finish(false); };
    const cancel = () => finish(false);
    const escape = (key: globalThis.KeyboardEvent) => { if (key.key === "Escape") { key.preventDefault(); finish(false); } };
    window.addEventListener("pointermove", move, { passive: false });
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancelPointer);
    window.addEventListener("blur", cancel);
    window.addEventListener("resize", cancel);
    window.addEventListener("scroll", cancel, true);
    window.addEventListener("keydown", escape);
    grid.addEventListener("lostpointercapture", cancelPointer);
    session.cleanup = () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancelPointer);
      window.removeEventListener("blur", cancel);
      window.removeEventListener("resize", cancel);
      window.removeEventListener("scroll", cancel, true);
      window.removeEventListener("keydown", escape);
      grid.removeEventListener("lostpointercapture", cancelPointer);
      if (grid.hasPointerCapture(pointerId)) grid.releasePointerCapture(pointerId);
    };
    // Capture on the stationary container; keyed cards can move between slots.
    try { grid.setPointerCapture(pointerId); } catch { /* Window listeners also handle capture unavailable. */ }
  };

  const keyboard = (event: KeyboardEvent<HTMLButtonElement>, item: OverlayItem) => {
    if (disabled || savingRef.current || items.length < 2) return;
    const session = sessionRef.current;
    if (session?.kind === "pointer") return;
    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      if (event.repeat) return;
      if (session) finish(true);
      else {
        positionsRef.current = new Map(slots().map((slot) => [slot.dataset.orderItem as OverlayItem, slot.getBoundingClientRect()]));
        const cancel = () => finish(false);
        window.addEventListener("pointerdown", cancel);
        window.addEventListener("blur", cancel);
        sessionRef.current = { item, original: [...draftRef.current], kind: "keyboard", active: true, cleanup: () => {
          window.removeEventListener("pointerdown", cancel);
          window.removeEventListener("blur", cancel);
        } };
        setActive({ item, kind: "keyboard" });
        setStatus(`已选中${labelOf(item)}，方向键移动，空格确认，Esc 取消`);
      }
    } else if (session && event.key === "Escape") {
      event.preventDefault();
      finish(false);
    } else if (session && event.key === "Tab") {
      finish(false);
    } else if (session && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const current = draftRef.current.indexOf(item);
      let next = current;
      if (event.key === "ArrowLeft") next--;
      if (event.key === "ArrowRight") next++;
      if (event.key === "Home") next = 0;
      if (event.key === "End") next = items.length - 1;
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        const rects = slots().map((slot) => slot.getBoundingClientRect());
        const rect = rects[current];
        next = findOrderSlot(rects, rect.left + rect.width / 2, rect.top + rect.height / 2 + (event.key === "ArrowDown" ? 1 : -1) * (rect.height + 10), current);
      }
      moveTo(session, Math.max(0, Math.min(items.length - 1, next)));
      requestAnimationFrame(() => focusItem(item));
    }
  };

  const content = (item: OverlayItem, index: number) => <>
    <HolderOutlined className="overlay-order-handle" aria-hidden />
    <span className="overlay-order-number" aria-hidden>{index + 1}</span>
    <span className="overlay-order-label">{labelOf(item)}</span>
  </>;

  if (!items.length) return <div className="overlay-order-empty">尚未选择显示项</div>;
  return <>
    <div ref={gridRef} className="overlay-order-grid" data-order-container role="list" aria-label="悬浮窗显示顺序" aria-busy={disabled}>
      {items.map((item, index) => <div key={item} data-order-item={item} role="listitem"
        className={active?.item === item ? `overlay-order-${active.kind === "pointer" ? "placeholder" : "keyboard"}` : undefined}>
        <button type="button" className="overlay-order-chip" aria-disabled={disabled}
          aria-label={`${labelOf(item)}，第 ${index + 1} 项，共 ${items.length} 项`} aria-describedby={helpId} aria-pressed={active?.item === item}
          onPointerDown={(event) => beginPointer(event, item)} onKeyDown={(event) => keyboard(event, item)}>
          {content(item, index)}
        </button>
      </div>)}
    </div>
    <div id={helpId} className="overlay-order-help" role="status" aria-live="polite">
      {preview?.outside ? "移回排序区域继续拖动，松手取消" : active ? status || "松手保存 · Esc 取消" : disabled ? "正在保存顺序…" : status || "拖动后松手保存；也可按空格选中，用方向键调整，Esc 取消"}
    </div>
    {active?.kind === "pointer" && preview && createPortal(<div className="overlay-order-chip overlay-order-ghost" aria-hidden data-outside={preview.outside}
      style={{ width: preview.width, height: preview.height, transform: `translate3d(${preview.x}px, ${preview.y}px, 0)` }}>
      {content(active.item, items.indexOf(active.item))}
    </div>, document.body)}
  </>;
}
