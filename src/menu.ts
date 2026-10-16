// Right-click / "⋯" menu. Items are action names handled by `paneAction` (actions.ts).

import { paneAction, remoteName, TRASH_LABEL } from "./actions";
import { icon } from "./icons";
import type { Pane } from "./pane";
import { local, remote } from "./panes";
import { app } from "./state";
import { $, esc } from "./util";

type Item = [action: string, label: string, iconName: string];

/** Opens the menu at (x, y); `above` = grow upward from y (used by the bottom selection bar). */
export function showMenu(p: Pane, x: number, y: number, above = false) {
  const sel = p.selectedItems();
  const one = sel.length === 1 ? sel[0] : null;
  const items: (Item | "-")[] = [];
  if (sel.length)
    items.push([
      "transfer",
      p.isRemote ? "Download to this computer" : `Upload to ${remoteName()}`,
      p.isRemote ? "arrowLeft" : "arrowRight",
    ]);
  if (one?.dir) items.push(["open-dir", "Open folder", "folder"]);
  if (one && !one.dir) items.push(["preview", "Quick look", "eye"], ["open", "Open with default app", "external"]);
  if (!p.isRemote) items.push(["reveal", "Show in Finder / Explorer", "search"]);
  if (one && p.isRemote && !one.dir) items.push(["link", "Copy 7-day link", "link"]);
  if (one) items.push(["copy-path", "Copy path", "copy"]);
  items.push(["size", sel.some((e) => e.dir) ? "Calculate folder size" : "Size of this folder", "sigma"]);
  if (sel.length) items.push("-", ["delete", p.isRemote ? `Delete from ${remoteName()}` : TRASH_LABEL, "trash"]);
  items.push("-", ["mkdir", "New folder", "folderPlus"], ["refresh", "Refresh", "refresh"]);
  if (!p.isRemote) items.push(["hidden", app.showHidden ? "Hide hidden files" : "Show hidden files", "eye"]);

  const m = $("ctx");
  m.innerHTML = items
    .map((it) =>
      it === "-"
        ? "<hr/>"
        : `<div data-a="${it[0]}"${it[0] === "delete" ? ' class="danger"' : ""}>${icon(it[2])}<span>${esc(it[1])}</span></div>`,
    )
    .join("");
  m.dataset.pane = p.kind;
  m.hidden = false;
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.max(4, Math.min(x, innerWidth - r.width - 4))}px`;
  const top = above ? y - r.height : y;
  m.style.top = `${Math.max(4, Math.min(top, innerHeight - r.height - 4))}px`;
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
