// The local pane, the remote views, and which pane has keyboard focus (ADR-029).
//
// The right column holds 1–3 remote views stacked top/bottom, each a `Pane` cloned from
// #remote-tpl with its own account, bucket and folder. `remote` is the view that last had focus:
// local → remote actions (F5, palette, Compare) target it. The layout (accounts, buckets, folders,
// heights) is saved per device in localStorage ("views").

import { hydrateIcons } from "./icons";
import { Pane, paneHooks } from "./pane";
import { $, store } from "./util";

export const MAX_VIEWS = 3;

export const local = new Pane("local", $("pane-local"));
/** Remote views, top to bottom. */
export const remotes: Pane[] = [];
/** The remote view that last had focus. Set by `addView` at boot, so it's always defined after. */
export let remote: Pane = undefined as unknown as Pane;
/** Pane that receives keyboard shortcuts (highlighted header). */
export let active: Pane = local;

export type ViewState = { account: string; bucket: string; prefix: string; grow?: number };

const grow = new WeakMap<Pane, number>();

/** The pane whose element contains `el` (drag & drop, menus). */
export function paneOf(el: Element | null): Pane | null {
  const box = el?.closest<HTMLElement>(".pane");
  return [local, ...remotes].find((p) => p.el === box) ?? null;
}

/** Adds a remote view below `after` (or at the bottom). The caller loads it. */
export function addView(state: ViewState, after?: Pane): Pane {
  const el = $<HTMLTemplateElement>("remote-tpl").content.firstElementChild!.cloneNode(true) as HTMLElement;
  const p = new Pane("remote", el);
  p.account = state.account;
  p.bucket = state.bucket;
  p.cwd = state.prefix;
  p.other = local;
  grow.set(p, state.grow ?? 1);
  const at = after ? remotes.indexOf(after) + 1 : remotes.length;
  const box = $("remotes");
  const handle = () => {
    const h = document.createElement("div");
    h.className = "hsplit";
    h.title = "Drag to resize · double-click to even out";
    return h;
  };
  if (at === 0) {
    box.prepend(el);
    if (remotes.length) el.after(handle());
  } else {
    remotes[at - 1].el.after(handle(), el);
  }
  remotes.splice(at, 0, p);
  hydrateIcons(el);
  paneHooks.created(p);
  if (!remote) remote = p;
  applyLayout();
  return p;
}

/** Closes a remote view (never the last one). */
export function closeView(p: Pane) {
  if (remotes.length < 2) return;
  const i = remotes.indexOf(p);
  if (i < 0) return;
  // Remove the view and the handle next to it.
  const h = (p.el.previousElementSibling ?? p.el.nextElementSibling) as HTMLElement | null;
  if (h?.classList.contains("hsplit")) h.remove();
  p.el.remove();
  remotes.splice(i, 1);
  if (remote === p) remote = remotes[Math.min(i, remotes.length - 1)];
  if (active === p) switchTo(remote);
  applyLayout();
  saveViews();
}

/** Per-view state of the layout: buttons, heights, which view is the local → remote target. */
function applyLayout() {
  const many = remotes.length > 1;
  // Grow factors are shares of the column. Keep them summing to the view count: a lone view left
  // with grow < 1 (after split, resize, close) would leave a gap under it.
  const sum = remotes.reduce((n, v) => n + (grow.get(v) ?? 1), 0) || 1;
  for (const v of remotes) grow.set(v, ((grow.get(v) ?? 1) * remotes.length) / sum);
  for (const v of remotes) {
    v.el.style.flexGrow = String(grow.get(v) ?? 1);
    v.el.classList.toggle("current", many && v === remote);
    v.q<HTMLButtonElement>(".close-view").hidden = !many;
    v.q<HTMLButtonElement>(".split-btn").disabled = remotes.length >= MAX_VIEWS;
  }
  local.other = remote;
}

export function saveViews() {
  const views: ViewState[] = remotes.map((v) => ({
    account: v.account,
    bucket: v.bucket,
    prefix: v.cwd,
    grow: Math.round((grow.get(v) ?? 1) * 1000) / 1000,
  }));
  store.set("views", JSON.stringify(views));
}

export function savedViews(): ViewState[] {
  try {
    const v = JSON.parse(store.get("views") ?? "[]");
    return Array.isArray(v) ? v.slice(0, MAX_VIEWS) : [];
  } catch {
    return [];
  }
}

export function setActive(p: Pane) {
  active = p;
  if (p.isRemote && p !== remote) {
    remote = p;
    applyLayout();
  }
  for (const q of [local, ...remotes]) q.el.classList.toggle("active", q === p);
}

/** Moves keyboard focus to `p`, selecting the first row if nothing is selected. */
export function switchTo(p: Pane) {
  setActive(p);
  p.list.focus();
  if (p.cursor() < 0) p.selectIndex(0);
}

/** Tab / Shift-Tab: local → views top to bottom → local. */
export function cyclePane(back = false) {
  const all = [local, ...remotes];
  const i = all.indexOf(active);
  switchTo(all[(i + (back ? all.length - 1 : 1)) % all.length]);
}

/** Drag a handle between two views to share their height. */
function setupHandles() {
  $("remotes").addEventListener("mousedown", (ev) => {
    const h = (ev.target as HTMLElement).closest<HTMLElement>(".hsplit");
    if (!h) return;
    ev.preventDefault();
    const above = remotes.find((v) => v.el === h.previousElementSibling);
    const below = remotes.find((v) => v.el === h.nextElementSibling);
    if (!above || !below) return;
    const a0 = above.el.getBoundingClientRect().height;
    const b0 = below.el.getBoundingClientRect().height;
    const g = (grow.get(above) ?? 1) + (grow.get(below) ?? 1);
    const y0 = ev.clientY;
    document.body.classList.add("resizing-v");
    const move = (e: MouseEvent) => {
      const a = Math.max(120, Math.min(a0 + b0 - 120, a0 + e.clientY - y0));
      grow.set(above, (g * a) / (a0 + b0));
      grow.set(below, (g * (a0 + b0 - a)) / (a0 + b0));
      applyLayout();
    };
    const up = () => {
      document.body.classList.remove("resizing-v");
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      saveViews();
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });
  $("remotes").addEventListener("dblclick", (ev) => {
    if (!(ev.target as HTMLElement).closest(".hsplit")) return;
    for (const v of remotes) grow.set(v, 1);
    applyLayout();
    saveViews();
  });
}

paneHooks.moved = () => saveViews();
setupHandles();
