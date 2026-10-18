// Backup pairs (experimental "backup_pairs"): "this folder ↔ that remote folder", backed up in one
// click. A run is Compare (no checksums) followed by uploading what's missing or changed, the
// same steps as the Compare dialog, without the dialog. Pairs are per device (localStorage): they
// point at local folders.

import { invoke } from "@tauri-apps/api/core";
import { local, remote } from "./panes";
import { openMenu, type MenuItem } from "./popover";
import { openRemote } from "./settings";
import { app, exp } from "./state";
import type { Diff } from "./types";
import { $, askText, fail, plural, store, toast } from "./util";

export type Pair = {
  id: string;
  name: string;
  local: string;
  account: string;
  bucket: string;
  prefix: string;
  /** Last run: when, and what it found / did. */
  last?: { at: number; text: string };
};

export function pairs(): Pair[] {
  try {
    const v = JSON.parse(store.get("backup-pairs") ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

const savePairs = (list: Pair[]) => store.set("backup-pairs", JSON.stringify(list));

const accountName = (id: string) => app.settings.profiles.find((p) => p.id === id)?.name ?? "Removed account";
const baseName = (p: string) =>
  p
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .pop() || p;

function when(t: number) {
  const m = Math.round((Date.now() - t) / 60000);
  return m < 1
    ? "just now"
    : m < 60
      ? `${m} min ago`
      : m < 1440
        ? `${Math.round(m / 60)} h ago`
        : new Date(t).toLocaleDateString();
}

/** Saves the left pane's folder and the focused remote view's folder as a pair. */
export async function savePairFromPanes() {
  if (!remote.profile() || !remote.bucket || !local.cwd)
    return toast("Open a local folder and a remote folder first", true);
  const name = await askText(
    "Name this backup pair",
    `${baseName(local.cwd)} → ${remote.profile()!.name}${remote.cwd ? `/${baseName(remote.cwd)}` : ""}`,
  );
  if (!name?.trim()) return;
  const p: Pair = {
    id: crypto.randomUUID(),
    name: name.trim(),
    local: local.cwd,
    account: remote.account,
    bucket: remote.bucket,
    prefix: remote.cwd,
  };
  savePairs([...pairs(), p]);
  toast(`Saved “${p.name}”. Run it from the Backups button or ⌘K.`);
}

function setLast(id: string, text: string) {
  savePairs(pairs().map((p) => (p.id === id ? { ...p, last: { at: Date.now(), text } } : p)));
}

/** Backs a pair up now: Compare, then upload what's missing or changed (a normal upload job). */
export async function runPair(p: Pair) {
  if (!app.settings.profiles.some((a) => a.id === p.account)) return fail(`“${p.name}”: its account was removed`);
  toast(`Backing up “${p.name}”: checking…`);
  try {
    const where = { account: p.account, bucket: p.bucket };
    const diff = await invoke<Diff>("compare", {
      id: crypto.randomUUID(),
      ...where,
      prefix: p.prefix,
      local: p.local,
      verify: false,
    });
    const rels = [...diff.only_local.map((f) => f.rel), ...diff.changed.map((c) => c.rel)];
    if (!rels.length) {
      setLast(p.id, `Up to date (${plural(diff.local_files, "file")})`);
      return toast(`“${p.name}” is up to date`);
    }
    await invoke("upload_rel", { ...where, prefix: p.prefix, root: p.local, rels });
    setLast(p.id, `Sent ${plural(rels.length, "file")}`);
  } catch (e) {
    setLast(p.id, "Failed");
    fail(e);
  }
}

/** Shows the pair's two folders in the panes. */
export async function showPair(p: Pair) {
  await Promise.all([local.go(p.local), openRemote(p.account, p.bucket, p.prefix)]);
}

function removePair(p: Pair) {
  savePairs(pairs().filter((x) => x.id !== p.id));
  toast(`Removed “${p.name}”`);
}

/** The Backups button's menu. */
export function openBackupsMenu(anchor: HTMLElement = $("btn-backups")) {
  const list = pairs();
  const pick = (title: string, run: (p: Pair) => unknown) => () =>
    openMenu(
      anchor,
      [{ header: title }, ...list.map((p): MenuItem => ({ label: p.name, icon: "folder", run: () => run(p) }))],
      {
        width: 320,
      },
    );
  const items: MenuItem[] = [{ header: "Back up now" }];
  for (const p of list)
    items.push({
      label: p.name,
      sub: `${baseName(p.local)} → ${accountName(p.account)} · ${p.last ? `${p.last.text}, ${when(p.last.at)}` : "never run"}`,
      icon: "arrowUp",
      run: () => runPair(p),
    });
  if (!list.length) items.push({ header: "No backup pairs yet" });
  items.push({ sep: true }, { label: "Save current folders as a pair…", icon: "plus", run: savePairFromPanes });
  if (list.length)
    items.push(
      { label: "Show a pair's folders…", icon: "eye", run: pick("Show folders", showPair) },
      { label: "Remove a pair…", icon: "trash", run: pick("Remove", removePair) },
    );
  openMenu(anchor, items, { width: 360 });
}

/** Shows the Backups button only while the experiment is on. */
export function setupBackups() {
  const sync = () => ($("btn-backups").hidden = !exp("backup_pairs"));
  $("btn-backups").onclick = () => openBackupsMenu();
  window.addEventListener("experimental", sync);
  window.addEventListener("settings-loaded", sync);
  sync();
}
