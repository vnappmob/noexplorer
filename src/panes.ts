// The two pane instances and which one has keyboard focus.

import { Pane } from "./pane";

export const local = new Pane("local");
export const remote = new Pane("remote");
local.other = remote;
remote.other = local;

/** Pane that receives keyboard shortcuts (highlighted header). */
export let active: Pane = local;

export function setActive(p: Pane) {
  active = p;
  local.el.classList.toggle("active", p === local);
  remote.el.classList.toggle("active", p === remote);
}

/** Moves keyboard focus to `p`, selecting the first row if nothing is selected. */
export function switchTo(p: Pane) {
  setActive(p);
  p.list.focus();
  if (p.cursor() < 0) p.selectIndex(0);
}
