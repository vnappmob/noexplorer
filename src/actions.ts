// User actions that talk to the backend: transfer, open, link, delete, mkdir.
// `paneAction` maps the context-menu / palette action names to these.

import { invoke } from "@tauri-apps/api/core";
import { openCompare } from "./compare";
import { duplicateItems, moveItems, renameItem } from "./fileops";
import { noteJobItems } from "./jobs";
import { requestSizes } from "./sizes";
import { ask } from "@tauri-apps/plugin-dialog";
import type { Pane } from "./pane";
import { closeView, local, remote } from "./panes";
import { splitView } from "./settings";
import { openPreview } from "./preview";
import { app } from "./state";
import type { Item, SizeInfo } from "./types";
import { askText, fail, fmtSize, plural, store, toast } from "./util";

/**
 * Copies `items` from `from` to `to`: local → remote uploads, remote → local downloads, and
 * remote → another remote view copies between accounts (server-side when possible). `to` defaults
 * to the other side (the focused remote view for local items); `targetDir` to its current folder.
 * Progress arrives later through `"job"` events (jobs.ts).
 */
export async function transfer(from: Pane, items: Item[], targetDir?: string, to?: Pane) {
  if (!items.length) return;
  const dest = to ?? (from.isRemote ? local : remote);
  if (from === dest) return;
  for (const v of [from, dest])
    if (v.isRemote && (!v.profile() || !v.bucket)) return toast("Add a remote account first", true);
  const list = items.map((e) => ({ key: e.key, size: e.size }));
  try {
    let id: string;
    if (from.isRemote && dest.isRemote) {
      id = await invoke<string>("copy_remote", {
        srcAccount: from.account || null,
        srcBucket: from.bucket,
        srcPrefix: from.cwd,
        items: list,
        dstAccount: dest.account || null,
        dstBucket: dest.bucket,
        dstPrefix: targetDir ?? dest.cwd,
      });
    } else if (dest.isRemote) {
      id = await invoke<string>("upload", {
        ...dest.where(),
        prefix: targetDir ?? dest.cwd,
        paths: items.map((e) => e.key),
      });
    } else {
      id = await invoke<string>("download", {
        ...from.where(),
        prefix: from.cwd,
        items: list,
        dest: targetDir ?? local.cwd,
      });
    }
    noteJobItems(
      id,
      items.map((e) => e.key),
    ); // the beam starts at these rows
  } catch (e) {
    fail(e);
  }
}

/** Opens a file with the OS default app (remote files are downloaded to a temp dir first). */
export async function openItem(p: Pane, e: Item) {
  try {
    if (p.isRemote) {
      toast(`Opening ${e.name}…`);
      await invoke("open_object", { ...p.where(), key: e.key });
    } else {
      await invoke("open_local", { path: e.key });
    }
  } catch (err) {
    fail(err);
  }
}

export async function copyLink(p: Pane, e: Item) {
  try {
    const url = await invoke<string>("presign", { ...p.where(), key: e.key, secs: 7 * 24 * 3600, inline: false });
    await navigator.clipboard.writeText(url);
    toast("Link copied (expires in 7 days)");
  } catch (err) {
    fail(err);
  }
}

/** A remote view's account name, for labels ("Upload to Home NAS"). */
export const remoteName = (v: Pane = remote) => v.profile()?.name || "the remote";

/** "Move to Trash" / "Recycle Bin" on Windows: the label the OS uses. */
export const TRASH_LABEL = navigator.userAgent.includes("Windows") ? "Delete (to Recycle Bin)" : "Move to Trash";

/** Local selection → the OS Trash (recoverable, so no confirmation, like Finder/Explorer). */
export async function trashLocal(list = local.selectedItems()) {
  if (!list.length) return;
  try {
    const n = await invoke<number>("trash_local", { paths: list.map((e) => e.key) });
    const bin = navigator.userAgent.includes("Windows") ? "the Recycle Bin" : "the Trash";
    toast(`Moved ${list.length === 1 ? `“${list[0].name}”` : plural(n, "item")} to ${bin}`);
  } catch (e) {
    fail(e);
  }
  // Pick the row after the trashed ones, so ⌘⌫ can be repeated.
  const next = local.view.findIndex((e) => list.some((x) => x.key === e.key));
  await local.load(true);
  if (next >= 0) local.selectIndex(Math.min(next, local.view.length - 1));
}

export async function deleteRemote(v: Pane = remote, list = v.selectedItems()) {
  if (!list.length) return;
  const dirs = list.filter((e) => e.dir).length;
  const what = list.length === 1 ? `"${list[0].name}"` : plural(list.length, "item");
  const ok = await ask(
    `Delete ${what} from ${remoteName(v)}?${dirs ? "\nFolders are deleted with everything inside them." : ""}`,
    {
      title: "Delete",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    },
  );
  if (!ok) return;
  // A background job (progress in the activity card); the rows stay, dimmed, until it's done and
  // the folder refreshes (jobs.ts).
  const keys = list.map((e) => e.key);
  keys.forEach((k) => v.pending.add(k));
  v.selected.clear();
  v.render();
  try {
    noteJobItems(await invoke<string>("delete", { ...v.where(), keys }), keys);
  } catch (e) {
    keys.forEach((k) => v.pending.delete(k));
    v.render();
    fail(e);
  }
}

export async function mkdir(p: Pane) {
  const name = await askText("New folder name");
  if (!name?.trim()) return;
  try {
    if (p.isRemote) await invoke("mkdir", { ...p.where(), prefix: p.cwd, name });
    else await invoke("mkdir_local", { path: p.cwd, name });
    await p.load();
  } catch (e) {
    fail(e);
  }
}

async function sizeOf(p: Pane, key: string) {
  return p.isRemote
    ? invoke<SizeInfo>("remote_size", { ...p.where(), prefix: key })
    : invoke<SizeInfo>("local_size", { path: key });
}

/**
 * "Σ Size": selected folders get their recursive size in the Size column; with no folder
 * selected, the current folder's total goes into the path bar.
 */
export async function calcSize(p: Pane) {
  // Local: one full scan of the folder fills every subfolder's size and the total (sizes.ts).
  if (!p.isRemote) return requestSizes(true);
  const dirs = p.selectedItems().filter((e) => e.dir);
  if (!dirs.length) return calcCurrentDir(p);
  p.q(".dirsize").textContent = `Calculating ${plural(dirs.length, "folder")}…`;
  try {
    await Promise.all(
      dirs.map(async (d) => {
        const s = await sizeOf(p, d.key);
        d.size = s.bytes;
        d.calc = true;
      }),
    );
    p.render();
  } catch (e) {
    fail(e);
  }
  p.q(".dirsize").textContent = "";
}

async function calcCurrentDir(p: Pane) {
  const where = p.cwd;
  const out = p.q(".dirsize");
  out.textContent = "Calculating…";
  try {
    const s = await sizeOf(p, where);
    if (p.cwd === where) out.textContent = `${fmtSize(s.bytes)} · ${plural(s.files, "file")}`;
  } catch (e) {
    out.textContent = "";
    fail(e);
  }
}

/** Runs a named action on `p`'s selection (footer buttons and context menu). */
export function paneAction(p: Pane, act: string) {
  const sel = p.selectedItems();
  const one = sel.length === 1 ? sel[0] : null;
  const actions: Record<string, () => unknown> = {
    transfer: () => transfer(p, sel),
    preview: () => one && !one.dir && openPreview(p, one),
    open: () => one && !one.dir && openItem(p, one),
    "open-dir": () => one && p.go(one.key),
    link: () => one && copyLink(p, one),
    "copy-path": () =>
      one && navigator.clipboard.writeText(p.isRemote ? `${p.bucket}/${one.key}` : one.key).then(() => toast("Copied")),
    reveal: () => invoke("reveal", { path: one ? one.key : p.cwd }).catch(fail),
    delete: () => (p.isRemote ? deleteRemote(p, sel) : trashLocal(sel)),
    size: () => calcSize(p),
    compare: () => openCompare(),
    rename: () => one && renameItem(p, one),
    move: () => moveItems(p, sel),
    duplicate: () => duplicateItems(p, sel),
    split: () => splitView(p),
    "close-view": () => closeView(p),
    mkdir: () => mkdir(p),
    refresh: () => p.load(true),
    hidden: () => {
      app.showHidden = !app.showHidden;
      store.set("hidden", app.showHidden ? "1" : "0");
      local.load(true);
    },
  };
  actions[act]?.();
}
