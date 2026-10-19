// Back (experimental "back_history"): up one level, back to the parent folder as you left it,
// scrolled and selected the same. ⌘[, the ← button and the mouse's back button do it; with the
// experiment on, every other "up" (↑ ⌘↑ Backspace) restores the parent the same way.
//
// Each pane remembers, per folder, where the list was scrolled and what was selected when it left
// (`leaving`, called by `Pane.go`, `Pane.up` and the account/bucket switches). Going up asks for the
// parent's spot back (`goingUp`), and the next listing applies it (`visited`).

import { type Pane, paneHooks } from "./pane";
import { exp } from "./state";

type Spot = { scroll: number; selected: string[] };
type State = { spots: Map<string, Spot>; returning: string | null };

/** Folders remembered per pane (oldest forgotten first). */
const MAX = 400;
const states = new WeakMap<Pane, State>();

const trim = (s: string) => s.replace(/[\\/]+$/, "");
const keyOf = (p: Pane, path = p.cwd) => (p.isRemote ? `${p.account}|${p.bucket}|${trim(path)}` : trim(path));

function state(p: Pane): State {
  let s = states.get(p);
  if (!s) states.set(p, (s = { spots: new Map(), returning: null }));
  return s;
}

/** Right before a pane leaves its folder (`paneHooks.leaving`). */
export function leaving(p: Pane) {
  if (!p.entries.length) return;
  const { spots } = state(p);
  const k = keyOf(p);
  spots.delete(k); // re-insert: the map's order is the forgetting order
  spots.set(k, { scroll: p.list.scrollTop, selected: [...p.selected] });
  if (spots.size > MAX) spots.delete(spots.keys().next().value!);
}

/** `Pane.up` is about to open `parent` (`paneHooks.goingUp`): restore it once it's listed. */
function goingUp(p: Pane, parent: string) {
  state(p).returning = exp("back_history") ? keyOf(p, parent) : null;
}

/** After every listing: finish a step up by restoring the parent's scroll and selection. */
export function visited(p: Pane) {
  const s = state(p);
  const spot = s.returning === keyOf(p) ? s.spots.get(s.returning) : undefined;
  s.returning = null;
  if (spot) {
    p.selected = new Set(spot.selected.filter((k) => p.entries.some((e) => e.key === k)));
    p.paint();
    p.list.scrollTop = spot.scroll; // the virtualized list redraws on the scroll event
  }
  render(p);
}

export function back(p: Pane) {
  if (exp("back_history")) p.up();
}

function render(p: Pane) {
  const b = p.q<HTMLButtonElement>(".back");
  b.hidden = !exp("back_history");
  b.disabled = p.isRemote ? !p.cwd : !p.parent;
}

/** Wires the ← button and the mouse's back button (every pane, local and remote). */
export function bindBack(p: Pane) {
  p.q(".back").onclick = () => back(p);
  p.el.addEventListener("mouseup", (ev) => {
    if (ev.button === 3 && exp("back_history")) (ev.preventDefault(), back(p));
  });
  window.addEventListener("experimental", () => render(p));
  window.addEventListener("settings-loaded", () => render(p));
  render(p);
}

paneHooks.goingUp = goingUp;
