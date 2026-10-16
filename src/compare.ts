// Compare: a local folder against a remote one (backend: compare.rs, ADR-028). Shows what's
// missing on each side, what differs (by size, or by content with "Verify"), and the junk files
// uploads skip on purpose, then offers to upload/download the difference.
//
// Which folders: a folder selected in one pane is compared with the same-named folder inside the
// other pane's current folder; otherwise the two current folders.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { local, remote } from "./panes";
import { activeProfile, app } from "./state";
import type { Diff } from "./types";
import { $, esc, fail, fmtSize, toast } from "./util";

type Tab = "only_local" | "only_remote" | "changed" | "junk" | "symlinks";
const MAX_ROWS = 2000;

const dlg = () => $<HTMLDialogElement>("compare");
let target: { root: string; prefix: string } | null = null;
let diff: Diff | null = null;
let tab: Tab = "only_local";
let runId = "";

const n = (k: number, one: string, many = `${one}s`) => `${k.toLocaleString()} ${k === 1 ? one : many}`;

/** Local root and remote prefix to compare, from the selections/current folders. */
function pickTarget(): { root: string; prefix: string } | null {
  if (!activeProfile() || !app.bucket) return (toast("Add a remote account first", true), null);
  const sep = local.cwd.includes("\\") ? "\\" : "/";
  const join = (dir: string, name: string) => (dir.endsWith(sep) ? dir + name : dir + sep + name);
  const ld = local.selectedItems().filter((e) => e.dir);
  const rd = remote.selectedItems().filter((e) => e.dir);
  if (ld.length === 1) return { root: ld[0].key, prefix: `${remote.cwd}${ld[0].name}/` };
  if (rd.length === 1) return { root: join(local.cwd, rd[0].name), prefix: rd[0].key };
  return { root: local.cwd, prefix: remote.cwd };
}

export function openCompare() {
  target = pickTarget();
  if (!target) return;
  diff = null;
  const where = `${target.prefix || "/"}`;
  $("cmp-where").innerHTML =
    `${icon("drive")}<span title="${esc(target.root)}">${esc(target.root)}</span><b>⇄</b>${icon("cloud")}<span>${esc(activeProfile()!.name)} · ${esc(app.bucket)}/${esc(where)}</span>`;
  render();
  if (!dlg().open) dlg().showModal();
  run();
}

async function run() {
  if (!target) return;
  runId = crypto.randomUUID();
  const id = runId;
  const verify = $<HTMLInputElement>("cmp-verify").checked;
  setBusy("Listing both folders…", 0);
  let un: UnlistenFn | null = null;
  try {
    un = await listen<{ id: string; phase: string; done: number; total: number }>("compare-progress", (e) => {
      const p = e.payload;
      if (p.id !== id) return;
      if (p.phase === "hashing" && p.total)
        setBusy(`Checking contents… ${fmtSize(p.done)} of ${fmtSize(p.total)}`, p.done / p.total);
    });
    diff = await invoke<Diff>("compare", {
      id,
      bucket: app.bucket,
      prefix: target.prefix,
      local: target.root,
      verify,
    });
    if (id !== runId) return;
    tab = diff.only_local.length
      ? "only_local"
      : diff.only_remote.length
        ? "only_remote"
        : diff.changed.length
          ? "changed"
          : "junk";
  } catch (e) {
    if (id === runId && String(e) !== "Cancelled") fail(e);
  } finally {
    un?.();
    if (id === runId) setBusy("", -1);
  }
  render();
}

function setBusy(text: string, frac: number) {
  const busy = frac >= 0;
  $("cmp-progress").hidden = !busy;
  $("cmp-progress-text").textContent = text;
  $<HTMLElement>("cmp-progress-bar").style.width = `${Math.max(2, frac * 100)}%`;
  $<HTMLButtonElement>("cmp-run").disabled = busy;
}

/** One-line verdict, explaining count differences in plain words. */
function verdict(d: Diff) {
  const problems = d.only_local.length + d.only_remote.length + d.changed.length;
  const here = d.local_files + d.junk.length + d.symlinks.length;
  const counts = `This computer: ${n(here, "file")} · Remote: ${n(d.remote_files, "file")}.`;
  const skipped = [
    d.junk.length && `${n(d.junk.length, "system junk file")} (.DS_Store, Thumbs.db…) that uploads skip on purpose`,
    d.symlinks.length && `${n(d.symlinks.length, "symlink")} that aren't followed`,
  ].filter(Boolean);
  const why = skipped.length ? ` The difference includes ${skipped.join(" and ")}.` : "";
  if (!problems) {
    const how = d.same_verified ? `${n(d.same_verified, "file")} verified by checksum` : "matched by name and size";
    return `<b class="ok">✓ Everything matches</b> (${how}). ${esc(counts)}${esc(why)}`;
  }
  const same = d.same_verified + d.same_size;
  const match = same
    ? ` ${n(same, "file")} match${d.same_verified ? ` (${d.same_verified.toLocaleString()} verified by checksum)` : ""}.`
    : "";
  return `<b class="warn">${n(problems, "difference")}</b>.${esc(match)} ${esc(counts)}${esc(why)}`;
}

function render() {
  const d = diff;
  const ok = !!d;
  $("cmp-verdict").innerHTML = d ? verdict(d) : "";
  const tabs: [Tab, string, number][] = d
    ? [
        ["only_local", "Missing on remote", d.only_local.length],
        ["only_remote", "Missing here", d.only_remote.length],
        ["changed", "Different", d.changed.length],
        ["junk", "Skipped junk", d.junk.length],
        ["symlinks", "Symlinks", d.symlinks.length],
      ]
    : [];
  $("cmp-tabs").innerHTML = tabs
    .filter(([k, , c]) => c || k === "only_local" || k === "only_remote" || k === "changed")
    .map(
      ([k, label, c]) =>
        `<button type="button" data-tab="${k}" class="${k === tab ? "on" : ""}">${label} <em>${c.toLocaleString()}</em></button>`,
    )
    .join("");
  const rows: string[] = [];
  if (d) {
    if (tab === "changed")
      for (const c of d.changed.slice(0, MAX_ROWS))
        rows.push(
          `<div class="cmp-row"><span>${esc(c.rel)}</span><small>${
            c.reason === "content"
              ? "same size, different content"
              : `${fmtSize(c.local_size)} here · ${fmtSize(c.remote_size)} remote`
          }</small></div>`,
        );
    else
      for (const f of (d[tab] as { rel: string; size: number }[]).slice(0, MAX_ROWS))
        rows.push(
          `<div class="cmp-row"><span>${esc(f.rel)}</span><small>${tab === "symlinks" ? "symlink" : fmtSize(f.size)}</small></div>`,
        );
    const total = tab === "changed" ? d.changed.length : (d[tab] as unknown[]).length;
    if (total > MAX_ROWS) rows.push(`<div class="cmp-more">…and ${(total - MAX_ROWS).toLocaleString()} more</div>`);
    if (!total) rows.push(`<div class="cmp-more">Nothing here.</div>`);
    if (d.errors.length) rows.push(`<div class="cmp-more err">${esc(d.errors.slice(0, 5).join(" · "))}</div>`);
  }
  $("cmp-list").innerHTML = rows.join("");
  const up = ok ? d!.only_local.length + d!.changed.length : 0;
  const down = ok ? d!.only_remote.length : 0;
  const upBtn = $<HTMLButtonElement>("cmp-up");
  upBtn.disabled = !up;
  upBtn.textContent = up ? `Upload ${n(up, "file")}` : "Upload";
  upBtn.title = "Missing on the remote, and files that differ (the remote copy is replaced)";
  const downBtn = $<HTMLButtonElement>("cmp-down");
  downBtn.disabled = !down;
  downBtn.textContent = down ? `Download ${n(down, "file")}` : "Download";
  downBtn.title = "Files only on the remote (nothing here is overwritten)";
}

async function uploadDiff() {
  if (!diff || !target) return;
  const rels = [...diff.only_local.map((f) => f.rel), ...diff.changed.map((c) => c.rel)];
  try {
    await invoke("upload_rel", { bucket: app.bucket, prefix: target.prefix, root: target.root, rels });
    dlg().close();
  } catch (e) {
    fail(e);
  }
}

async function downloadDiff() {
  if (!diff || !target) return;
  const items = diff.only_remote.map((f) => ({ key: f.rel, size: f.size }));
  try {
    await invoke("download_rel", { bucket: app.bucket, prefix: target.prefix, root: target.root, items });
    dlg().close();
  } catch (e) {
    fail(e);
  }
}

export function setupCompare() {
  $("cmp-run").onclick = run;
  $("cmp-verify").addEventListener("change", run);
  $("cmp-up").onclick = uploadDiff;
  $("cmp-down").onclick = downloadDiff;
  $("cmp-cancel").onclick = () => invoke("cancel", { id: runId });
  $("cmp-close").onclick = () => dlg().close();
  dlg().addEventListener("close", () => invoke("cancel", { id: runId }).catch(() => {}));
  $("cmp-tabs").addEventListener("click", (ev) => {
    const b = (ev.target as HTMLElement).closest<HTMLElement>("[data-tab]");
    if (b) ((tab = b.dataset.tab as Tab), render());
  });
}
