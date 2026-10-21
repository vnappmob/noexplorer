// ⌘K command palette: one search box for commands, accounts, buckets, places and the files in
// both panes' current folders. Entries are built fresh each time it opens, so they always reflect
// the current state; actions reuse the same functions as buttons and shortcuts.

import { canSpeedTest, openSpeedTest } from "./speedtest";
import { canManagePolicy, openPolicy } from "./policy";
import { favorites, openFavorite } from "./favorites";
import { pairs, runPair, savePairFromPanes } from "./backups";
import { invoke } from "@tauri-apps/api/core";
import { calcSize, deleteRemote, mkdir, paneAction, remoteName, transfer, trashLocal, TRASH_LABEL } from "./actions";
import { describeAccount } from "./providers";
import { openCompare } from "./compare";
import { setTextSize, setTheme, appearance } from "./appearance";
import { icon } from "./icons";
import type { Pane } from "./pane";
import { active, closeView, local, MAX_VIEWS, remote, remotes, switchTo } from "./panes";
import {
  addAccount,
  canCreateBuckets,
  newBucket,
  openSettings,
  selectBucket,
  splitView,
  switchProfile,
} from "./settings";
import { app, exp } from "./state";
import { checkForUpdates } from "./updater";
import { $, esc, fail, toast } from "./util";

type Entry = { group: string; label: string; sub?: string; icon: string; run: () => unknown };

const dlg = () => $<HTMLDialogElement>("palette");
const input = () => $<HTMLInputElement>("pal-q");
let entries: Entry[] = [];
let shown: Entry[] = [];
let hl = 0;

function build(): Entry[] {
  const out: Entry[] = [];
  const cmd = (label: string, ic: string, run: () => unknown, sub?: string) =>
    out.push({ group: "Commands", label, sub, icon: ic, run });
  const sel = active.selectedItems();
  if (local.selectedItems().length)
    cmd(`Upload selection to ${remoteName()}`, "arrowRight", () => transfer(local, local.selectedItems()), "F5");
  if (remote.selectedItems().length)
    cmd("Download selection to this computer", "arrowLeft", () => transfer(remote, remote.selectedItems()), "F5");
  if (sel.length === 1 && !sel[0].dir)
    cmd(`Quick look “${sel[0].name}”`, "eye", () => paneAction(active, "preview"), "Space");
  if (remote.selectedItems().length) cmd(`Delete selection from ${remoteName()}`, "trash", () => deleteRemote(), "⌘⌫");
  if (local.selectedItems().length) cmd(TRASH_LABEL, "trash", () => trashLocal(), "⌘⌫");
  cmd("New folder", "folderPlus", () => mkdir(active));
  if (canCreateBuckets(remote))
    cmd(remote.profile()?.kind === "azure" ? "New container…" : "New bucket…", "plus", () => newBucket(remote));
  if (canManagePolicy(remote)) cmd(`Access policy for “${remote.bucket}”…`, "key", () => openPolicy(remote));
  if (canSpeedTest())
    cmd("Speed test…", "gauge", () =>
      openSpeedTest(remote.bucket ? { account: remote.account, bucket: remote.bucket, cwd: remote.cwd } : undefined),
    );
  cmd("Refresh", "refresh", () => active.load(true), "⌘R");
  cmd("Folder size (Σ)", "sigma", () => calcSize(active));
  if (remote.bucket) cmd("Compare this computer ⇄ remote (find differences, checksums)", "compare", openCompare);
  cmd("Go to path…", "pencil", () => editPath(active), "⌘L");
  cmd("Copy current path", "copy", () => copyPath(active));
  cmd(app.showHidden ? "Hide hidden files" : "Show hidden files", "eye", () => paneAction(local, "hidden"));
  if (remotes.length < MAX_VIEWS) cmd("Split view: another account below", "split", () => splitView(remote), "⌘\\");
  if (remotes.length > 1) cmd("Close this remote view", "x", () => closeView(remote), "⌘W");
  for (const f of favorites())
    out.push({
      group: "Favorites",
      label: f.name,
      sub: f.account === undefined ? f.path : `${f.bucket}/${f.path}`,
      icon: "star",
      run: () => openFavorite(f),
    });
  if (exp("backup_pairs")) {
    for (const p of pairs()) cmd(`Back up: ${p.name}`, "arrowUp", () => runPair(p));
    if (remote.bucket) cmd("Save current folders as a backup pair", "plus", savePairFromPanes);
  }
  cmd("Settings", "sliders", () => openSettings(), "⌘,");
  cmd("Add account (S3, Nextcloud, WebDAV)", "plus", addAccount);
  cmd("Check for updates", "update", () => checkForUpdates(true));
  const t = appearance.theme;
  for (const th of ["light", "dark", "system"] as const)
    if (th !== t) cmd(`Theme: ${th[0].toUpperCase()}${th.slice(1)}`, "sun", () => setTheme(th));
  const size = +appearance.size;
  if (size < 18) cmd("Larger text", "plus", () => setTextSize(String(size < 14 ? 14 : size < 16 ? 16 : 18)));
  if (size > 13) cmd("Smaller text", "sigma", () => setTextSize(String(size > 16 ? 16 : size > 14 ? 14 : 13)));

  const current = remote.account;
  for (const p of app.settings.profiles)
    if (p.id !== current)
      out.push({
        group: "Accounts",
        label: `Switch to ${p.name}`,
        sub: describeAccount(p),
        icon: "cloud",
        run: () => switchProfile(p.id),
      });
  for (const b of remote.buckets)
    if (b.name !== remote.bucket)
      out.push({
        group: "Buckets",
        label: b.name,
        sub: b.region ?? undefined,
        icon: "bucket",
        run: () => selectBucket(remote, b.name),
      });
  for (const r of local.roots)
    out.push({
      group: "Places",
      label: r.name.replace(/^\p{Extended_Pictographic}\s*/u, ""),
      sub: r.path,
      icon: "drive",
      run: () => (switchTo(local), local.go(r.path)),
    });
  for (const p of [local, ...remotes]) {
    for (const e of p.entries) {
      out.push({
        group: p.isRemote ? `In ${p.profile()?.name ?? ""} · ${p.bucket}` : "On this computer",
        label: e.name,
        icon: e.dir ? "folder" : "doc",
        run: () => (e.dir ? (switchTo(p), p.go(e.key)) : reveal(p, e.key)),
      });
    }
  }
  return out;
}

/** Selects a file in its pane and scrolls it into view. */
function reveal(p: Pane, key: string) {
  switchTo(p);
  p.q<HTMLInputElement>(".filter").value = "";
  p.render();
  const i = p.view.findIndex((e) => e.key === key);
  if (i >= 0) p.selectIndex(i);
}

export function copyPath(p: Pane) {
  const path = p.fullPath();
  if (path) navigator.clipboard.writeText(path).then(() => toast("Path copied"), fail);
}

/** Swaps the crumbs for a text box (⌘L or a click on empty path-bar space). Enter goes there. */
export function editPath(p: Pane) {
  const box = p.q<HTMLInputElement>(".path-input");
  const crumbs = p.q(".crumbs");
  box.value = p.isRemote ? p.cwd : p.cwd;
  box.placeholder = p.isRemote ? "folder/sub/ (in this bucket)" : "/Users/me/Documents";
  crumbs.hidden = true;
  box.hidden = false;
  box.focus();
  box.select();
  const done = (go: boolean) => {
    box.removeEventListener("keydown", onKey);
    box.removeEventListener("blur", onBlur);
    box.hidden = true;
    crumbs.hidden = false;
    if (!go) return;
    let v = box.value.trim();
    if (p.isRemote) {
      // Accept "s3://bucket/a/b", "bucket/a/b" (current bucket) or "a/b".
      v = v.replace(/^s3:\/\//, "");
      if (p.bucket && (v === p.bucket || v.startsWith(p.bucket + "/"))) v = v.slice(p.bucket.length + 1);
      v = v.replace(/^\/+/, "");
      if (v && !v.endsWith("/")) v += "/";
      p.go(v);
    } else if (v) {
      if (v.startsWith("~")) v = v.replace(/^~/, homeOf());
      invoke("list_local", { path: v, hidden: app.showHidden }).then(
        () => p.go(v),
        (e) => fail(e),
      );
    }
  };
  const onKey = (ev: KeyboardEvent) => {
    if (ev.key === "Enter") done(true);
    else if (ev.key === "Escape") done(false);
    else return;
    ev.preventDefault();
    ev.stopPropagation();
  };
  const onBlur = () => done(false);
  box.addEventListener("keydown", onKey);
  box.addEventListener("blur", onBlur);
}

function homeOf() {
  return local.roots.find((r) => /Home$/.test(r.name))?.path ?? "";
}

// ---------- rendering ----------

function score(e: Entry, q: string) {
  if (!q) return e.group === "Commands" ? 1 : 0;
  const s = e.label.toLowerCase();
  if (s.startsWith(q)) return 4;
  if (s.includes(q)) return 3;
  if ((e.sub ?? "").toLowerCase().includes(q)) return 2;
  let i = 0; // subsequence ("nwf" → "New folder")
  for (const ch of s) if (ch === q[i]) i++;
  return i === q.length ? 1 : 0;
}

function render() {
  const q = input().value.trim().toLowerCase();
  shown = entries
    .map((e, i) => ({ e, s: score(e, q), i }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .slice(0, 60)
    .map((x) => x.e);
  hl = Math.min(hl, Math.max(0, shown.length - 1));
  let group = "";
  $("pal-list").innerHTML =
    shown
      .map((e, i) => {
        const head = e.group !== group ? `<div class="mh">${esc((group = e.group))}</div>` : "";
        return `${head}<div class="mi${i === hl ? " hl" : ""}" role="option" data-i="${i}"><span class="mi-ico">${icon(e.icon)}</span><span>${esc(e.label)}</span>${e.sub ? `<span class="mi-sub">${esc(e.sub)}</span>` : ""}</div>`;
      })
      .join("") || `<div class="none">No matches</div>`;
  $("pal-list").querySelector(".mi.hl")?.scrollIntoView({ block: "nearest" });
}

function run(i: number) {
  const e = shown[i];
  dlg().close();
  if (e) Promise.resolve(e.run()).catch(fail);
}

export function openPalette() {
  entries = build();
  hl = 0;
  input().value = "";
  render();
  if (!dlg().open) dlg().showModal();
  input().focus();
}

export function setupPalette() {
  $("palette-btn").onclick = openPalette;
  input().addEventListener("input", () => ((hl = 0), render()));
  input().addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown") hl = Math.min(hl + 1, shown.length - 1);
    else if (ev.key === "ArrowUp") hl = Math.max(hl - 1, 0);
    else if (ev.key === "Enter") return (ev.preventDefault(), run(hl));
    else return;
    ev.preventDefault();
    render();
  });
  $("pal-list").addEventListener("click", (ev) => {
    const el = (ev.target as HTMLElement).closest<HTMLElement>(".mi");
    if (el) run(+el.dataset.i!);
  });
  dlg().addEventListener("mousedown", (ev) => {
    if (ev.target === dlg()) dlg().close(); // click on the backdrop
  });
}
