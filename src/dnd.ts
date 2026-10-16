// Drag & drop.
//
// 1. In-app drag between panes uses plain mouse events, NOT HTML5 drag & drop: on Windows,
//    Tauri's native file-drop handler (needed for #2) disables HTML5 DnD inside the webview.
//    See ADR-003.
// 2. Files dragged in from Finder / Explorer arrive via Tauri's `onDragDropEvent` with
//    absolute paths; dropping them on the remote pane uploads them.
//
// Drop targets: a folder row, a breadcrumb, or anywhere else in the pane (= its current folder).

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { transfer } from "./actions";
import type { Pane } from "./pane";
import { local, paneOf } from "./panes";
import { activeProfile } from "./state";
import type { Item } from "./types";
import { $, plural } from "./util";

type DropTarget = { pane: Pane; dir: string; el: Element | null };

function dropTargetAt(x: number, y: number): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  const paneEl = el?.closest<HTMLElement>(".pane");
  if (!paneEl) return null;
  const pane = paneOf(paneEl);
  if (!pane) return null;
  const row = el!.closest<HTMLElement>(".row[data-dir]");
  if (row) return { pane, dir: row.dataset.dir!, el: row };
  const crumb = el!.closest<HTMLElement>(".crumb[data-p]");
  if (crumb) return { pane, dir: crumb.dataset.p!, el: crumb };
  return { pane, dir: pane.cwd, el: null };
}

let highlighted: Element[] = [];
function highlight(t: DropTarget | null) {
  highlighted.forEach((e) => e.classList.remove("drop-target", "drop-pane"));
  highlighted = [];
  if (!t) return;
  t.pane.el.classList.add("drop-pane");
  highlighted.push(t.pane.el);
  if (t.el) {
    t.el.classList.add("drop-target");
    highlighted.push(t.el);
  }
}

// ---------- 1. in-app drag ----------

let drag: {
  from: Pane;
  items: Item[];
  x: number;
  y: number;
  /** Becomes true once the mouse moved past the threshold. */
  active: boolean;
  /** Row to single-select on a plain click (no drag) on an already-selected row; -1 = none. */
  collapseTo: number;
  target: DropTarget | null;
} | null = null;

const DRAG_THRESHOLD = 5;

/** Called from the row mousedown handler (main.ts) to arm a potential drag. */
export function armDrag(from: Pane, x: number, y: number, collapseTo: number) {
  const items = from.selectedItems();
  if (items.length) drag = { from, items, x, y, active: false, collapseTo, target: null };
}

function endDrag() {
  document.body.classList.remove("dragging");
  $("ghost").hidden = true;
  highlight(null);
  drag = null;
}

function setupInternalDrag() {
  document.addEventListener("mousemove", (ev) => {
    if (!drag) return;
    if (!drag.active) {
      if (Math.hypot(ev.clientX - drag.x, ev.clientY - drag.y) < DRAG_THRESHOLD) return;
      drag.active = true;
      document.body.classList.add("dragging");
      const n = drag.items.length;
      $("ghost").textContent = n === 1 ? `${drag.items[0].dir ? "📁" : "📄"} ${drag.items[0].name}` : plural(n, "item");
      $("ghost").hidden = false;
    }
    const g = $("ghost");
    g.style.left = `${ev.clientX + 14}px`;
    g.style.top = `${ev.clientY + 10}px`;
    const t = dropTargetAt(ev.clientX, ev.clientY);
    drag.target = t && t.pane !== drag.from ? t : null; // only cross-pane drops
    highlight(drag.target);
  });
  document.addEventListener("mouseup", () => {
    if (!drag) return;
    const d = drag;
    endDrag();
    if (d.active && d.target) transfer(d.from, d.items, d.target.dir, d.target.pane);
    else if (!d.active && d.collapseTo >= 0) d.from.selectIndex(d.collapseTo);
  });
  window.addEventListener("blur", endDrag);
}

// ---------- 2. drops from the OS ----------

async function setupOsDrop() {
  await getCurrentWebview().onDragDropEvent((ev) => {
    const p = ev.payload;
    const overlay = $("drop");
    if (p.type === "leave") {
      overlay.hidden = true;
      highlight(null);
      return;
    }
    // Positions are physical pixels; elementFromPoint wants CSS pixels.
    const dpr = window.devicePixelRatio || 1;
    const t = dropTargetAt(p.position.x / dpr, p.position.y / dpr);
    const ok = !!t?.pane.isRemote && !!t.pane.profile() && !!t.pane.bucket;
    if (p.type === "drop") {
      overlay.hidden = true;
      highlight(null);
      if (ok)
        transfer(
          local,
          p.paths.map((path) => ({ key: path, name: "", size: 0, modified: null, dir: false })),
          t!.dir,
          t!.pane,
        );
      return;
    }
    // enter / over
    highlight(ok ? t : null);
    $("drop-text").textContent = ok
      ? `Drop to upload to ${t!.pane.profile()!.name} · ${t!.pane.bucket}/${t!.dir}`
      : activeProfile()
        ? "Drop on the remote pane (right) to upload"
        : "Add a remote account first";
    overlay.hidden = false;
  });
}

export async function setupDnd() {
  setupInternalDrag();
  await setupOsDrop();
}
