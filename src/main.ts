// Entry point: wires DOM events to the modules and boots the app.
// Module map: see docs/ARCHITECTURE.md ("Frontend").

import { openItem, paneAction, transfer } from "./actions";
import { setupDnd, armDrag } from "./dnd";
import { setupJobs } from "./jobs";
import { setupKeyboard } from "./keyboard";
import { setupMenu, showMenu } from "./menu";
import { paneHooks, type Pane } from "./pane";
import { local, remote, setActive } from "./panes";
import { closePreview, openPreview, pv, pvItem } from "./preview";
import { loadBuckets, openSettings, setupSettings } from "./settings";
import { app } from "./state";
import type { Config, SortBy } from "./types";
import { $, fail, store } from "./util";
import { invoke } from "@tauri-apps/api/core";

// Remote files preview on Enter; local files open in their default app.
paneHooks.openFile = (p, e) => (p.isRemote ? openPreview(p, e) : openItem(p, e));

function bindPane(p: Pane) {
  p.el.addEventListener("mousedown", () => setActive(p));
  p.list.addEventListener("focus", () => setActive(p));

  // Selection: click, ⌘/Ctrl-click toggle, Shift-click range. Also arms a drag.
  p.list.addEventListener("mousedown", (ev) => {
    const row = (ev.target as HTMLElement).closest<HTMLElement>(".row");
    if (!row) {
      if (ev.button === 0) {
        p.selected.clear();
        p.paint();
      }
      return;
    }
    const i = +row.dataset.i!;
    const key = p.view[i].key;
    if (ev.button === 2) {
      if (!p.selected.has(key)) p.selectIndex(i);
      return;
    }
    let collapseTo = -1;
    if (ev.metaKey || ev.ctrlKey) {
      p.selected.has(key) ? p.selected.delete(key) : p.selected.add(key);
      p.anchor = i;
      p.paint();
    } else if (ev.shiftKey) {
      p.selectIndex(i, true);
    } else if (p.selected.has(key)) {
      collapseTo = i; // keep the multi-selection so it can be dragged
    } else {
      p.selectIndex(i);
    }
    armDrag(p, ev.clientX, ev.clientY, collapseTo);
  });
  p.list.addEventListener("dblclick", (ev) => {
    const row = (ev.target as HTMLElement).closest<HTMLElement>(".row");
    if (row) p.activate(p.view[+row.dataset.i!]);
  });
  p.list.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    setActive(p);
    showMenu(p, ev.clientX, ev.clientY);
  });

  p.q(".crumbs").addEventListener("click", (ev) => {
    const a = (ev.target as HTMLElement).closest<HTMLElement>("a");
    if (a) p.go(a.dataset.p!);
  });
  p.q(".up").onclick = () => p.up();
  p.q(".refresh").onclick = () => p.load(true);
  p.q(".filter").addEventListener("input", () => {
    p.selected.clear();
    p.render();
  });
  p.el.querySelectorAll<HTMLElement>(".cols span").forEach((s) =>
    s.addEventListener("click", () => {
      const by = s.dataset.sort as SortBy;
      p.sortDir = by === p.sortBy ? -p.sortDir : 1;
      p.sortBy = by;
      store.set(`${p.kind}.sortBy`, p.sortBy);
      store.set(`${p.kind}.sortDir`, p.sortDir > 0 ? "asc" : "desc");
      p.render();
    }),
  );
  p.el.querySelectorAll<HTMLElement>("[data-act]").forEach((b) => (b.onclick = () => paneAction(p, b.dataset.act!)));
}

function bindPreview() {
  $("pv-close").onclick = closePreview;
  $("pv-transfer").onclick = () => pv && pvItem() && transfer(pv.pane, [pvItem()!]);
  $("pv-open").onclick = () => pv && pvItem() && openItem(pv.pane, pvItem()!);
  $("preview").addEventListener("mousedown", (ev) => {
    if (ev.target === $("preview")) closePreview(); // click on the backdrop
  });
}

async function main() {
  bindPane(local);
  bindPane(remote);
  local.q<HTMLSelectElement>(".roots").addEventListener("change", (ev) => {
    const v = (ev.target as HTMLSelectElement).value;
    if (v) local.go(v);
  });
  bindPreview();
  setupMenu();
  setupSettings();
  setupKeyboard();
  window.addEventListener("focus", () => local.load(true)); // pick up changes made outside the app
  setActive(local);

  await setupJobs();
  await setupDnd();

  local.cwd = store.get("localPath") ?? "";
  const localLoad = local.load();
  app.cfg = await invoke<Config | null>("get_config");
  if (!app.cfg) {
    remote.render();
    openSettings(); // first run
  } else {
    await loadBuckets();
  }
  await localLoad;
  local.list.focus();
}

main().catch(fail);
