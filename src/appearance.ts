// Per-device look & layout preferences (localStorage): theme, text size, pane split.
// Theme/size are applied as `data-theme` and `--fs` on <html>; styles.css does the rest.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { $, store } from "./util";

export type Theme = "system" | "light" | "dark";
const SIZES = ["13", "14", "16", "18"];

export const appearance = {
  get theme(): Theme {
    const t = store.get("theme");
    return t === "light" || t === "dark" ? t : "system";
  },
  get size(): string {
    const s = store.get("textSize") ?? "";
    return SIZES.includes(s) ? s : "14";
  },
};

export function applyAppearance() {
  const root = document.documentElement;
  const t = appearance.theme;
  if (t === "system") root.removeAttribute("data-theme");
  else root.dataset.theme = t;
  root.style.setProperty("--fs", `${appearance.size}px`);
  // Native title bar / dialogs follow the chosen theme too (null = follow the OS).
  getCurrentWindow()
    .setTheme(t === "system" ? null : t)
    .catch(() => {});
}

export function setTheme(t: Theme) {
  store.set("theme", t);
  applyAppearance();
}

export function setTextSize(px: string) {
  store.set("textSize", px);
  applyAppearance();
}

// ---------- pane splitter ----------

const MIN = 20; // % of the window, per pane

function setSplit(pct: number) {
  pct = Math.max(MIN, Math.min(100 - MIN, pct));
  $<HTMLElement>("panes").style.setProperty("--left", `${pct}%`);
  return pct;
}

export function setupSplitter() {
  const panes = $<HTMLElement>("panes");
  const splitter = panes.querySelector<HTMLElement>(".splitter")!;
  const saved = Number(store.get("split"));
  if (saved) setSplit(saved);

  splitter.addEventListener("mousedown", (ev) => {
    ev.preventDefault();
    document.body.classList.add("resizing");
    const rect = panes.getBoundingClientRect();
    let pct = 50;
    const move = (e: MouseEvent) => (pct = setSplit(((e.clientX - rect.left) / rect.width) * 100));
    const up = () => {
      document.body.classList.remove("resizing");
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      store.set("split", pct.toFixed(1));
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });
  splitter.addEventListener("dblclick", () => store.set("split", String(setSplit(50))));
}
