import { describe, expect, it } from "vitest";
import {
  nextVerticalScrollTop,
  routePanelWheel,
  wheelDeltaToPixels,
} from "./panelWheel";

describe("AI 会话面板滚轮路由", () => {
  it("内容可滚动时按滚轮增量计算面板位置", () => {
    expect(nextVerticalScrollTop({
      scrollTop: 120,
      scrollHeight: 1_000,
      clientHeight: 400,
    }, 180)).toBe(300);
  });

  it("在上下边界返回空，让调用方继续寻找外层滚动容器", () => {
    expect(nextVerticalScrollTop({
      scrollTop: 0,
      scrollHeight: 1_000,
      clientHeight: 400,
    }, -120)).toBeNull();

    expect(nextVerticalScrollTop({
      scrollTop: 600,
      scrollHeight: 1_000,
      clientHeight: 400,
    }, 120)).toBeNull();
  });

  it("内容不足一屏时不错误吞掉外层滚轮", () => {
    expect(nextVerticalScrollTop({
      scrollTop: 0,
      scrollHeight: 320,
      clientHeight: 400,
    }, 120)).toBeNull();
  });

  it("将行和页单位转换为稳定的像素增量", () => {
    expect(wheelDeltaToPixels(3, 1, 18, 500)).toBe(54);
    expect(wheelDeltaToPixels(1, 2, 18, 500)).toBe(500);
    expect(wheelDeltaToPixels(42, 0, 18, 500)).toBe(42);
  });

  it("鼠标停在标题或按钮区域时仍滚动对应内容列表", () => {
    class FakeElement {
      parentElement: FakeElement | null = null;
      scrollTop = 0;
      scrollHeight = 0;
      clientHeight = 0;
      overflowY = "visible";
    }
    const previousHTMLElement = Object.getOwnPropertyDescriptor(globalThis, "HTMLElement");
    const previousElement = Object.getOwnPropertyDescriptor(globalThis, "Element");
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
    Object.defineProperties(globalThis, {
      HTMLElement: { configurable: true, value: FakeElement },
      Element: { configurable: true, value: FakeElement },
      window: {
        configurable: true,
        value: {
          getComputedStyle: (element: FakeElement) => ({
            lineHeight: "16px",
            overflowY: element.overflowY,
          }),
        },
      },
      document: {
        configurable: true,
        value: { scrollingElement: null },
      },
    });

    try {
      const region = new FakeElement();
      const preferred = new FakeElement();
      preferred.parentElement = region;
      preferred.scrollTop = 100;
      preferred.scrollHeight = 1_000;
      preferred.clientHeight = 400;
      preferred.overflowY = "auto";

      for (const targetName of ["标题", "按钮"]) {
        const target = new FakeElement();
        target.parentElement = region;
        let prevented = false;
        const event = {
          target,
          defaultPrevented: false,
          cancelable: true,
          ctrlKey: false,
          shiftKey: false,
          deltaX: 0,
          deltaY: 120,
          deltaMode: 0,
          preventDefault: () => { prevented = true; },
        } as unknown as WheelEvent;

        expect(routePanelWheel(
          region as unknown as HTMLElement,
          event,
          preferred as unknown as HTMLElement,
        ), targetName).toBe(true);
        expect(prevented, targetName).toBe(true);
      }
      expect(preferred.scrollTop).toBe(340);
    } finally {
      for (const [key, descriptor] of [
        ["HTMLElement", previousHTMLElement],
        ["Element", previousElement],
        ["window", previousWindow],
        ["document", previousDocument],
      ] as const) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor);
        else delete (globalThis as Record<string, unknown>)[key];
      }
    }
  });
});
