// Drag & drop.
//
// 1. In-app drag between panes uses plain mouse events, NOT HTML5 drag & drop: on Windows,
//    Tauri's native file-drop handler (needed for #2) disables HTML5 DnD inside the webview.
//    See ADR-003.
// 2. Files dragged in from Finder / Explorer arrive via Tauri's `onDragDropEvent` with
//    absolute paths; dropping them on a remote view uploads them, on the left pane moves them.
// 3. Dragging out to other apps (a browser's upload box, Finder…): when an in-app drag leaves the
//    window with the button down, dragout.rs takes over with a native drag session (macOS only so
//    far, ADR-053). Remote files are downloaded to a temp folder first.
//
// Drop targets: a folder row, a breadcrumb, or anywhere else in the pane (= its current folder).

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { transfer } from "./actions";
import { moveItems } from "./fileops";
import type { Pane } from "./pane";
import { local, paneOf } from "./panes";
import { activeProfile } from "./state";
import type { Item } from "./types";
import { $, fail, fmtSize, plural, toast } from "./util";

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
  /** Left the window: handed to a native drag-out (or preparing one). */
  out: boolean;
  /** Remote items being downloaded for a drag-out, started near the window edge. */
  prep?: Promise<string[]>;
} | null = null;

const DRAG_THRESHOLD = 5;

/** Called from the row mousedown handler (main.ts) to arm a potential drag. */
export function armDrag(from: Pane, x: number, y: number, collapseTo: number) {
  const items = from.selectedItems();
  if (items.length) drag = { from, items, x, y, active: false, collapseTo, target: null, out: false };
}

// ---------- 3. drag out to other apps ----------

const CAN_DRAG_OUT = navigator.userAgent.includes("Mac");
/** Remote files are downloaded before the drag can start; past this, downloading first is better.
 *  Same limit as `dragout.rs` `DRAG_OUT_MAX`, which also counts what's inside folders. */
const DRAG_OUT_MAX = 200 * 1024 * 1024;

/**
 * The native session starts this close to the window edge, still inside it. Started after the
 * pointer had left, it began over the other app, which only noticed the drag on the next mouse
 * move (the "nudge it a bit" lag). Started here, crossing into the other app is that move.
 */
const OUT_EDGE = 8;
/** Remote items start downloading this close to the edge, so they're ready by the time it's crossed. */
const PREP_EDGE = 64;

/** Distance from the pointer to the nearest window edge (negative outside). */
const edgeDistance = (ev: MouseEvent) =>
  Math.min(ev.clientX, ev.clientY, window.innerWidth - 1 - ev.clientX, window.innerHeight - 1 - ev.clientY);

/** Starts downloading a remote drag's items in the background; errors wait for the drag-out. */
function prepare(d: NonNullable<typeof drag>) {
  if (d.prep || !d.from.isRemote) return;
  // Folder sizes are usually unknown here; the backend checks the real total after listing them.
  if (d.items.reduce((s, e) => s + (e.dir ? 0 : e.size), 0) > DRAG_OUT_MAX) return;
  d.prep = invoke<string[]>("drag_out_prepare", { ...d.from.where(), keys: d.items.map((e) => e.key) });
  d.prep.catch(() => {}); // reported by dragOut, if the drag gets there
}

async function startDragOut(paths: string[]) {
  try {
    await invoke("drag_out", { paths });
  } catch (e) {
    // Released before the files were ready: simply no drag.
    if (!/released/i.test(String(e))) fail(e);
  }
}

/** The in-app drag left the window: hand it to the system so other apps can take the files. */
async function dragOut() {
  const d = drag;
  if (!CAN_DRAG_OUT || !d?.active || d.out) return;
  d.out = true;
  d.target = null;
  highlight(null);
  if (!d.from.isRemote) {
    endDrag();
    return startDragOut(d.items.map((e) => e.key));
  }
  prepare(d);
  if (!d.prep) {
    endDrag();
    return toast(`Over ${fmtSize(DRAG_OUT_MAX)}: download first, then drag from the left pane`, true);
  }
  $("ghost").textContent = `Preparing ${d.items.length === 1 ? d.items[0].name : plural(d.items.length, "item")}…`;
  try {
    const paths = await d.prep;
    if (drag !== d) return; // the button came up while downloading
    endDrag();
    await startDragOut(paths);
  } catch (e) {
    if (drag === d) endDrag();
    fail(e);
  }
}

function endDrag() {
  document.body.classList.remove("dragging");
  $("ghost").hidden = true;
  highlight(null);
  drag = null;
}

function setupInternalDrag() {
  document.addEventListener("mousemove", (ev) => {
    if (!drag || drag.out) return;
    if (drag.active && CAN_DRAG_OUT) {
      const edge = edgeDistance(ev);
      if (edge < OUT_EDGE) return void dragOut();
      if (edge < PREP_EDGE) prepare(drag);
    }
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
    // Cross-pane drops copy. Dropping on a folder (row or crumb) in the same pane moves there, but
    // never onto one of the dragged items or into the folder they're already in.
    const sameOk =
      !!t?.el &&
      t.pane === drag.from &&
      t.dir.replace(/[\\/]+$/, "") !== drag.from.cwd.replace(/[\\/]+$/, "") &&
      !drag.items.some((e) => t.dir === e.key || t.dir.startsWith(e.key.replace(/[\\/]*$/, "/")));
    drag.target = t && (t.pane !== drag.from || sameOk) ? t : null;
    highlight(drag.target);
  });
  document.addEventListener("mouseup", () => {
    if (!drag) return;
    const d = drag;
    endDrag();
    if (d.active && d.target && d.target.pane === d.from) moveItems(d.from, d.items, d.target.dir);
    else if (d.active && d.target) transfer(d.from, d.items, d.target.dir, d.target.pane);
    else if (!d.active && d.collapseTo >= 0) d.from.selectIndex(d.collapseTo);
  });
  // WebKit keeps sending moves past the window edge while the button is down; this covers it if not.
  document.addEventListener("mouseleave", () => void dragOut());
  window.addEventListener("blur", () => {
    if (!drag?.out) endDrag();
  });
}

// ---------- 2. drops from the OS ----------
//
// On a remote view: upload. The left pane first opens the folder the files came from (when they
// all share one) with them selected, so the beam starts at real rows and the user sees what left.
// On the left pane: move them into its folder (or the folder row / crumb under the pointer), the
// same never-overwrite, same-disk move as in-app moves (fileops.rs), undoable with ⌘Z.

const trim = (p: string) => p.replace(/[\\/]+$/, "");
const parentDir = (p: string) => {
  const t = trim(p);
  const i = Math.max(t.lastIndexOf("/"), t.lastIndexOf("\\"));
  return i <= 0 ? t.slice(0, i + 1) || "/" : t.slice(0, i);
};
const baseName = (p: string) => trim(p).split(/[\\/]/).pop() ?? p;
const sepOf = (p: string) => (p.includes("\\") && !p.includes("/") ? "\\" : "/");
const joinPath = (dir: string, name: string) => `${trim(dir) || ""}${sepOf(dir)}${name}`;

/** The one folder all `paths` are in, or null when they come from several (Finder's Recents, search). */
function sharedParent(paths: string[]) {
  const dirs = new Set(paths.map(parentDir));
  return dirs.size === 1 ? [...dirs][0] : null;
}

/** Paths that would actually move into `dir` (not the folder itself, nor something already in it). */
const movable = (paths: string[], dir: string) =>
  paths.filter(
    (p) =>
      trim(parentDir(p)) !== trim(dir) &&
      trim(p) !== trim(dir) &&
      !trim(dir).startsWith(trim(p) + "/") &&
      !trim(dir).startsWith(trim(p) + "\\"),
  );

async function uploadDropped(paths: string[], t: DropTarget) {
  const from = sharedParent(paths);
  let items: Item[] = [];
  if (from) {
    if (trim(local.cwd) !== trim(from) || local.search) await local.go(from);
    if (trim(local.cwd) === trim(from)) items = local.selectKeys(paths);
  }
  // Rows not found (hidden files, a folder the system didn't let us list): upload by path anyway.
  const found = new Set(items.map((e) => trim(e.key)));
  for (const p of paths)
    if (!found.has(trim(p))) items.push({ key: p, name: baseName(p), size: 0, modified: null, dir: false });
  transfer(local, items, t.dir, t.pane);
}

async function moveDropped(paths: string[], dir: string) {
  const list = movable(paths, dir);
  if (!list.length) return toast("Already in this folder");
  await moveItems(
    local,
    list.map((p) => ({ key: p, name: baseName(p), size: 0, modified: null, dir: false })),
    dir,
  );
  if (trim(local.cwd) === trim(dir)) local.selectKeys(list.map((p) => joinPath(dir, baseName(p))));
}

/**
 * Tauri types drop positions as physical pixels, but only Windows sends those (wry's WebView2
 * handler, `ScreenToClient`). macOS (`draggingLocation`) and Linux (GTK) send logical points, which
 * already are CSS pixels: dividing them by the Retina ratio halved them, so every drop landed up
 * and to the left (on the left pane, on a folder row above the pointer).
 */
const DROP_SCALE = () => (navigator.userAgent.includes("Windows") ? window.devicePixelRatio || 1 : 1);

async function setupOsDrop() {
  // Paths come with "enter" only; "over" carries just the position.
  let paths: string[] = [];
  await getCurrentWebview().onDragDropEvent((ev) => {
    const p = ev.payload;
    const overlay = $("drop");
    if (p.type === "leave") {
      overlay.hidden = true;
      highlight(null);
      paths = [];
      return;
    }
    if (p.type === "enter" || p.type === "drop") paths = p.paths;
    const t = dropTargetAt(p.position.x / DROP_SCALE(), p.position.y / DROP_SCALE());
    const up = !!t?.pane.isRemote && !!t.pane.profile() && !!t.pane.bucket;
    const mv = !!t && t.pane === local && !!t.dir && movable(paths, t.dir).length > 0;
    if (p.type === "drop") {
      overlay.hidden = true;
      highlight(null);
      const dropped = paths;
      paths = [];
      if (up) uploadDropped(dropped, t!);
      else if (mv) moveDropped(dropped, t!.dir);
      else if (t?.pane === local) toast("Already in this folder");
      return;
    }
    // enter / over
    highlight(up || mv ? t : null);
    const what = paths.length === 1 ? `“${baseName(paths[0])}”` : plural(paths.length, "item");
    $("drop-text").textContent = up
      ? `Drop to upload ${what} to ${t!.pane.profile()!.name} · ${t!.pane.bucket}/${t!.dir}`
      : mv
        ? `Drop to move ${what} to ${baseName(t!.dir) || t!.dir}`
        : t?.pane === local
          ? "Already in this folder"
          : activeProfile()
            ? "Drop on a remote view to upload, or on the left pane to move here"
            : "Add a remote account first";
    overlay.hidden = false;
  });
}

export async function setupDnd() {
  setupInternalDrag();
  await setupOsDrop();
}
