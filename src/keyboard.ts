// Global keyboard shortcuts. The full table is in README.md — update both together.

import { deleteRemote, openItem, transfer, trashLocal } from "./actions";
import { active, closeView, cyclePane, local, remote, remotes, switchTo } from "./panes";
import { closePreview, openPreview, previewStep, pv, pvItem } from "./preview";
import { openSettings, splitView } from "./settings";
import { editPath, openPalette } from "./palette";
import { renameItem, undo } from "./fileops";
import { exp } from "./state";
import { $ } from "./util";

function onKey(ev: KeyboardEvent) {
  const mod = ev.metaKey || ev.ctrlKey;
  const inInput = ev.target instanceof Element && ev.target.matches("input, select, textarea");
  if (["settings", "ask", "palette", "compare"].some((id) => $<HTMLDialogElement>(id).open)) return;

  // Preview open: it owns the keyboard.
  if (pv) {
    const item = pvItem();
    if (ev.key === "Escape" || ev.key === " ") closePreview();
    else if (ev.key === "ArrowRight" || ev.key === "ArrowDown") previewStep(1);
    else if (ev.key === "ArrowLeft" || ev.key === "ArrowUp") previewStep(-1);
    else if (ev.key === "F5" || (mod && ev.key === "Enter")) item && transfer(pv.pane, [item]);
    else return;
    ev.preventDefault();
    return;
  }

  if (mod && ev.key === ",") return (ev.preventDefault(), openSettings());
  if (mod && ev.key === "r") return (ev.preventDefault(), active.load(true));
  if (mod && ev.key === "k") return (ev.preventDefault(), openPalette());
  if (mod && ev.key === "l") return (ev.preventDefault(), editPath(active));
  if (mod && ev.key === "f") return (ev.preventDefault(), active.toggleFilter(true));
  // File ops (experimental): F2 renames, ⌘Z undoes the last rename / move / duplicate.
  if (exp("file_ops") && !inInput) {
    if (ev.key === "F2" && active.selectedItems().length === 1)
      return (ev.preventDefault(), renameItem(active, active.selectedItems()[0]));
    if (mod && ev.key.toLowerCase() === "z" && !ev.shiftKey) return (ev.preventDefault(), undo());
  }
  if (ev.key === "Tab" && !inInput) return (ev.preventDefault(), cyclePane(ev.shiftKey));
  // Remote views (ADR-029): ⌘\ splits the focused view, ⌘W closes it (never the last one).
  if (mod && ev.key === "\\") return (ev.preventDefault(), splitView(remote));
  if (mod && ev.key.toLowerCase() === "w" && remotes.length > 1) return (ev.preventDefault(), closeView(remote));
  if (inInput) {
    if (ev.key === "Escape" || ev.key === "ArrowDown") {
      (ev.target as HTMLElement).blur();
      active.list.focus();
      if (ev.key === "ArrowDown") active.selectIndex(0);
    }
    return;
  }

  const p = active;
  const sel = p.selectedItems();
  const i = p.cursor();
  switch (true) {
    case ev.key === "F5" || (mod && ev.key === "Enter"):
      transfer(p, sel);
      break;
    case ev.key === "Enter" || (ev.key === "ArrowDown" && mod):
      if (sel.length === 1) p.activate(sel[0]);
      break;
    // Grid view (experimental): arrows move around the grid; Tab still switches panes.
    case p.isGrid && !mod && (ev.key === "ArrowLeft" || ev.key === "ArrowRight"):
      p.selectIndex(
        i < 0 ? 0 : Math.max(0, Math.min(p.view.length - 1, i + (ev.key === "ArrowLeft" ? -1 : 1))),
        ev.shiftKey,
      );
      break;
    case p.isGrid && !mod && (ev.key === "ArrowDown" || ev.key === "ArrowUp"):
      p.selectIndex(
        i < 0 ? 0 : Math.max(0, Math.min(p.view.length - 1, i + (ev.key === "ArrowUp" ? -p.cols : p.cols))),
        ev.shiftKey,
      );
      break;
    case ev.key === "ArrowDown": {
      const from = ev.shiftKey ? p.view.findLastIndex((e) => p.selected.has(e.key)) : i;
      p.selectIndex(i < 0 ? 0 : from + 1, ev.shiftKey);
      break;
    }
    case ev.key === "ArrowUp" && mod:
      p.up();
      break;
    case ev.key === "ArrowUp":
      p.selectIndex(i < 0 ? 0 : i - 1, ev.shiftKey);
      break;
    case ev.key === "ArrowLeft" && !mod:
      switchTo(local);
      break;
    case ev.key === "ArrowRight" && !mod:
      switchTo(remote);
      break;
    case ev.key === " ":
      if (sel.length === 1 && !sel[0].dir) openPreview(p, sel[0]);
      break;
    case (ev.key === "Backspace" && mod) || ev.key === "Delete":
      if (p.isRemote) deleteRemote(p);
      else trashLocal();
      break;
    case ev.key === "Backspace":
      p.up();
      break;
    case mod && ev.key === "a":
      p.selected = new Set(p.view.map((e) => e.key));
      p.paint();
      break;
    case mod && ev.key === "o":
      if (sel.length === 1 && !sel[0].dir) openItem(p, sel[0]);
      break;
    case ev.key === "Escape":
      p.selected.clear();
      p.paint();
      break;
    default:
      return;
  }
  ev.preventDefault();
}

export function setupKeyboard() {
  document.addEventListener("keydown", onKey);
}
