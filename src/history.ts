// Back (experimental "back_history"): each pane remembers the folders it left, with where the list
// was scrolled and what was selected, and ⌘[ / the ← button / the mouse's back button return there.
// Only back: going somewhere new is the way forward.
//
// A move is recorded after each successful listing (`paneHooks.visited`): the previous location
// goes on the pane's stack, unless the move itself was a step back. The scroll position of the
// location on screen is kept up to date as the list scrolls, so it's there when the pane leaves.

import type { Pane } from "./pane";
import { loadBuckets } from "./settings";
import { exp } from "./state";

type Loc = { account: string; bucket: string; path: string };
type Spot = Loc & { scroll: number; selected: string[] };
type State = { stack: Spot[]; here: Spot | null; returning: Spot | null };

const MAX = 50;
const states = new WeakMap<Pane, State>();

const locOf = (p: Pane): Loc => ({
  account: p.isRemote ? p.account : "",
  bucket: p.isRemote ? p.bucket : "",
  path: p.cwd,
});
const same = (a: Loc, b: Loc) =>
  a.account === b.account && a.bucket === b.bucket && a.path.replace(/[\\/]+$/, "") === b.path.replace(/[\\/]+$/, "");

function state(p: Pane): State {
  let s = states.get(p);
  if (!s) states.set(p, (s = { stack: [], here: null, returning: null }));
  return s;
}

/** Right before a pane leaves its folder: keep where it was scrolled and what was selected. Scroll
 * events also update this, but they arrive a frame late (and not at all while hidden). */
export function leaving(p: Pane) {
  const here = state(p).here;
  if (here && same(here, locOf(p))) Object.assign(here, { scroll: p.list.scrollTop, selected: [...p.selected] });
}

/** After every listing: record the move, or finish a step back (scroll and selection). */
export function visited(p: Pane) {
  const s = state(p);
  const loc = locOf(p);
  if (s.returning && same(s.returning, loc)) {
    const to = s.returning;
    s.returning = null;
    p.selected = new Set(to.selected.filter((k) => p.view.some((e) => e.key === k)));
    p.paint();
    p.list.scrollTop = to.scroll; // the virtualized list redraws on the scroll event
  } else if (s.here && !same(s.here, loc)) {
    s.stack.push(s.here);
    if (s.stack.length > MAX) s.stack.shift();
  }
  s.here = { ...loc, scroll: p.list.scrollTop, selected: [...p.selected] };
  render(p);
}

export async function back(p: Pane) {
  if (!exp("back_history")) return;
  const s = state(p);
  const to = s.stack.pop();
  if (!to) return;
  s.returning = to;
  s.here = null; // the folder being left isn't pushed again
  render(p);
  if (!p.isRemote || (p.account === to.account && p.bucket === to.bucket)) return p.go(to.path);
  // (`go` and the switches call `leaving`, which only updates `here`; it's null here on purpose.)
  p.account = to.account;
  p.bucket = to.bucket;
  p.cwd = to.path;
  await loadBuckets(p, true);
}

function render(p: Pane) {
  const b = p.q<HTMLButtonElement>(".back");
  b.hidden = !exp("back_history");
  b.disabled = !state(p).stack.length;
}

/** Wires the ← button, the mouse's back button and scroll tracking (every pane, local and remote). */
export function bindBack(p: Pane) {
  p.q(".back").onclick = () => back(p);
  p.el.addEventListener("mouseup", (ev) => {
    if (ev.button === 3 && exp("back_history")) (ev.preventDefault(), back(p));
  });
  p.list.addEventListener(
    "scroll",
    () => {
      // Only while the pane still shows that place: opening a folder scrolls to the top first.
      const here = state(p).here;
      if (here && same(here, locOf(p))) here.scroll = p.list.scrollTop;
    },
    { passive: true },
  );
  // Selection changes are picked up when leaving: keep `here` in step with clicks too.
  const keep = () => {
    const here = state(p).here;
    if (here && same(here, locOf(p))) here.selected = [...p.selected];
  };
  p.list.addEventListener("click", keep);
  p.list.addEventListener("keyup", keep);
  window.addEventListener("experimental", () => render(p));
  window.addEventListener("settings-loaded", () => render(p));
  render(p);
}
