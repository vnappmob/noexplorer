// Move to… on a remote view: a folder picker instead of typing a path.
// It starts from the folders the ↑ tree (tree.ts) already knows for this account + bucket, with the
// path you're on opened, so most of the time it appears complete without a request. A folder nobody
// has listed yet is listed the moment you open or pick it; only its subfolders are kept, in the same
// cache, so the ↑ tree and the next Move to see them too. Folders you open again are re-listed
// quietly in the background (the cached rows show at once).

import { invoke } from "@tauri-apps/api/core";
import { icon } from "./icons";
import type { Pane } from "./pane";
import { remotes } from "./panes";
import { type Folder, known, remember, rootOf } from "./tree";
import type { Item } from "./types";
import { $, askText, esc, fail, plural } from "./util";

type State = {
  root: string;
  where: { account: string | null; bucket: string };
  bucket: string;
  /** The items' own folders (and everything below them): can't move a folder into itself. */
  blocked: string[];
  /** The folder all items are in now, if they share one ("" = top). */
  from: string | null;
  /** Folders on the way to the current one: shown even when their parent's listing misses them. */
  chain: string[];
  sel: string;
  open: Set<string>;
  loading: Set<string>;
  /** Re-listed during this dialog, so a cached folder is checked once, not on every toggle. */
  fresh: Set<string>;
  errors: Map<string, string>;
};

let s: State | null = null;

const trim = (k: string) => k.replace(/^\/+/, "").replace(/\/+$/, "");
const parentOf = (k: string) => (k.includes("/") ? k.slice(0, k.lastIndexOf("/")) : "");
const nameOf = (k: string) => k.slice(k.lastIndexOf("/") + 1);
/** "", "a", "a/b", … down to `k`. */
function chainOf(k: string) {
  const out = [""];
  let acc = "";
  for (const part of k.split("/").filter(Boolean)) out.push((acc = acc ? `${acc}/${part}` : part));
  return out;
}
const prefix = (k: string) => (k ? `${k}/` : "");
const isBlocked = (st: State, k: string) => st.blocked.some((b) => k === b || k.startsWith(`${b}/`));
const byName = (a: Folder, b: Folder) =>
  a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });

function kidsOf(st: State, path: string): Folder[] | undefined {
  const listed = known(st.root).get(path);
  if (!listed) return undefined;
  const kids = [...listed];
  for (const c of st.chain)
    if (c && parentOf(c) === path && !kids.some((k) => k.path === c)) kids.push({ name: nameOf(c), path: c });
  return kids.sort(byName);
}

/** Lists `path` (only its folders are kept). `quiet`: a re-check of a cached folder, no spinner. */
async function load(st: State, path: string, quiet = false) {
  if (st.loading.has(path)) return;
  st.loading.add(path);
  st.errors.delete(path);
  if (!quiet) draw();
  try {
    const list = await invoke<Item[]>("list", { ...st.where, prefix: prefix(path) });
    remember(st.root, path, list);
    st.fresh.add(path);
  } catch (e) {
    if (!quiet || !known(st.root).has(path)) st.errors.set(path, String(e));
  }
  st.loading.delete(path);
  if (s === st) draw();
}

function expand(st: State, path: string) {
  st.open.add(path);
  if (!known(st.root).has(path)) void load(st, path);
  else if (!st.fresh.has(path)) void load(st, path, true);
}

// ---------- drawing ----------

function rowHtml(st: State, path: string, name: string, depth: number, kids: Folder[] | undefined) {
  const blocked = isBlocked(st, path);
  const expandable = !blocked && (kids === undefined || kids.length > 0);
  const expanded = expandable && st.open.has(path);
  const spin = st.loading.has(path) && !kids;
  const tog = spin
    ? `<span class="tw mv-spin">${icon("refresh")}</span>`
    : expandable
      ? `<span class="tw" data-toggle>${icon(expanded ? "chevDown" : "chevRight")}</span>`
      : `<span class="tw"></span>`;
  const tag = blocked ? "moving" : path === st.from ? "current" : "";
  const cls = `tree-row${path === st.sel ? " sel" : ""}${blocked ? " off" : ""}`;
  return `<button type="button" class="${cls}" role="treeitem" aria-selected="${path === st.sel}"${expandable ? ` aria-expanded="${expanded}"` : ""}${blocked ? ' aria-disabled="true"' : ""} data-path="${esc(path)}" style="--d:${depth}" tabindex="${path === st.sel ? 0 : -1}">${tog}${icon(path ? "folder" : "bucket")}<span class="tn">${esc(name)}</span>${tag ? `<em class="mv-tag">${tag}</em>` : ""}</button>`;
}

function treeHtml(st: State) {
  const out: string[] = [];
  const walk = (path: string, name: string, depth: number) => {
    const kids = kidsOf(st, path);
    out.push(rowHtml(st, path, name, depth, kids));
    if (isBlocked(st, path) || !st.open.has(path)) return;
    const err = st.errors.get(path);
    if (err && !kids)
      out.push(
        `<p class="mv-note err" style="--d:${depth + 1}">${esc(err)} <button type="button" class="link" data-retry="${esc(path)}">Try again</button></p>`,
      );
    else if (!kids) out.push(`<p class="mv-note" style="--d:${depth + 1}">Loading…</p>`);
    else for (const k of kids) walk(k.path, k.name, depth + 1);
  };
  walk("", st.bucket, 0);
  return out.join("");
}

function draw() {
  const st = s;
  if (!st) return;
  const tree = $("mv-tree");
  const hadFocus = tree.contains(document.activeElement);
  const top = tree.scrollTop;
  tree.innerHTML = treeHtml(st);
  tree.scrollTop = top;
  if (hadFocus) tree.querySelector<HTMLElement>(".tree-row.sel")?.focus({ preventScroll: true });
  const ok = $<HTMLButtonElement>("mv-ok");
  ok.disabled = isBlocked(st, st.sel) || st.sel === st.from;
  ok.title = st.sel === st.from ? "The items are already in this folder" : "";
  $<HTMLButtonElement>("mv-new").disabled = isBlocked(st, st.sel);
}

function select(st: State, path: string, opts: { expand?: boolean; syncInput?: boolean; focus?: boolean } = {}) {
  st.sel = path;
  if (opts.syncInput !== false) $<HTMLInputElement>("mv-path").value = prefix(path);
  if (opts.expand && !isBlocked(st, path)) expand(st, path);
  draw();
  const row = $("mv-tree").querySelector<HTMLElement>(`[data-path="${CSS.escape(path)}"]`);
  row?.scrollIntoView({ block: "nearest" });
  if (opts.focus) row?.focus({ preventScroll: true });
}

// ---------- events ----------

function onClick(ev: MouseEvent) {
  const st = s;
  if (!st) return;
  const t = ev.target as HTMLElement;
  const retry = t.closest<HTMLElement>("[data-retry]")?.dataset.retry;
  if (retry !== undefined) return void load(st, retry);
  const row = t.closest<HTMLElement>(".tree-row");
  if (!row || row.getAttribute("aria-disabled") === "true") return;
  const path = row.dataset.path!;
  if (t.closest("[data-toggle]")) {
    if (st.open.has(path)) st.open.delete(path);
    else expand(st, path);
    return draw();
  }
  // Picking a folder also opens it, so its subfolders are one click away.
  select(st, path, { expand: true });
}

function onDblClick(ev: MouseEvent) {
  const row = (ev.target as HTMLElement).closest<HTMLElement>(".tree-row");
  if (!row || (ev.target as HTMLElement).closest("[data-toggle]")) return;
  if (!$<HTMLButtonElement>("mv-ok").disabled) $<HTMLFormElement>("mv-form").requestSubmit($("mv-ok"));
}

function onKey(ev: KeyboardEvent) {
  const st = s;
  if (!st) return;
  const rows = [...$("mv-tree").querySelectorAll<HTMLElement>(".tree-row:not([aria-disabled])")];
  const i = rows.findIndex((r) => r.dataset.path === st.sel);
  const go = (r: HTMLElement | undefined) => r && select(st, r.dataset.path!, { focus: true });
  switch (ev.key) {
    case "ArrowDown":
      go(rows[Math.min(rows.length - 1, i + 1)]);
      break;
    case "ArrowUp":
      go(rows[Math.max(0, i - 1)]);
      break;
    case "ArrowRight":
      if (!st.open.has(st.sel)) (expand(st, st.sel), draw());
      else if (rows[i + 1] && parentOf(rows[i + 1].dataset.path!) === st.sel) go(rows[i + 1]);
      break;
    case "ArrowLeft":
      if (st.open.has(st.sel) && st.sel) (st.open.delete(st.sel), draw());
      else if (st.sel) select(st, parentOf(st.sel), { focus: true });
      break;
    case "Enter":
      ev.preventDefault();
      if (!$<HTMLButtonElement>("mv-ok").disabled) $<HTMLFormElement>("mv-form").requestSubmit($("mv-ok"));
      return;
    default:
      return;
  }
  ev.preventDefault();
}

/** Typing a path picks it in the tree when it's there (and lists its parent when it's not known). */
function onType() {
  const st = s;
  if (!st) return;
  const path = trim($<HTMLInputElement>("mv-path").value);
  // Only folders that exist get opened (a half-typed name must not be listed).
  const exists = (c: string) => !c || !!kidsOf(st, parentOf(c))?.some((k) => k.path === c);
  for (const c of chainOf(path).slice(0, -1)) if (exists(c) && !isBlocked(st, c)) expand(st, c);
  select(st, path, { syncInput: false });
}

async function newFolder() {
  const st = s;
  if (!st || isBlocked(st, st.sel)) return;
  const name = (await askText(`New folder in “${st.sel ? nameOf(st.sel) : st.bucket}”`))?.trim();
  if (!name || s !== st) return;
  try {
    await invoke("mkdir", { ...st.where, prefix: prefix(st.sel), name });
  } catch (e) {
    return fail(e);
  }
  const path = st.sel ? `${st.sel}/${name}` : name;
  const m = known(st.root);
  const kids = m.get(st.sel);
  if (kids && !kids.some((k) => k.path === path)) kids.push({ name, path });
  else if (!kids) m.set(st.sel, [{ name, path }]);
  m.set(path, []); // brand new: nothing inside
  // A view showing the parent folder gets the new folder too.
  for (const v of remotes) if (rootOf(v) === st.root && trim(v.cwd) === st.sel) void v.load(true);
  st.open.add(st.sel);
  select(st, path, { focus: true });
}

let bound = false;
function bind() {
  if (bound) return;
  bound = true;
  const tree = $("mv-tree");
  tree.addEventListener("click", onClick);
  tree.addEventListener("dblclick", onDblClick);
  tree.addEventListener("keydown", onKey);
  $("mv-path").addEventListener("input", onType);
  $("mv-new").addEventListener("click", newFolder);
  $("mv-cancel").addEventListener("click", () => $<HTMLDialogElement>("move-pick").close("cancel"));
}

/** Asks where to move `items` (in remote view `p`). Resolves to the destination prefix ("" = the top,
 * otherwise ending in "/"), or null when cancelled. */
export function pickMoveTarget(p: Pane, items: Item[]): Promise<string | null> {
  bind();
  const parents = new Set(items.map((e) => parentOf(trim(e.key))));
  const here = trim(p.cwd);
  const st: State = {
    root: rootOf(p),
    where: p.where(),
    bucket: p.bucket,
    blocked: items.filter((e) => e.dir).map((e) => trim(e.key)),
    from: parents.size === 1 ? [...parents][0] : null,
    chain: chainOf(here),
    sel: here,
    open: new Set(),
    loading: new Set(),
    fresh: new Set(),
    errors: new Map(),
  };
  s = st;
  // The path you're on starts open; anything on it that was never listed is listed now. What opens
  // by itself isn't re-checked: only folders the user opens are.
  for (const c of st.chain) if (known(st.root).has(c)) st.fresh.add(c);
  for (const c of st.chain) if (!isBlocked(st, c)) expand(st, c);
  $("mv-title").textContent = `Move ${items.length === 1 ? `“${items[0].name}”` : plural(items.length, "item")} to`;
  const d = $<HTMLDialogElement>("move-pick");
  d.returnValue = "";
  d.showModal();
  select(st, isBlocked(st, here) ? "" : here, { focus: true });
  $("mv-tree").querySelector(".tree-row.sel")?.scrollIntoView({ block: "center" });
  return new Promise((resolve) => {
    d.addEventListener(
      "close",
      () => {
        s = null;
        resolve(d.returnValue === "ok" ? prefix(trim($<HTMLInputElement>("mv-path").value)) : null);
      },
      { once: true },
    );
  });
}
