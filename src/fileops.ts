// Rename, move, duplicate and one-step undo, in either pane.
// Local operations return at once (fileops.rs); remote ones run as a background job ("move" /
// "dup") whose end refreshes the views (jobs.ts) and arms the undo.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { trashLocal } from "./actions";
import type { Pane } from "./pane";
import { local, remotes } from "./panes";
import type { Item, JobEvent } from "./types";
import { askText, fail, plural, toast } from "./util";

type OpResult = { done: [string, string][]; errors: string[] };

/** What ⌘Z reverses: the last finished operation. */
type Undo =
  | { side: "local"; kind: "rename" | "move" | "dup"; pairs: [string, string][] }
  | { side: "remote"; kind: "rename" | "move" | "dup"; pairs: [string, string][]; account: string; bucket: string };

let last: Undo | null = null;
/** Remote jobs still running → the undo they arm when they finish. */
const pendingUndo = new Map<string, Undo>();

const sep = (p: string) => (p.includes("\\") && !p.includes("/") ? "\\" : "/");
const parentOf = (p: string) => p.slice(0, Math.max(p.replace(/[\\/]+$/, "").lastIndexOf(sep(p)), 0)) || sep(p);
const baseName = (p: string) =>
  p
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .pop() ?? p;

/** Remote key of `name` next to `key` (folders keep their trailing slash). */
const siblingKey = (key: string, name: string, dir: boolean) => {
  const k = key.replace(/\/$/, "");
  const i = k.lastIndexOf("/");
  return `${i >= 0 ? k.slice(0, i + 1) : ""}${name}${dir ? "/" : ""}`;
};

/** "name copy.ext", "name copy 2.ext", … — the first one not in `taken`. */
function copyName(name: string, dir: boolean, taken: Set<string>) {
  const dot = dir ? -1 : name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  for (let i = 1; ; i++) {
    const n = `${stem} copy${i > 1 ? ` ${i}` : ""}${ext}`;
    if (!taken.has(n)) return n;
  }
}

function report(r: OpResult, verb: string) {
  if (r.errors.length) fail(`${r.errors.length} not ${verb}. ${r.errors[0]}`);
  else if (r.done.length > 1) toast(`${plural(r.done.length, "item")} ${verb}`);
}

async function runRemote(v: Pane, kind: "rename" | "move" | "dup", pairs: [string, string][]) {
  try {
    const id = await invoke<string>("remote_move", { ...v.where(), pairs, copy: kind === "dup" });
    pendingUndo.set(id, { side: "remote", kind, pairs, account: v.account, bucket: v.bucket });
  } catch (e) {
    fail(e);
  }
}

export async function renameItem(p: Pane, e: Item) {
  const name = (await askText("Rename", e.name, true))?.trim();
  if (!name || name === e.name) return;
  if (p.isRemote) return runRemote(p, "rename", [[e.key, siblingKey(e.key, name, e.dir)]]);
  try {
    const to = await invoke<string>("rename_local", { path: e.key, name });
    last = { side: "local", kind: "rename", pairs: [[e.key, to]] };
    await p.load(true);
    p.selectIndex(p.view.findIndex((x) => x.key === to));
  } catch (err) {
    fail(err);
  }
}

export async function moveItems(p: Pane, items: Item[], dest?: string) {
  if (!items.length) return;
  if (p.isRemote) {
    const target =
      dest ??
      (await askText(
        `Move ${items.length === 1 ? `“${items[0].name}”` : plural(items.length, "item")} to folder`,
        p.cwd,
      ));
    if (target == null) return;
    const dir = target.trim().replace(/^\/+/, "").replace(/\/?$/, "/").replace(/^\/$/, "");
    const pairs = items
      .map((e): [string, string] => [e.key, `${dir}${e.name}${e.dir ? "/" : ""}`])
      .filter(([a, b]) => a !== b);
    return pairs.length ? runRemote(p, "move", pairs) : undefined;
  }
  const to = dest ?? ((await open({ directory: true, defaultPath: p.cwd, title: "Move to folder" })) as string | null);
  if (!to) return;
  const r = await invoke<OpResult>("move_local", { paths: items.map((e) => e.key), dest: to });
  if (r.done.length) last = { side: "local", kind: "move", pairs: r.done };
  report(r, "moved");
  await p.load(true);
}

export async function duplicateItems(p: Pane, items: Item[]) {
  if (!items.length) return;
  if (p.isRemote) {
    const taken = new Set(p.view.map((e) => e.name));
    const pairs = items.map((e): [string, string] => {
      const n = copyName(e.name, e.dir, taken);
      taken.add(n);
      return [e.key, siblingKey(e.key, n, e.dir)];
    });
    return runRemote(p, "dup", pairs);
  }
  const r = await invoke<OpResult>("duplicate_local", { paths: items.map((e) => e.key) });
  if (r.done.length) last = { side: "local", kind: "dup", pairs: r.done };
  report(r, "duplicated");
  await p.load(true);
}

/** ⌘Z: reverses the last rename, move or duplicate (duplicates go to the Trash / get deleted). */
export async function undo() {
  const u = last;
  if (!u) return;
  last = null;
  try {
    if (u.side === "local") {
      if (u.kind === "dup") {
        await trashLocal(
          u.pairs.map(([, to]) => ({ key: to, name: baseName(to), size: 0, modified: null, dir: false })),
        );
      } else if (u.kind === "rename") {
        await invoke("rename_local", { path: u.pairs[0][1], name: baseName(u.pairs[0][0]) });
      } else {
        const r = await invoke<OpResult>("move_local", {
          paths: u.pairs.map(([, to]) => to),
          dest: parentOf(u.pairs[0][0]),
        });
        report(r, "moved back");
      }
      toast("Undone");
      await local.load(true);
    } else {
      const v = remotes.find((x) => x.account === u.account && x.bucket === u.bucket);
      const where = { account: u.account, bucket: u.bucket };
      if (u.kind === "dup") await invoke("delete", { ...where, keys: u.pairs.map(([, to]) => to) });
      else await invoke("remote_move", { ...where, pairs: u.pairs.map(([a, b]) => [b, a]), copy: false });
      toast("Undoing…");
      v?.load(true);
    }
  } catch (e) {
    fail(e);
  }
}

export function setupFileOps() {
  // A remote operation arms ⌘Z only once it finished cleanly.
  listen<JobEvent>("job", ({ payload: j }) => {
    const u = pendingUndo.get(j.id);
    if (!u || j.status === "running") return;
    pendingUndo.delete(j.id);
    if (j.status === "done") last = u;
  });
}
