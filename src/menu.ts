// Right-click context menu. Items are action names handled by `paneAction` (actions.ts).

import { paneAction } from "./actions";
import type { Pane } from "./pane";
import { local, remote } from "./panes";
import { app } from "./state";
import { $ } from "./util";

export function showMenu(p: Pane, x: number, y: number) {
  const sel = p.selectedItems();
  const one = sel.length === 1 ? sel[0] : null;
  const items: [string, string][] = [];
  if (sel.length) items.push(["transfer", p.isRemote ? "⬇ Download to left pane" : "⬆ Upload to S3"]);
  if (one?.dir) items.push(["open-dir", "📂 Open folder"]);
  if (one && !one.dir) items.push(["preview", "👁 Quick look"], ["open", "↗ Open with default app"]);
  if (!p.isRemote) items.push(["reveal", "🔎 Show in Finder / Explorer"]);
  if (one && p.isRemote && !one.dir) items.push(["link", "🔗 Copy 7-day link"]);
  if (one) items.push(["copy-path", "📋 Copy path"]);
  if (sel.length && p.isRemote) items.push(["-", ""], ["delete", "🗑 Delete from S3"]);
  items.push(["-", ""], ["mkdir", "＋ New folder"], ["refresh", "⟳ Refresh"]);
  if (!p.isRemote) items.push(["hidden", `${app.showHidden ? "✓ " : ""}Show hidden files`]);

  const m = $("ctx");
  m.innerHTML = items.map(([a, t]) => (a === "-" ? "<hr/>" : `<div data-a="${a}">${t}</div>`)).join("");
  m.dataset.pane = p.kind;
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.min(x, innerWidth - r.width - 4)}px`;
  m.style.top = `${Math.min(y, innerHeight - r.height - 4)}px`;
}

export function setupMenu() {
  document.addEventListener("contextmenu", (ev) => ev.preventDefault()); // no browser menu
  document.addEventListener("mousedown", (ev) => {
    if (!(ev.target as HTMLElement).closest("#ctx")) $("ctx").hidden = true;
  });
  $("ctx").addEventListener("click", (ev) => {
    const a = (ev.target as HTMLElement).closest<HTMLElement>("[data-a]")?.dataset.a;
    $("ctx").hidden = true;
    if (a) paneAction($("ctx").dataset.pane === "remote" ? remote : local, a);
  });
}
