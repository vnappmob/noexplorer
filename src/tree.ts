// Folder tree (experimental "folder_tree"): the ↑ button opens a tree of the folders this pane has
// seen, to jump anywhere in one click. Every listing records the folder's subfolders (folders only,
// in memory, per local disk or remote account + bucket), so the tree grows as you browse.
//
// The tree shows the path you're on, expanded, with the siblings seen at each level; other folders
// that were listed get a ▸ to open them in place. The box at the top filters every known folder.

import { icon } from "./icons";
import type { Pane } from "./pane";
import { exp } from "./state";
import { esc } from "./util";

type Folder = { name: string; path: string };

/** Folders recorded per root, at most this many listings (oldest forgotten first). */
const MAX = 5000;
const cache = new Map<string, Map<string, Folder[]>>();

const trim = (s: string) => s.replace(/[\\/]+$/, "");
const rootOf = (p: Pane) => (p.isRemote ? `${p.account}|${p.bucket}` : "local");
/** A path as the pane navigates to it: remote prefixes end in "/" (the bucket root is ""). */
const target = (p: Pane, path: string) => (p.isRemote && path ? `${trim(path)}/` : path);

function folders(p: Pane) {
  let m = cache.get(rootOf(p));
  if (!m) cache.set(rootOf(p), (m = new Map()));
  return m;
}

/** After every listing: remember this folder's subfolders. */
export function record(p: Pane) {
  const m = folders(p);
  const k = trim(p.cwd);
  m.delete(k);
  m.set(
    k,
    p.entries.filter((e) => e.dir).map((e) => ({ name: e.name, path: trim(e.key) })),
  );
  if (m.size > MAX) m.delete(m.keys().next().value!);
}

// ---------- the dropdown ----------

let pane: Pane | null = null;
let anchor: HTMLElement | null = null;
const open = new Set<string>();

function pop() {
  let el = document.getElementById("tree-pop");
  if (!el) {
    el = document.createElement("div");
    el.id = "tree-pop";
    el.className = "tree-pop";
    el.hidden = true;
    el.innerHTML = `<label class="menu-search">${icon("search")}<input id="tree-q" placeholder="Find a folder you've opened…" spellcheck="false" autocomplete="off" /></label><div id="tree-rows" role="tree"></div>`;
    document.body.append(el);
    el.querySelector("#tree-q")!.addEventListener("input", draw);
    el.addEventListener("click", onClick);
    el.addEventListener("keydown", onKey);
  }
  return el;
}

/** The chain from the top of the path to the current folder: the pane's breadcrumbs. */
const chainOf = (p: Pane): Folder[] => p.crumbs.map((c) => ({ name: c.name, path: trim(c.path) }));

function children(p: Pane, path: string, chainNext?: Folder): Folder[] {
  const kids = [...(folders(p).get(path) ?? [])];
  if (chainNext && !kids.some((k) => k.path === chainNext.path)) kids.push(chainNext);
  return kids.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
}

function row(p: Pane, n: Folder, depth: number, expanded: boolean, expandable: boolean) {
  const here = n.path === trim(p.cwd);
  const tog = expandable
    ? `<span class="tw" data-toggle="${esc(n.path)}">${icon(expanded ? "chevDown" : "chevRight")}</span>`
    : `<span class="tw"></span>`;
  return `<button type="button" class="tree-row${here ? " here" : ""}" role="treeitem" aria-expanded="${expanded}" data-go="${esc(n.path)}" style="--d:${depth}">${tog}${icon("folder")}<span class="tn">${esc(n.name)}</span></button>`;
}

function drawTree(p: Pane): string {
  const chain = chainOf(p);
  if (!chain.length) return "";
  const onChain = new Map(chain.map((c, i) => [c.path, i]));
  const out: string[] = [];
  const walk = (n: Folder, depth: number) => {
    const i = onChain.get(n.path);
    const next = i !== undefined ? chain[i + 1] : undefined;
    const kids = children(p, n.path, next);
    const expanded = i !== undefined || open.has(n.path);
    out.push(row(p, n, depth, expanded && kids.length > 0, kids.length > 0));
    if (expanded) for (const k of kids) walk(k, depth + 1);
  };
  // The top of the chain, with its siblings when its parent was listed too.
  const top = chain[0];
  const parentPath = p.isRemote ? null : trim(top.path.slice(0, top.path.length - top.name.length)) || "/";
  const tops = parentPath && folders(p).has(parentPath) ? children(p, parentPath, top) : [top];
  for (const t of tops) walk(t, 0);
  return out.join("");
}

function drawFound(p: Pane, q: string): string {
  const s = q.toLowerCase();
  const seen = new Map<string, Folder>();
  for (const kids of folders(p).values())
    for (const k of kids) if (k.name.toLowerCase().includes(s)) seen.set(k.path, k);
  const hits = [...seen.values()].sort((a, b) => a.path.length - b.path.length).slice(0, 100);
  if (!hits.length) return `<p class="tree-empty">No folder you've opened matches “${esc(q)}”.</p>`;
  return hits
    .map(
      (n) =>
        `<button type="button" class="tree-row found" data-go="${esc(n.path)}"><span class="tw"></span>${icon("folder")}<span class="tn">${esc(n.name)}<small>${esc(n.path)}</small></span></button>`,
    )
    .join("");
}

function draw() {
  if (!pane) return;
  const q = (document.getElementById("tree-q") as HTMLInputElement).value.trim();
  document.getElementById("tree-rows")!.innerHTML = q ? drawFound(pane, q) : drawTree(pane);
}

export function closeTree() {
  const el = document.getElementById("tree-pop");
  if (el) el.hidden = true;
  anchor?.setAttribute("aria-expanded", "false");
  pane = anchor = null;
}

export function openTree(p: Pane, button: HTMLElement) {
  if (pane === p && !pop().hidden) return closeTree();
  pane = p;
  anchor = button;
  const el = pop();
  (el.querySelector("#tree-q") as HTMLInputElement).value = "";
  draw();
  el.hidden = false;
  button.setAttribute("aria-expanded", "true");
  const r = button.getBoundingClientRect();
  el.style.left = `${Math.max(8, Math.min(r.left, innerWidth - el.offsetWidth - 8))}px`;
  el.style.top = `${r.bottom + 6}px`;
  el.style.maxHeight = `${Math.max(160, innerHeight - r.bottom - 24)}px`;
  // The current folder in the middle; typing goes straight to the filter (↓ moves into the tree).
  const here = el.querySelector<HTMLElement>(".tree-row.here");
  if (here) el.scrollTop = here.offsetTop - el.clientHeight / 2;
  (el.querySelector("#tree-q") as HTMLElement).focus({ preventScroll: true });
}

function onClick(ev: MouseEvent) {
  if (!pane) return;
  const t = ev.target as HTMLElement;
  const tog = t.closest<HTMLElement>("[data-toggle]");
  if (tog) {
    const k = tog.dataset.toggle!;
    if (!open.delete(k)) open.add(k);
    return draw();
  }
  const go = t.closest<HTMLElement>("[data-go]")?.dataset.go;
  if (go === undefined) return;
  const p = pane;
  closeTree();
  if (go !== trim(p.cwd)) p.go(target(p, go));
}

function onKey(ev: KeyboardEvent) {
  const rows = [...document.querySelectorAll<HTMLElement>("#tree-rows .tree-row")];
  const i = rows.indexOf(document.activeElement as HTMLElement);
  if (ev.key === "Escape") return (ev.preventDefault(), anchor?.focus(), closeTree());
  if (ev.key === "Enter" && i < 0) return (ev.preventDefault(), rows[0]?.click());
  if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
    ev.preventDefault();
    // From the filter box, ↓ starts at the current folder (or the first match).
    const from = i < 0 ? rows.findIndex((r) => r.classList.contains("here")) : i;
    const to = i < 0 && from >= 0 ? from : from + (ev.key === "ArrowDown" ? 1 : -1);
    rows[Math.max(0, Math.min(rows.length - 1, to))]?.focus();
  } else if ((ev.key === "ArrowRight" || ev.key === "ArrowLeft") && i >= 0) {
    const k = rows[i].dataset.go!;
    if (ev.key === "ArrowRight") open.add(k);
    else open.delete(k);
    ev.preventDefault();
    draw();
    document.querySelector<HTMLElement>(`#tree-rows [data-go="${CSS.escape(k)}"]`)?.focus();
  }
}

/** ↑ opens the tree while the experiment is on (⌘↑ and Backspace still go up). */
export function bindTree(p: Pane) {
  const up = p.q<HTMLButtonElement>(".up");
  const sync = () => {
    const on = exp("folder_tree");
    up.classList.toggle("tree-on", on);
    up.setAttribute("aria-haspopup", on ? "tree" : "false");
    if (on) up.disabled = false;
  };
  up.onclick = () => (exp("folder_tree") ? openTree(p, up) : p.up());
  window.addEventListener("experimental", sync);
  window.addEventListener("settings-loaded", sync);
  sync();
  return sync;
}

document.addEventListener("mousedown", (ev) => {
  const el = document.getElementById("tree-pop");
  if (!el || el.hidden) return;
  const t = ev.target as globalThis.Node;
  if (!el.contains(t) && !anchor?.contains(t)) closeTree();
});
