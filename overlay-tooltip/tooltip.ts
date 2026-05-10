import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

interface TooltipPayload {
  title: string;
  lines: string[];
}

function render(payload: TooltipPayload): { width: number; height: number } {
  const root = document.getElementById("tip")!;
  root.replaceChildren();
  if (payload.title) {
    const t = document.createElement("div");
    t.className = "title";
    t.textContent = payload.title;
    root.appendChild(t);
  }
  for (const line of payload.lines) {
    const el = document.createElement("span");
    el.className = "line";
    el.textContent = line;
    root.appendChild(el);
  }
  const rect = root.getBoundingClientRect();
  return {
    width: Math.max(60, Math.ceil(rect.width) + 2),
    height: Math.max(24, Math.ceil(rect.height) + 2),
  };
}

async function main() {
  await listen<TooltipPayload>("overlay-tooltip:show", (e) => {
    const { width, height } = render(e.payload);
    void invoke("overlay_tooltip_fit", { args: { width, height } });
  });
}

void main();
