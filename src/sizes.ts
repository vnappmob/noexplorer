// Folder sizes in the local pane (backend: sizes.rs, ADR-022). Whenever the local pane lists a
// folder it asks for sizes; the backend answers from its index and streams the rest as
// "dir-sizes" events while it scans. Σ forces a full scan (including folders an automatic scan
// skips, like Documents before you've opened it). Also draws the storage bar under the pane:
// this folder vs the rest of the disk, like macOS System Settings → Storage.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { local } from "./panes";
import { app } from "./state";
import type { DirSizes, Totals, VolumeInfo } from "./types";
import { fail, fmtSize, plural } from "./util";

/** Asks for the sizes of the local pane's folder. `manual` = the Σ button. */
export function requestSizes(manual = false) {
  if (!manual) refreshDisk();
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
  folderTotal = d.total;
  scanning = d.scanning;
  drawDisk();
  if (d.total) {
    out.textContent = `${fmtSize(d.total.bytes)} · ${plural(d.total.files, "file")}`;
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

// ---------- storage bar ----------

let disk: VolumeInfo | null = null;
let diskFor = "";
let folderTotal: Totals | null = null;
let scanning = false;

/** Reads the disk under the local folder (cheap: one statfs / resource lookup). */
function refreshDisk() {
  const at = local.cwd;
  if (at !== diskFor) {
    folderTotal = null;
    scanning = false;
  }
  diskFor = at;
  invoke<VolumeInfo>("volume_info", { path: at }).then(
    (v) => {
      if (local.cwd !== at) return;
      disk = v;
      drawDisk();
    },
    () => {
      disk = null;
      drawDisk();
    },
  );
}

function drawDisk() {
  const el = local.q(".disk");
  const show = !!disk && disk.total > 0;
  el.hidden = !show;
  local.el.classList.toggle("has-disk", show);
  if (!disk || !show) return;
  const { total, available } = disk;
  const used = Math.max(0, total - available);
  // Apparent sizes can exceed what's allocated (clones, sparse files): never draw past "used".
  const here = folderTotal ? Math.min(folderTotal.bytes, used) : 0;
  const pct = (n: number) => `${((n / total) * 100).toFixed(2)}%`;
  local.q(".disk-name").textContent = disk.name || "Disk";
  local.q(".disk-free").textContent = `${fmtSize(available)} available of ${fmtSize(total)}`;
  const segHere = local.q(".seg.here");
  segHere.style.width = pct(here);
  segHere.classList.toggle("pending", !folderTotal && scanning);
  local.q(".seg.other").style.width = pct(used - here);
  const name = local.crumbs.at(-1)?.name || local.cwd;
  const lgHere = local.q(".lg-here");
  lgHere.hidden = !folderTotal && !scanning;
  lgHere.querySelector("span")!.textContent = folderTotal
    ? `${name} ${fmtSize(folderTotal.bytes)}`
    : `${name} · scanning…`;
  local.q(".lg-other").querySelector("span")!.textContent = `${folderTotal ? "Other" : "Used"} ${fmtSize(used - here)}`;
  local.q(".lg-free").querySelector("span")!.textContent = `Available ${fmtSize(available)}`;
  local
    .q(".disk-bar")
    .setAttribute(
      "aria-label",
      `${disk.name}: ${fmtSize(used)} used of ${fmtSize(total)}${folderTotal ? `, ${name} ${fmtSize(folderTotal.bytes)}` : ""}`,
    );
}

export function setupSizes() {
  listen<DirSizes>("dir-sizes", (ev) => apply(ev.payload));
}
