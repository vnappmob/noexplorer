// iPhone backups (experimental, iphone_backup.rs): back up a connected iPhone to read apps' files,
// which then show (read-only) in the device's `Backup/` folder. In the right-click menu at the top
// of the device and in `Backup/`, and in ⌘K.

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { Pane } from "./pane";
import { exp } from "./state";
import type { IPhoneBackup } from "./types";
import { fail, fmtSize, toast } from "./util";

export const canBackUp = (v: Pane) => exp("iphone") && v.isRemote && v.profile()?.kind === "iphone";
/** Where the right-click menu offers the backup: the device's top level and its `Backup/` folder. */
export const backupHere = (v: Pane) => canBackUp(v) && !v.search && (v.cwd === "" || v.cwd.startsWith("Backup/"));

const when = (ms: number) => new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

export const backupInfo = (v: Pane, size = false) =>
  invoke<IPhoneBackup | null>("iphone_backup_info", { account: v.account, size }).catch(() => null);

/** Backs up the view's iPhone (asks first the first time: it's the whole device). */
export async function backUp(v: Pane) {
  const info = await backupInfo(v);
  if (info?.running) return toast("This iPhone is being backed up already");
  if (!info) {
    const ok = await ask(
      "NoExplorer asks the iPhone for a full backup, the same one Finder or Apple Devices makes, and keeps it on this computer. Then the files of every app show up in the iPhone's Backup folder, read-only, even with the iPhone unplugged.\n\n" +
        "The first backup can take a while and needs about as much free space as the data on the iPhone; later ones only bring what changed. Keep the iPhone connected and unlocked. If it asks for its passcode, enter it.\n\n" +
        "Encrypted backups can't be read yet: if “Encrypt local backup” is on for this iPhone, NoExplorer tells you before it starts.",
      { title: "Back up this iPhone?", kind: "info", okLabel: "Back up", cancelLabel: "Cancel" },
    );
    if (!ok) return;
  } else if (!info.complete) toast("The last backup didn't finish: the iPhone starts it over");
  try {
    await invoke<string>("iphone_backup", { account: v.account });
    toast("Backing up… keep the iPhone connected");
  } catch (e) {
    fail(e);
  }
}

/** Moves the backup to the Trash, after saying how big it is. */
export async function deleteBackup(v: Pane) {
  const info = await backupInfo(v, true);
  if (!info) return toast("There's no backup of this iPhone");
  const size = info.size != null ? ` (${fmtSize(info.size)})` : "";
  const ok = await ask(`Move the backup from ${when(info.date)}${size} to the Trash?`, {
    title: "Delete backup",
    kind: "warning",
    okLabel: "Move to Trash",
    cancelLabel: "Cancel",
  });
  if (!ok) return;
  try {
    await invoke("iphone_delete_backup", { account: v.account });
    toast("Backup moved to the Trash");
    if (v.cwd.startsWith("Backup/")) v.cwd = ""; // gone: back to the top of the device
    window.dispatchEvent(new CustomEvent("iphone-backup", { detail: v.account }));
  } catch (e) {
    fail(e);
  }
}
