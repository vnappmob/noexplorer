// Global keyboard shortcuts. The full table is in README.md — update both together.

import { deleteRemote, openItem, transfer, trashLocal } from "./actions";
import { active, local, remote, switchTo } from "./panes";
import { closePreview, openPreview, previewStep, pv, pvItem } from "./preview";
import { openSettings } from "./settings";
import { editPath, openPalette } from "./palette";
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
  if (ev.key === "Tab" && !inInput) return (ev.preventDefault(), switchTo(active.other));
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
      if (p.isRemote) deleteRemote();
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
