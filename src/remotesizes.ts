// Remote folder sizes. Counting a remote folder means listing every object under it, so it's never
// done behind your back by default: Σ counts, and what it counted is remembered (per device, in
// localStorage) and shown dimmed, with its age, the next time the folder is listed.
// Settings → Experimental → "Remote folder sizes" also counts the folders of each listing in the
// background (S3 and Azure, whose flat listings make it one request per 1000 objects), at most
// CAP objects per folder, so one huge folder can't keep the app busy for minutes.

import { invoke } from "@tauri-apps/api/core";
import type { Pane } from "./pane";
import { ago } from "./pane";
import { exp } from "./state";
import type { Item, SizeInfo } from "./types";
import { fmtSize, plural, store } from "./util";

type Known = SizeInfo & { at: number };

const KEY = "remoteSizes";
/** Folders remembered (least recently counted dropped first). */
const MAX = 3000;
/** Automatic sizes recount a folder counted longer ago than this. */
const FRESH = 15 * 60_000;
/** Objects listed at most per folder by automatic sizes (≈ 200 requests on S3). */
const CAP = 200_000;
/** Folders counted at once by automatic sizes, per view. */
const PARALLEL = 2;

let known = load();
/** The folder each view is counting (a newer listing takes over). */
const running = new WeakMap<Pane, string>();

function load(): Map<string, Known> {
  try {
    return new Map(JSON.parse(store.get(KEY) ?? "[]") as [string, Known][]);
  } catch {
    return new Map();
  }
}

let saveTimer = 0;
function save() {
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    while (known.size > MAX) known.delete(known.keys().next().value!);
    store.set(KEY, JSON.stringify([...known]));
  }, 500);
}

const id = (p: Pane, key: string) => `${p.account}\n${p.bucket}\n${key}`;
const autoOn = (p: Pane) => exp("remote_sizes") && ["s3", "azure", undefined].includes(p.profile()?.kind);

/** Σ or an automatic count finished: remember it. */
export function rememberRemote(p: Pane, key: string, s: SizeInfo) {
  const k = id(p, key);
  known.delete(k); // re-inserted last: the map's order is the eviction order
  known.set(k, { bytes: s.bytes, files: s.files, partial: !!s.partial, at: Date.now() });
  save();
}

function fill(e: Item, s: Known, fresh: boolean) {
  e.size = s.bytes;
  e.calc = true;
  e.partial = s.partial;
  e.stale = !fresh;
  e.sizeAt = fresh ? undefined : s.at;
}

/** The folder's total, when every subfolder's size is known: the subfolders plus the files here. */
function drawTotal(p: Pane) {
  const out = p.q(".dirsize");
  const here = known.get(id(p, p.cwd));
  const dirs = p.entries.filter((e) => e.dir);
  if (dirs.length && dirs.every((e) => e.calc)) {
    const bytes = p.entries.reduce((n, e) => n + e.size, 0);
    const files = p.entries.reduce((n, e) => n + (e.dir ? (known.get(id(p, e.key))?.files ?? 0) : 1), 0);
    const approx = dirs.some((e) => e.stale || e.partial);
    out.textContent = `${approx ? "≈ " : ""}${fmtSize(bytes)} · ${plural(files, "file")}`;
    out.title = approx ? "Some folders are remembered or only partly counted. Σ counts everything again." : "";
  } else if (here) {
    out.textContent = `≈ ${fmtSize(here.bytes)} · ${plural(here.files, "file")}`;
    out.title = `Calculated ${ago(here.at)}. Σ calculates it again.`;
  }
}

/** After a remote listing: remembered sizes fill its folders; automatic sizes count the rest. */
export function remoteListed(p: Pane) {
  if (running.get(p) !== id(p, p.cwd)) p.el.classList.remove("scanning");
  if (p.search) return;
  for (const e of p.entries) {
    const s = e.dir && known.get(id(p, e.key));
    if (s) fill(e, s, false);
  }
  drawTotal(p);
  p.repaintRows();
  if (autoOn(p)) countFolders(p);
}

/** Counts the listing's folders that aren't remembered (or are old), a few at a time, until the
 * view moves elsewhere. */
async function countFolders(p: Pane) {
  const where = id(p, p.cwd);
  const todo = p.entries.filter((e) => {
    const s = e.dir && known.get(id(p, e.key));
    return e.dir && (!s || Date.now() - s.at > FRESH);
  });
  if (!todo.length) return;
  running.set(p, where);
  p.el.classList.add("scanning");
  let repaint = 0;
  const worker = async () => {
    for (let e = todo.shift(); e; e = todo.shift()) {
      if (id(p, p.cwd) !== where) return;
      try {
        const s = await invoke<SizeInfo>("remote_size", { ...p.where(), prefix: e.key, cap: CAP });
        rememberRemote(p, e.key, s);
        if (id(p, p.cwd) !== where) return;
        const row = p.entries.find((x) => x.key === e.key);
        if (row) fill(row, known.get(id(p, e.key))!, true);
        clearTimeout(repaint);
        repaint = window.setTimeout(() => (drawTotal(p), p.repaintRows()), 120);
      } catch {
        // A folder that can't be counted (permissions): leave it blank, Σ shows the error.
      }
    }
  };
  await Promise.all(Array.from({ length: PARALLEL }, worker));
  if (running.get(p) !== where) return;
  running.delete(p);
  p.el.classList.remove("scanning");
  if (id(p, p.cwd) === where) {
    drawTotal(p);
    // Sorting by size reorders rows: only once the numbers settle.
    if (p.sortBy === "size") p.render();
    else p.repaintRows();
  }
}
