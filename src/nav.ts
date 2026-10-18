// Tabs, back/forward history and favorites (experimental "tabs").
//
// Each pane has tabs; each tab remembers where it is, its own back/forward stacks and its scroll
// position. A location is a local folder, or account + bucket + prefix for a remote view. History
// is recorded after each successful listing (`paneHooks.visited`), except when the move itself came
// from back/forward or a tab switch. Favorites are saved per device and show up in the places menu,
// the account menu and ⌘K.

import type { Pane } from "./pane";
import { local, remotes } from "./panes";
import { loadBuckets } from "./settings";
import { app, exp } from "./state";
import { $, esc, store, toast } from "./util";
import { icon } from "./icons";

export type Loc = { path: string; account?: string; bucket?: string };
type Tab = { loc: Loc; back: Loc[]; fwd: Loc[]; scroll: number };
type State = { tabs: Tab[]; i: number; last: Loc | null; quiet: boolean; scroll: number | null };

const states = new WeakMap<Pane, State>();

const trim = (s: string) => s.replace(/[\\/]+$/, "");
export const locOf = (p: Pane): Loc =>
  p.isRemote ? { account: p.account, bucket: p.bucket, path: p.cwd } : { path: p.cwd };
const same = (a: Loc | null, b: Loc | null) =>
  !!a && !!b && a.account === b.account && a.bucket === b.bucket && trim(a.path) === trim(b.path);

function state(p: Pane): State {
  let s = states.get(p);
  if (!s) {
    s = { tabs: [{ loc: locOf(p), back: [], fwd: [], scroll: 0 }], i: 0, last: null, quiet: false, scroll: null };
    states.set(p, s);
  }
  return s;
}

/** A location's short name: the folder, or the bucket / account at a root. */
export function locName(l: Loc) {
  const base = trim(l.path).split(/[\\/]/).pop();
  if (base) return base;
  if (l.account !== undefined)
    return l.bucket || app.settings.profiles.find((x) => x.id === l.account)?.name || "Remote";
  return l.path || "/";
}

/** Called after every successful listing (pane.ts `load`). */
export function visited(p: Pane) {
  const s = state(p);
  const loc = locOf(p);
  const tab = s.tabs[s.i];
  if (s.last && !same(s.last, loc) && !s.quiet) {
    tab.back.push(s.last);
    if (tab.back.length > 100) tab.back.shift();
    tab.fwd = [];
  }
  s.quiet = false;
  s.last = loc;
  tab.loc = loc;
  if (s.scroll != null) {
    p.list.scrollTop = s.scroll;
    s.scroll = null;
  }
  render(p);
}

/** Goes to `loc` in `p` without recording it (history and tab moves record themselves). */
async function goQuiet(p: Pane, loc: Loc) {
  const s = state(p);
  if (same(locOf(p), loc)) return render(p);
  s.quiet = true;
  if (!p.isRemote) return p.go(loc.path);
  if (p.account === loc.account && p.bucket === loc.bucket) return p.go(loc.path);
  p.account = loc.account ?? "";
  p.bucket = loc.bucket ?? "";
  p.cwd = loc.path;
  await loadBuckets(p, true);
}

export function back(p: Pane) {
  const s = state(p);
  const tab = s.tabs[s.i];
  const to = tab.back.pop();
  if (!to) return;
  tab.fwd.push(locOf(p));
  goQuiet(p, to);
}

export function forward(p: Pane) {
  const s = state(p);
  const tab = s.tabs[s.i];
  const to = tab.fwd.pop();
  if (!to) return;
  tab.back.push(locOf(p));
  goQuiet(p, to);
}

// ---------- tabs ----------

export function newTab(p: Pane) {
  const s = state(p);
  s.tabs[s.i].scroll = p.list.scrollTop;
  s.tabs.splice(s.i + 1, 0, { loc: locOf(p), back: [], fwd: [], scroll: 0 });
  s.i++;
  render(p);
}

export function switchTab(p: Pane, j: number) {
  const s = state(p);
  if (j === s.i || !s.tabs[j]) return;
  s.tabs[s.i].scroll = p.list.scrollTop;
  s.i = j;
  s.scroll = s.tabs[j].scroll;
  goQuiet(p, s.tabs[j].loc);
  render(p);
}

/** Closes tab `j` (the current one by default). Returns false when it's the pane's only tab. */
export function closeTab(p: Pane, j = state(p).i) {
  const s = state(p);
  if (s.tabs.length < 2) return false;
  s.tabs.splice(j, 1);
  if (j < s.i || s.i >= s.tabs.length) s.i = Math.max(0, s.i - 1);
  if (j === s.i || j === s.i + 1) {
    s.scroll = s.tabs[s.i].scroll;
    goQuiet(p, s.tabs[s.i].loc);
  }
  render(p);
  return true;
}

export const tabCount = (p: Pane) => state(p).tabs.length;
export const cycleTab = (p: Pane, d: number) => {
  const s = state(p);
  switchTab(p, (s.i + d + s.tabs.length) % s.tabs.length);
};

// ---------- favorites ----------

export type Favorite = Loc & { name: string };

export function favorites(): Favorite[] {
  try {
    const v = JSON.parse(store.get("favorites") ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

const isFavorite = (l: Loc) => favorites().some((f) => same(f, l));

export function toggleFavorite(p: Pane) {
  const l = locOf(p);
  const list = favorites();
  const had = list.some((f) => same(f, l));
  store.set("favorites", JSON.stringify(had ? list.filter((f) => !same(f, l)) : [...list, { ...l, name: locName(l) }]));
  toast(had ? `Removed “${locName(l)}” from favorites` : `Added “${locName(l)}” to favorites`);
  render(p);
}

/** Opens a favorite in the right pane: local ones on the left, remote ones in the focused view
 * (or a view already showing that account + bucket). */
export function openFavorite(f: Favorite, view?: Pane) {
  if (f.account === undefined) return local.go(f.path);
  const v = view ?? remotes.find((x) => x.account === f.account && x.bucket === f.bucket) ?? remotes[0];
  return goRecorded(v, f);
}

async function goRecorded(p: Pane, loc: Loc) {
  if (p.account === loc.account && p.bucket === loc.bucket) return p.go(loc.path);
  p.account = loc.account ?? "";
  p.bucket = loc.bucket ?? "";
  p.cwd = loc.path;
  await loadBuckets(p, true);
}

// ---------- rendering ----------

/** Tab strip (2+ tabs), back/forward buttons and the favorite star of `p`. */
export function render(p: Pane) {
  const on = exp("tabs");
  const s = state(p);
  const tab = s.tabs[s.i];
  const back = p.q<HTMLButtonElement>(".back");
  const fwd = p.q<HTMLButtonElement>(".fwd");
  const fav = p.q<HTMLButtonElement>(".fav-btn");
  back.hidden = fwd.hidden = fav.hidden = !on;
  back.disabled = !tab.back.length;
  fwd.disabled = !tab.fwd.length;
  const starred = isFavorite(locOf(p));
  fav.classList.toggle("on", starred);
  fav.title = starred ? "Remove from favorites" : "Add to favorites";
  let bar = p.el.querySelector<HTMLElement>(".tabbar");
  if (!on || s.tabs.length < 2) return void bar?.remove();
  if (!bar) {
    bar = document.createElement("div");
    bar.className = "tabbar";
    bar.setAttribute("role", "tablist");
    p.el.prepend(bar);
    bar.addEventListener("click", (ev) => {
      const t = (ev.target as HTMLElement).closest<HTMLElement>("[data-tab]");
      if (!t) return;
      const j = +t.dataset.tab!;
      if ((ev.target as HTMLElement).closest(".tab-x")) closeTab(p, j);
      else switchTab(p, j);
    });
  }
  bar.innerHTML = s.tabs
    .map(
      (t, j) =>
        `<span class="ptab${j === s.i ? " on" : ""}" role="tab" aria-selected="${j === s.i}" data-tab="${j}" title="${esc(t.loc.path || locName(t.loc))}">
          <span class="ptab-name">${esc(locName(j === s.i ? locOf(p) : t.loc))}</span>
          <button class="tab-x" aria-label="Close tab">${icon("x")}</button></span>`,
    )
    .join("");
}

/** Wires a pane's buttons and mouse back/forward (called for every pane, local and remote). */
export function bindNav(p: Pane) {
  p.q(".back").onclick = () => back(p);
  p.q(".fwd").onclick = () => forward(p);
  p.q(".fav-btn").onclick = () => toggleFavorite(p);
  // Mouse buttons 4 / 5 (back / forward), like a browser.
  p.el.addEventListener("mouseup", (ev) => {
    if (!exp("tabs")) return;
    if (ev.button === 3) (ev.preventDefault(), back(p));
    if (ev.button === 4) (ev.preventDefault(), forward(p));
  });
  window.addEventListener("experimental", () => render(p));
  render(p);
}

export function favoritesMenuItems(kind: "local" | "remote", run: (f: Favorite) => unknown) {
  if (!exp("tabs")) return [];
  const list = favorites().filter((f) => (kind === "local") === (f.account === undefined));
  return list.map((f) => ({
    label: f.name,
    sub: f.account === undefined ? f.path : `${f.bucket}/${f.path}`,
    icon: "star",
    run: () => run(f),
  }));
}

export const tabsFor = (el: Element) => $("panes").contains(el);
