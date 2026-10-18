// Entry point: wires DOM events to the modules and boots the app.
// Module map: see docs/ARCHITECTURE.md ("Frontend").

import { setupNativeLook } from "./native";
import { bindNav, favoritesMenuItems, visited } from "./nav";
import { fillThumbs } from "./thumbs";
import { setupBackups } from "./backups";
import { setupFileOps } from "./fileops";
import { setupExperimental } from "./experimental";
import { calcSize, mkdir, openItem, transfer } from "./actions";
import { applyAppearance, setupSplitter } from "./appearance";
import { setupDnd, armDrag } from "./dnd";
import { hydrateIcons } from "./icons";
import { checkInterrupted, setupJobs, watchView } from "./jobs";
import { setupKeyboard } from "./keyboard";
import { setupMenu, showMenu } from "./menu";
import { copyPath, editPath, setupPalette } from "./palette";
import { openMenu, setupPopover } from "./popover";
import { requestSizes, setupSizes } from "./sizes";
import { setupCompare } from "./compare";
import { paneHooks, type Pane } from "./pane";
import { closeView, local, setActive } from "./panes";
import { closePreview, openPreview, pv, pvItem } from "./preview";
import { bindPickers, initSettings, setupSettings, splitView } from "./settings";
import { setupUpdater } from "./updater";
import type { SortBy } from "./types";
import { $, fail, store } from "./util";

// Remote files preview on Enter; local files open in their default app.
paneHooks.openFile = (p, e) => (p.isRemote ? openPreview(p, e) : openItem(p, e));
paneHooks.listed = (p) => !p.isRemote && requestSizes();
paneHooks.drawn = fillThumbs;
paneHooks.visited = visited;
// Each remote view (ADR-029): DOM events, its account/bucket chips, split/close buttons.
paneHooks.created = (p) => {
  bindPane(p);
  bindPickers(p);
  watchView(p);
  p.q(".split-btn").onclick = () => splitView(p);
  p.q(".close-view").onclick = () => closeView(p);
};

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

  // Path bar: a crumb goes there; empty space switches to typing a path.
  p.q(".crumbs").addEventListener("click", (ev) => {
    const c = (ev.target as HTMLElement).closest<HTMLElement>(".crumb");
    if (c) p.go(c.dataset.p!);
    else editPath(p);
  });
  p.q(".up").onclick = () => p.up();
  p.q(".refresh").onclick = () => p.load(true);
  p.q(".mkdir-btn").onclick = () => mkdir(p);
  bindNav(p); // tabs, back/forward, favorites (experimental)
  // Grid view (experimental): the header button flips list ⇄ grid; the experiment can come and go.
  p.q(".view-btn").onclick = () => p.setMode(p.isGrid ? "list" : "grid");
  p.applyMode();
  window.addEventListener("experimental", () => (p.applyMode(), p.render()));
  window.addEventListener("settings-loaded", () => (p.applyMode(), p.render()));
  p.q(".copy-path").onclick = () => copyPath(p);
  const filter = p.q<HTMLInputElement>(".filter");
  filter.addEventListener("input", () => {
    p.selected.clear();
    p.render();
  });
  filter.addEventListener("blur", () => {
    if (!filter.value) p.toggleFilter(false);
  });
  p.q(".filter-btn").onclick = () => p.toggleFilter(filter.hidden !== false);
  // "⋯" in the header and in the selection bar open the same menu as a right-click.
  p.q(".more-btn").onclick = (ev) => {
    const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    setActive(p);
    showMenu(p, r.left, r.bottom + 6);
  };
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
  p.q(".calc-size").onclick = () => calcSize(p);
}

/** "This computer ▾": Home, common folders, disks and volumes. */
function openPlacesMenu() {
  const items = local.roots.map((r) => {
    const name = r.name.replace(/^\p{Extended_Pictographic}\s*/u, "");
    const ic = /Home$/.test(r.name)
      ? "home"
      : /Downloads$/.test(r.name)
        ? "download"
        : /Desktop$/.test(r.name)
          ? "desktop"
          : /^💽/u.test(r.name)
            ? "drive"
            : "folder";
    return { label: name, sub: r.path, icon: ic, checked: local.cwd === r.path, run: () => local.go(r.path) };
  });
  const favs = favoritesMenuItems("local", (f) => local.go(f.path));
  openMenu($("places-chip"), favs.length ? [{ header: "Favorites" }, ...favs, { sep: true }, ...items] : items, {
    width: 260,
  });
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
  applyAppearance();
  setupSplitter();
  if (/Mac/.test(navigator.userAgent)) document.body.classList.add("mac"); // room for the traffic lights
  hydrateIcons();
  bindPane(local);
  $("places-chip").onclick = openPlacesMenu;
  bindPreview();
  setupPopover();
  setupPalette();
  setupSizes();
  setupCompare();
  setupMenu();
  setupSettings();
  setupExperimental();
  setupFileOps();
  setupBackups();
  setupNativeLook();
  setupKeyboard();
  window.addEventListener("focus", () => local.load(true)); // pick up changes made outside the app
  setActive(local);

  await setupJobs();
  await setupDnd();

  local.cwd = store.get("localPath") ?? "";
  const localLoad = local.load();
  await initSettings();
  await localLoad;
  setupUpdater();
  checkInterrupted();
  local.list.focus();
}

main().catch(fail);
