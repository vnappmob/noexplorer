// Search below the current folder (search.rs): Enter in the filter box searches every subfolder,
// on this computer or the remote. Results stream into the pane as they're found (each row says
// which folder it's in), a bar above the list shows progress, and Stop / ✕ / Esc / opening a
// folder end it. The pane's own listing comes back on any normal load (Pane.load ends a search).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import type { Pane } from "./pane";
import { app } from "./state";
import type { Item } from "./types";
import { esc, fail } from "./util";

type SearchEvent = {
  id: string;
  hits: Item[];
  done: boolean;
  dirs: number;
  files: number;
  skipped: number;
  capped: boolean;
  error: string | null;
};

const panes = new Map<string, Pane>();
const count = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
const sepRe = /[\\/]/;

/** "a/b" for a result at <root>/a/b/name ("" right in the folder searched). */
function whereOf(root: string, key: string) {
  const rel = key
    .slice(root.length)
    .replace(/^[\\/]+/, "")
    .replace(/[\\/]+$/, "");
  const parts = rel.split(sepRe);
  parts.pop();
  return parts.join("/");
}

export async function startSearch(p: Pane, raw: string) {
  const query = raw.trim();
  if (!query) return;
  if (p.search) p.endSearch();
  const root = p.cwd;
  let id: string;
  try {
    id = p.isRemote
      ? await invoke<string>("search_remote", { ...p.where(), prefix: root, query })
      : await invoke<string>("search_local", { path: root, query, hidden: app.showHidden });
  } catch (e) {
    return fail(e);
  }
  p.search = { id, query, root, running: true, dirs: 0, files: 0, skipped: 0, capped: false, error: null };
  panes.set(id, p);
  p.entries = [];
  p.selected.clear();
  p.list.scrollTop = 0;
  drawBar(p);
  p.render();
}

function drawBar(p: Pane) {
  const s = p.search;
  let bar = p.el.querySelector<HTMLElement>(".search-bar");
  if (!s) return void bar?.remove();
  if (!bar) {
    bar = document.createElement("div");
    bar.className = "search-bar";
    p.q(".pathbar").after(bar);
    bar.addEventListener("click", (ev) => {
      const a = (ev.target as HTMLElement).closest<HTMLElement>("[data-sa]")?.dataset.sa;
      if (a === "stop" && p.search) invoke("cancel_search", { id: p.search.id });
      if (a === "close") p.load();
    });
  }
  const found = p.entries.length;
  const where = s.root ? s.root.split(sepRe).filter(Boolean).pop() : p.isRemote ? p.bucket : "/";
  const stats = [
    s.running ? `${count(found, "match", "matches")} so far` : count(found, "match", "matches"),
    p.isRemote ? `${count(s.files, "file", "files")} checked` : `${count(s.dirs, "folder", "folders")} searched`,
    s.skipped ? `${s.skipped} skipped` : "",
    s.capped ? "stopped at the first 10,000: add words to narrow it" : "",
  ].filter(Boolean);
  bar.innerHTML = `${s.running ? '<span class="spin" aria-hidden="true"></span>' : icon("search")}
    <span class="sb-text"><b>“${esc(s.query)}”</b> in ${esc(where || "")} and below <small>${esc(stats.join(" · "))}</small></span>
    ${s.running ? `<button type="button" data-sa="stop">Stop</button>` : ""}
    <button type="button" class="ib" data-sa="close" title="Back to the folder (Esc)" aria-label="Close search">${icon("x")}</button>`;
  if (s.skipped && !p.isRemote)
    bar.title =
      "Skipped: other disks, unreadable folders, and private folders (Desktop, Documents, Downloads…) you haven't opened in NoExplorer yet. Open one once to include it.";
}

/** Opens the folder a result is in, with the result selected. */
export async function showInFolder(p: Pane, e: Item) {
  const key = e.key.replace(/[\\/]+$/, "");
  const cut = Math.max(key.lastIndexOf("/"), key.lastIndexOf("\\"));
  const parent = p.isRemote ? (cut >= 0 ? key.slice(0, cut + 1) : "") : key.slice(0, Math.max(cut, 1));
  await p.go(parent);
  const i = p.view.findIndex((x) => x.key.replace(/[\\/]+$/, "") === key);
  if (i >= 0) p.selectIndex(i);
}

export function setupSearch() {
  let pendingRender = 0;
  listen<SearchEvent>("search", (ev) => {
    const r = ev.payload;
    const p = panes.get(r.id);
    if (!p || p.search?.id !== r.id) return;
    const s = p.search;
    for (const h of r.hits) p.entries.push({ ...h, where: whereOf(s.root, h.key) });
    Object.assign(s, { dirs: r.dirs, files: r.files, skipped: r.skipped, capped: r.capped, error: r.error });
    if (r.done) {
      s.running = false;
      panes.delete(r.id);
      if (r.error) fail(r.error);
    }
    drawBar(p);
    // Batches arrive every 100 ms; one render per frame-ish keeps 10,000 rows smooth.
    if (r.hits.length || r.done) {
      clearTimeout(pendingRender);
      pendingRender = window.setTimeout(() => p.render(), r.done ? 0 : 60);
    }
  });
}
