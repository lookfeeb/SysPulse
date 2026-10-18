import { useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { Empty, Spin } from "antd";
import type { AiSessionSummary } from "@/bindings";
import type { SessionGroupView } from "./sessionPresentation";
import { buildSessionRows, visibleRowRange } from "./virtualSessionRows";

interface Props {
  groups: SessionGroupView[];
  scrollRef: RefObject<HTMLDivElement | null>;
  loading: boolean;
  emptyText: string;
  emptyAction?: ReactNode;
  resetKey: string;
  renderSession: (summary: AiSessionSummary) => ReactNode;
}

export default function VirtualSessionList({ groups, scrollRef, loading, emptyText, emptyAction, resetKey, renderSession }: Props) {
  const layout = useMemo(() => buildSessionRows(groups), [groups]);
  const [viewport, setViewport] = useState({ top: 0, height: 600 });
  const pendingFocus = useRef<number | null>(null);
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const update = () => setViewport({ top: element.scrollTop, height: element.clientHeight });
    element.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(element);
    update();
    return () => { element.removeEventListener("scroll", update); observer.disconnect(); };
  }, [scrollRef]);
  useLayoutEffect(() => {
    pendingFocus.current = null;
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    setViewport((value) => ({ ...value, top: 0 }));
  }, [resetKey, scrollRef]);
  const top = Math.min(viewport.top, Math.max(0, layout.height - viewport.height));
  const range = visibleRowRange(layout.rows, top, viewport.height);
  useLayoutEffect(() => {
    if (pendingFocus.current !== null) {
      scrollRef.current?.querySelector<HTMLButtonElement>(`[data-row-index="${pendingFocus.current}"] .aim-ses-select`)?.focus({ preventScroll: true });
      pendingFocus.current = null;
    }
  }, [viewport, scrollRef]);
  return (
    <div className="aim-ses-scroll" ref={scrollRef} aria-label="会话列表" onKeyDown={(event) => {
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
      if (!(event.target as HTMLElement).closest(".aim-ses-select")) return;
      const element = (event.target as HTMLElement).closest<HTMLElement>("[data-row-index]");
      if (!element || !scrollRef.current) return;
      const current = Number(element.dataset.rowIndex);
      const step = event.key === "ArrowUp" || event.key === "End" ? -1 : 1;
      let next = event.key === "Home" ? 0 : event.key === "End" ? layout.rows.length - 1 : current + step;
      while (next >= 0 && next < layout.rows.length && !layout.rows[next].session) next += step;
      if (next < 0 || next >= layout.rows.length) return;
      event.preventDefault();
      pendingFocus.current = next;
      const row = layout.rows[next];
      const target = row.top < viewport.top ? row.top
        : row.top + row.height > viewport.top + viewport.height ? row.top + row.height - viewport.height : viewport.top;
      scrollRef.current.scrollTop = target;
      setViewport({ ...viewport, top: target });
    }}>
      <Spin spinning={loading}>
        {layout.rows.length === 0 ? <div className="aim-empty"><Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={emptyText}>{emptyAction}</Empty></div> : (
          <div style={{ height: layout.height, position: "relative" }}>
            {layout.rows.slice(range.start, range.end).map((row, offset) => (
              <div key={row.key} data-row-index={range.start + offset}
                style={{ position: "absolute", top: row.top, height: row.height, left: 0, right: 0 }}>
                {row.group ? <div className="aim-ses-group-hd"><span>{row.group.label}</span><i aria-hidden="true" /><b>{row.group.sessions.length}</b></div>
                  : renderSession(row.session)}
              </div>
            ))}
          </div>
        )}
      </Spin>
    </div>
  );
}
