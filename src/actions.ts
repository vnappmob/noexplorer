// User actions that talk to the backend: transfer, open, link, delete, mkdir.
// `paneAction` maps the `data-act` / context-menu action names to these.

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { Pane } from "./pane";
import { local, remote } from "./panes";
import { openPreview } from "./preview";
import { activeProfile, app } from "./state";
import type { Item, SizeInfo } from "./types";
import { askText, fail, fmtSize, plural, store, toast } from "./util";

/**
 * Copies `items` from `from` to the other pane: local → upload to S3, remote → download.
 * `targetDir` is a remote prefix or local path; defaults to the other pane's current folder.
 * Progress arrives later through `"job"` events (jobs.ts).
 */
export async function transfer(from: Pane, items: Item[], targetDir?: string) {
  if (!items.length) return;
  if (!activeProfile() || !app.bucket) return toast("Add an S3 account first", true);
  try {
    if (from === local) {
      await invoke("upload", { bucket: app.bucket, prefix: targetDir ?? remote.cwd, paths: items.map((e) => e.key) });
    } else {
      await invoke("download", {
        bucket: app.bucket,
        prefix: remote.cwd,
        items: items.map((e) => ({ key: e.key, size: e.size })),
        dest: targetDir ?? local.cwd,
      });
    }
  } catch (e) {
    fail(e);
  }
}

/** Opens a file with the OS default app (remote files are downloaded to a temp dir first). */
export async function openItem(p: Pane, e: Item) {
  try {
    if (p.isRemote) {
      toast(`Opening ${e.name}…`);
      await invoke("open_object", { bucket: app.bucket, key: e.key });
    } else {
      await invoke("open_local", { path: e.key });
    }
  } catch (err) {
    fail(err);
  }
}

export async function copyLink(e: Item) {
  try {
    const url = await invoke<string>("presign", { bucket: app.bucket, key: e.key, secs: 7 * 24 * 3600, inline: false });
    await navigator.clipboard.writeText(url);
    toast("Link copied (expires in 7 days)");
  } catch (err) {
    fail(err);
  }
}

export async function deleteRemote(list = remote.selectedItems()) {
  if (!list.length) return;
  const dirs = list.filter((e) => e.dir).length;
  const what = list.length === 1 ? `"${list[0].name}"` : plural(list.length, "item");
  const ok = await ask(`Delete ${what} from S3?${dirs ? "\nFolders are deleted with everything inside them." : ""}`, {
    title: "Delete",
    kind: "warning",
    okLabel: "Delete",
    cancelLabel: "Cancel",
  });
  if (!ok) return;
  try {
    const n = await invoke<number>("delete", { bucket: app.bucket, keys: list.map((e) => e.key) });
    toast(`Deleted ${plural(n, "object")}`);
  } catch (e) {
    fail(e);
  }
  remote.load();
}

export async function mkdir(p: Pane) {
  const name = await askText("New folder name");
  if (!name?.trim()) return;
  try {
    if (p.isRemote) await invoke("mkdir", { bucket: app.bucket, prefix: p.cwd, name });
    else await invoke("mkdir_local", { path: p.cwd, name });
    await p.load();
  } catch (e) {
    fail(e);
  }
}

async function sizeOf(p: Pane, key: string) {
  return p.isRemote
    ? invoke<SizeInfo>("remote_size", { bucket: app.bucket, prefix: key })
    : invoke<SizeInfo>("local_size", { path: key });
}

/**
 * "Σ Size": selected folders get their recursive size in the Size column; with no folder
 * selected, the current folder's total goes into the path bar.
 */
export async function calcSize(p: Pane) {
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
    link: () => one && copyLink(one),
    "copy-path": () =>
      one &&
      navigator.clipboard.writeText(p.isRemote ? `${app.bucket}/${one.key}` : one.key).then(() => toast("Copied")),
    reveal: () => invoke("reveal", { path: one ? one.key : p.cwd }).catch(fail),
    delete: () => p.isRemote && deleteRemote(sel),
    size: () => calcSize(p),
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
