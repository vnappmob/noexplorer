// Favorites: the star in each path bar saves the folder, per device. They show up in the places
// menu (local), the account menu (remote) and ⌘K. A location is a local folder, or account +
// bucket + prefix for a remote view.

import type { Pane } from "./pane";
import { local, remotes } from "./panes";
import { loadBuckets } from "./settings";
import { app } from "./state";
import { store, toast } from "./util";

export type Loc = { path: string; account?: string; bucket?: string };
export type Favorite = Loc & { name: string };

const trim = (s: string) => s.replace(/[\\/]+$/, "");
const locOf = (p: Pane): Loc => (p.isRemote ? { account: p.account, bucket: p.bucket, path: p.cwd } : { path: p.cwd });
const same = (a: Loc, b: Loc) => a.account === b.account && a.bucket === b.bucket && trim(a.path) === trim(b.path);

/** A location's short name: the folder, or the bucket / account at a root. */
function locName(l: Loc) {
  const base = trim(l.path).split(/[\\/]/).pop();
  if (base) return base;
  if (l.account !== undefined)
    return l.bucket || app.settings.profiles.find((x) => x.id === l.account)?.name || "Remote";
  return l.path || "/";
}

export function favorites(): Favorite[] {
  try {
    const v = JSON.parse(store.get("favorites") ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export function toggleFavorite(p: Pane) {
  const l = locOf(p);
  const list = favorites();
  const had = list.some((f) => same(f, l));
  store.set("favorites", JSON.stringify(had ? list.filter((f) => !same(f, l)) : [...list, { ...l, name: locName(l) }]));
  toast(had ? `Removed “${locName(l)}” from favorites` : `Added “${locName(l)}” to favorites`);
  renderStar(p);
}

/** Opens a favorite: local ones on the left, remote ones in `view`, else a view already showing
 * that account + bucket, else the first one. */
export async function openFavorite(f: Favorite, view?: Pane) {
  if (f.account === undefined) return local.go(f.path);
  const v = view ?? remotes.find((x) => x.account === f.account && x.bucket === f.bucket) ?? remotes[0];
  if (v.account === f.account && v.bucket === f.bucket) return v.go(f.path);
  v.account = f.account ?? "";
  v.bucket = f.bucket ?? "";
  v.cwd = f.path;
  await loadBuckets(v, true);
}

/** The star in `p`'s path bar (called after every listing, `paneHooks.visited`). */
export function renderStar(p: Pane) {
  const fav = p.q<HTMLButtonElement>(".fav-btn");
  const starred = favorites().some((f) => same(f, locOf(p)));
  fav.classList.toggle("on", starred);
  fav.title = starred ? "Remove from favorites" : "Add to favorites";
}

export function bindStar(p: Pane) {
  p.q(".fav-btn").onclick = () => toggleFavorite(p);
  renderStar(p);
}

export function favoritesMenuItems(kind: "local" | "remote", run: (f: Favorite) => unknown) {
  return favorites()
    .filter((f) => (kind === "local") === (f.account === undefined))
    .map((f) => ({
      label: f.name,
      sub: f.account === undefined ? f.path : `${f.bucket}/${f.path}`,
      icon: "star",
      run: () => run(f),
    }));
}
