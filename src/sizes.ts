// Folder sizes in the local pane (backend: sizes.rs, ADR-022). Whenever the local pane lists a
// folder it asks for sizes; the backend answers from its index and streams the rest as
// "dir-sizes" events while it scans. Σ forces a full scan (including folders an automatic scan
// skips, like Documents before you've opened it).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { local } from "./panes";
import { app } from "./state";
import type { DirSizes } from "./types";
import { fail, fmtSize, plural } from "./util";

/** Asks for the sizes of the local pane's folder. `manual` = the Σ button. */
export function requestSizes(manual = false) {
  if (!manual && !app.settings.general.auto_sizes) return;
  const dirs = local.entries.filter((e) => e.dir).map((e) => e.key);
  invoke<DirSizes>("folder_sizes", { path: local.cwd, dirs, auto: !manual }).then(apply, fail);
}

export function stopSizes() {
  invoke("stop_sizes").catch(() => {});
}

function apply(d: DirSizes) {
  if (d.folder !== local.cwd) return;
  // Something was added/removed/renamed here (Finder, a download…): re-list; sizes come back
  // from the index right after (paneHooks.listed → requestSizes).
  if (d.relist) return void local.load(true);
  const byKey = new Map(local.entries.map((e) => [e.key, e]));
  for (const s of d.sizes) {
    const e = byKey.get(s.path);
    if (!e) continue;
    e.size = s.bytes;
    e.calc = true;
    e.partial = s.skipped > 0;
  }
  const out = local.q(".dirsize");
  if (d.total) {
    const ge = d.total.skipped ? "≥ " : "";
    out.textContent = `${ge}${fmtSize(d.total.bytes)} · ${plural(d.total.files, "file")}`;
    out.title = d.total.skipped
      ? "Some folders were skipped (other disks, private folders you haven't opened yet, or no permission). Click Σ to scan everything."
      : "";
  } else if (d.scanning) {
    out.textContent = "Scanning…";
  }
  local.el.classList.toggle("scanning", d.scanning);
  local.sizesSettled = !d.scanning && !!d.total;
  // Sorting by size reorders rows: only once the numbers settle.
  if (!d.scanning && local.sortBy === "size") local.render();
  else local.repaintRows();
}

export function setupSizes() {
  listen<DirSizes>("dir-sizes", (ev) => apply(ev.payload));
}
