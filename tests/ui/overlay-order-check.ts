import { overlayOrderFixture as fixture } from "./fixtures";
import { useConfigStore } from "../../src/stores/configStore";

/** Run on the General page of ?overlay-order. Events only touch the mock IPC. */
export async function checkOverlayOrder() {
  const original = structuredClone(useConfigStore.getState().config!);
  const results: string[] = [];
  const wait = (ms = 40) => new Promise((resolve) => setTimeout(resolve, ms));
  const order = () => [...document.querySelectorAll<HTMLElement>("[data-order-item]")].map((node) => node.dataset.orderItem!);
  const equal = (actual: unknown, expected: unknown, message: string) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${message}: ${JSON.stringify(actual)}`);
  };
  const reset = async () => {
    fixture.writes.length = 0;
    fixture.failNext = false;
    fixture.delayMs = 80;
    useConfigStore.getState().set(structuredClone(original));
    await wait(180);
  };
  const point = (index: number) => {
    const rect = document.querySelectorAll("[data-order-item]")[index].getBoundingClientRect();
    return { clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 };
  };
  const send = (type: string, position: { clientX: number; clientY: number }, target: EventTarget = window, pointerId = 91) => {
    target.dispatchEvent(new PointerEvent(type, { ...position, bubbles: true, cancelable: true, isPrimary: true, pointerType: "mouse", pointerId, button: 0, buttons: type === "pointerup" ? 0 : 1 }));
  };
  const start = (index = 0) => send("pointerdown", point(index), document.querySelectorAll("[data-order-item] button")[index]);
  const ghost = () => Boolean(document.querySelector(".overlay-order-ghost"));
  await reset();
  const initial = order();

  start();
  send("pointerup", { ...point(0), clientX: point(0).clientX + 2 });
  await wait();
  equal(fixture.writes.length, 0, "click/micro movement must not save");
  equal(ghost(), false, "click must not leave a preview");
  results.push("点击和轻微抖动不触发排序");

  start();
  const destination = point(4);
  send("pointermove", destination);
  await wait();
  const moved = [...initial.slice(1, 5), initial[0], ...initial.slice(5)];
  equal(order(), moved, "fast movement destination");
  equal(document.querySelector<HTMLElement>(".overlay-order-placeholder")?.dataset.orderItem, initial[0], "active item identity");
  equal(ghost(), true, "floating preview");
  send("pointermove", destination);
  await wait();
  equal(order(), moved, "stationary pointer must not oscillate");
  equal(fixture.writes.length, 0, "no writes while dragging");
  send("pointerup", destination, window, 92);
  await wait();
  equal(ghost(), true, "another pointer must not finish this drag");
  fixture.delayMs = 250;
  send("pointerup", destination);
  send("pointerup", destination);
  await wait();
  equal(order(), moved, "optimistic order must remain during saving");
  equal(fixture.writes.length, 1, "one write on release");
  equal([...document.querySelectorAll<HTMLButtonElement>("[data-order-item] button")].every((button) => button.getAttribute("aria-disabled") === "true"), true, "disable conflicting edits during saving");
  await wait(300);
  equal(order(), moved, "saved order");
  results.push("快速拖动、高亮、稳定落点、多指隔离、延迟保存和单次写入");

  for (const method of ["pointercancel", "Escape", "outside", "lostpointercapture", "blur"]) {
    await reset();
    start();
    send("pointermove", point(3));
    await wait();
    if (method === "Escape") window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    else if (method === "outside") send("pointerup", { clientX: 0, clientY: 0 });
    else if (method === "blur") window.dispatchEvent(new Event("blur"));
    else send(method, point(3), method === "lostpointercapture" ? document.querySelector("[data-order-container]")! : window);
    await wait();
    equal(order(), initial, `${method} rollback`);
    equal(fixture.writes.length, 0, `${method} must not save`);
    equal(ghost(), false, `${method} preview cleanup`);
  }
  results.push("Esc、拖出区域、指针取消、捕获丢失和失焦均恢复原顺序");

  await reset();
  fixture.failNext = true;
  start();
  const failedDestination = point(2);
  // Release before the next animation frame must still use the final position.
  send("pointerup", failedDestination);
  await wait(180);
  equal(fixture.writes.length, 1, "fast release commit");
  equal(order(), initial, "failed save rollback");
  equal(document.querySelector(".overlay-order-help")?.textContent, "保存失败，已恢复原顺序", "failure feedback");
  results.push("帧间快速松手不丢落点，保存失败提示并回退");

  await reset();
  start();
  send("pointermove", point(2));
  await wait();
  const replacement = structuredClone(original);
  replacement.overlay.items = ["cpu", "mem"];
  useConfigStore.getState().set(replacement);
  await wait();
  send("pointerup", { clientX: 400, clientY: 200 });
  await wait();
  equal(order(), ["cpu", "mem"], "external selection wins over stale drag");
  equal(fixture.writes.length, 0, "external update must cancel drag");
  equal(ghost(), false, "external update preview cleanup");
  results.push("外部配置变化取消旧拖动，不覆盖新配置");
  await reset();
  return results;
}
