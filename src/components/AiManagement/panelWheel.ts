const SCROLL_EPSILON = 0.5;
const DEFAULT_LINE_HEIGHT = 16;

export interface VerticalScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

interface ScrollDestination {
  element: HTMLElement;
  scrollTop: number;
}

export function nextVerticalScrollTop(
  metrics: VerticalScrollMetrics,
  deltaY: number,
): number | null {
  if (!Number.isFinite(deltaY) || Math.abs(deltaY) <= SCROLL_EPSILON) return null;

  const maximum = Math.max(0, metrics.scrollHeight - metrics.clientHeight);
  if (maximum <= SCROLL_EPSILON) return null;

  const current = Math.min(maximum, Math.max(0, metrics.scrollTop));
  const next = Math.min(maximum, Math.max(0, current + deltaY));
  return Math.abs(next - current) > SCROLL_EPSILON ? next : null;
}

export function wheelDeltaToPixels(
  deltaY: number,
  deltaMode: number,
  lineHeight: number,
  pageHeight: number,
): number {
  if (deltaMode === 1) return deltaY * lineHeight;
  if (deltaMode === 2) return deltaY * pageHeight;
  return deltaY;
}

function lineHeightInPixels(element: HTMLElement): number {
  const parsed = Number.parseFloat(window.getComputedStyle(element).lineHeight);
  return Number.isFinite(parsed) ? parsed : DEFAULT_LINE_HEIGHT;
}

function isVerticalScrollContainer(element: HTMLElement): boolean {
  const overflowY = window.getComputedStyle(element).overflowY;
  return overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay";
}

function destinationFor(element: HTMLElement, deltaY: number): ScrollDestination | null {
  const scrollTop = nextVerticalScrollTop(element, deltaY);
  return scrollTop === null ? null : { element, scrollTop };
}

function eventTargetElement(target: EventTarget | null): HTMLElement | null {
  if (target instanceof HTMLElement) return target;
  return target instanceof Element ? target.parentElement : null;
}

function findScrollDestination(
  region: HTMLElement,
  preferredScroller: HTMLElement,
  eventTarget: EventTarget | null,
  deltaY: number,
): ScrollDestination | null {
  let current = eventTargetElement(eventTarget);

  while (current && current !== region) {
    if (isVerticalScrollContainer(current)) {
      const nestedDestination = destinationFor(current, deltaY);
      if (nestedDestination) return nestedDestination;
    }
    current = current.parentElement;
  }

  // 鼠标位于固定标题、筛选器或操作区时，仍优先滚动该区域对应的内容列表。
  const preferredDestination = destinationFor(preferredScroller, deltaY);
  if (preferredDestination) return preferredDestination;

  if (preferredScroller !== region && isVerticalScrollContainer(region)) {
    const regionDestination = destinationFor(region, deltaY);
    if (regionDestination) return regionDestination;
  }

  current = region.parentElement;
  while (current) {
    if (isVerticalScrollContainer(current)) {
      const ancestorDestination = destinationFor(current, deltaY);
      if (ancestorDestination) return ancestorDestination;
    }
    current = current.parentElement;
  }

  const documentScroller = document.scrollingElement;
  if (documentScroller instanceof HTMLElement) {
    return destinationFor(documentScroller, deltaY);
  }
  return null;
}

export function routePanelWheel(
  region: HTMLElement,
  event: WheelEvent,
  preferredScroller: HTMLElement = region,
): boolean {
  if (
    event.defaultPrevented
    || !event.cancelable
    || event.ctrlKey
    || event.shiftKey
    || Math.abs(event.deltaY) <= Math.abs(event.deltaX)
  ) {
    return false;
  }

  const deltaY = wheelDeltaToPixels(
    event.deltaY,
    event.deltaMode,
    lineHeightInPixels(preferredScroller),
    Math.max(preferredScroller.clientHeight, 1),
  );
  const destination = findScrollDestination(region, preferredScroller, event.target, deltaY);
  if (!destination) return false;

  event.preventDefault();
  destination.element.scrollTop = destination.scrollTop;
  return true;
}

export function bindPanelWheelRouting(
  region: HTMLElement,
  preferredScroller: HTMLElement = region,
): () => void {
  const onWheel = (event: WheelEvent) => {
    routePanelWheel(region, event, preferredScroller);
  };

  region.addEventListener("wheel", onWheel, { capture: true, passive: false });
  return () => region.removeEventListener("wheel", onWheel, true);
}
