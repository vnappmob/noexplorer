// One file list (local or remote): navigation, listing, sorting, filtering, selection, rendering.
// A Pane knows nothing about transfers/preview; it calls `paneHooks` for those (wired in main.ts).

import { invoke } from "@tauri-apps/api/core";
import { kindOf } from "./files";
import { azureEndpoint } from "./providers";
import { icon } from "./icons";
import { app } from "./state";
import type { BucketInfo, Crumb, Item, LocalListing, Profile, SortBy } from "./types";
import { $, esc, fail, fmtDate, fmtSize, plural, store } from "./util";

/** Row icon per file kind (files.ts `kindOf`). */
const FILE_ICON: Record<string, string> = {
  image: "image",
  video: "film",
  audio: "music",
  pdf: "doc",
  text: "doc",
  archive: "archive",
  other: "doc",
};

/** Rows kept in the DOM beyond the visible ones, on each side (see `drawRows`). */
const OVERSCAN = 15;
/** Grid view: grid rows kept beyond the visible ones. */
const GRID_OVERSCAN = 3;

export const paneHooks = {
  /** Enter / double-click on a file. */
  openFile: (_p: Pane, _e: Item): void => {},
  /** A remote view was created (bind its DOM events and pickers: main.ts). */
  created: (_p: Pane): void => {},
  /** A remote view changed account, bucket or folder (persist the layout). */
  moved: (_p: Pane): void => {},
  /** After every listing (entries are fresh objects, so computed sizes must be re-applied). */
  listed: (_p: Pane): void => {},
  /** "Try again" on a remote view's error: reconnect the account (settings.ts loadBuckets). */
  retry: (_p: Pane): void => {},
  /** A listing succeeded (favorites star and Back, favorites.ts / history.ts). */
  visited: (_p: Pane): void => {},
  /** About to leave the current folder (Back keeps its scroll and selection, history.ts). */
  leaving: (_p: Pane): void => {},
  /** `up()` is about to open `parent` (Back restores it as it was left, history.ts). */
  goingUp: (_p: Pane, _parent: string): void => {},
  /** Grid view: tiles were (re)drawn; fill their thumbnails (thumbs.ts). */
  drawn: (_p: Pane): void => {},
};

/** A search running or shown in a pane (search.ts / search.rs). */
export type Search = {
  id: string;
  query: string;
  root: string;
  running: boolean;
  dirs: number;
  files: number;
  skipped: number;
  capped: boolean;
  error: string | null;
};

/** Shown in a remote view's list instead of rows: an account being connected, or why it failed. */
export type Notice = { kind: "connecting" } | { kind: "error"; msg: string };

/** A listing slower than this gets the loading bar; faster ones would only flicker. */
const BUSY_AFTER = 150;

export class Pane {
  readonly el: HTMLElement;
  readonly list: HTMLElement;
  /** The opposite pane (set by panes.ts). */
  other!: Pane;
  /** Remote: prefix ("" = bucket root, else ends with "/"). Local: absolute folder path. */
  cwd = "";
  /** Local only: parent folder, null at a filesystem root. */
  parent: string | null = null;
  crumbs: Crumb[] = [];
  /** Local only: Home, common folders, disks and volumes (the "This computer" menu). */
  roots: Crumb[] = [];
  /** Everything in `cwd`. */
  entries: Item[] = [];
  /** Search results shown instead of the folder (search.ts); any normal load ends it. */
  search: Search | null = null;
  /** `entries` after filter + sort — what's on screen; row `data-i` indexes into this. */
  view: Item[] = [];
  selected = new Set<string>();
  /** Anchor row for shift-click / shift-arrow range selection. */
  anchor = -1;
  sortBy: SortBy;
  sortDir: number;
  /** Guards against out-of-order async listings. */
  private seq = 0;
  /** Virtualized list: only rows near the viewport are in the DOM, inside `.rows`, which is as tall
   * as all of them (a width change, like dragging the splitter, would otherwise re-lay out every row
   * of a big folder). `rowH` is measured from a rendered row; `win` = [first, end) rows in the DOM. */
  private rowsEl: HTMLElement | null = null;
  private rowH = 0;
  private win: [number, number] = [0, 0];
  /** Local: the automatic size scan of this folder finished; folders still without a size were
   * skipped (other disk, private folder not opened yet) and show "—". Set by sizes.ts. */
  sizesSettled = false;

  /** Remote views only: the account (profile id) and container this view shows (ADR-029). */
  account = "";
  bucket = "";
  /** Containers of `account` (the bucket picker). */
  buckets: BucketInfo[] = [];
  /** List or grid, per pane kind. */
  mode: "list" | "grid";
  /** Grid view: tiles per row (measured in `drawRows`). */
  cols = 1;
  /** Keys being deleted in the background: shown dimmed until the job ends (actions.ts). */
  pending = new Set<string>();
  /** Bumped on every account switch, so a slow answer for the previous account is dropped. */
  token = 0;
  notice: Notice | null = null;
  private busyTimer = 0;

  constructor(
    readonly kind: "local" | "remote",
    el: HTMLElement,
  ) {
    this.el = el;
    this.list = this.q(".list");
    // Screen readers: a list of files you can select several of.
    this.list.setAttribute("role", "listbox");
    this.list.setAttribute("aria-multiselectable", "true");
    this.list.setAttribute("aria-label", kind === "local" ? "Files on this computer" : "Remote files");
    this.mode = store.get(`${kind}.view`) === "grid" ? "grid" : "list";
    this.sortBy = (store.get(`${kind}.sortBy`) as SortBy) || "name";
    this.sortDir = store.get(`${kind}.sortDir`) === "desc" ? -1 : 1;
    // Watermark: a fixed background behind the rows (the list is transparent and sits above it),
    // so it stays visible however long the list is or wherever it's scrolled.
    const mark = this.isRemote ? "markCloud" : "markComputer";
    this.el.insertAdjacentHTML(
      "beforeend",
      `<div class="mark mark-${kind}" aria-hidden="true"><span>${icon(mark)}</span></div>`,
    );
    // Called straight from the events (no rAF: it pauses in covered windows); drawRows returns
    // early while the visible rows are already in the DOM.
    const redraw = () => this.drawRows();
    this.list.addEventListener("scroll", redraw, { passive: true });
    new ResizeObserver(redraw).observe(this.list);
    // Text size changes the row height.
    window.addEventListener("textsize", () => ((this.rowH = 0), this.drawRows(true)));
    this.list.addEventListener("click", (ev) => {
      const b = (ev.target as HTMLElement).closest<HTMLElement>("[data-notice]");
      if (b?.dataset.notice === "retry") paneHooks.retry(this);
      else if (b?.dataset.notice === "top") this.go("");
    });
  }

  /** Grid view is on (and allowed: the experiment can be turned off later). */
  get isGrid() {
    return this.mode === "grid";
  }

  /** Switches between list and grid, keeping the selection in view. */
  setMode(mode: "list" | "grid") {
    this.mode = mode;
    store.set(`${this.kind}.view`, mode);
    this.rowH = 0;
    this.applyMode();
    this.render();
    const i = this.cursor();
    if (i >= 0) this.reveal(i);
  }

  /** Header button and list class follow the mode. */
  applyMode() {
    const b = this.q<HTMLButtonElement>(".view-btn");
    b.innerHTML = icon(this.isGrid ? "list" : "grid");
    b.title = this.isGrid ? "List view" : "Grid view";
    this.list.classList.toggle("grid", this.isGrid);
    this.el.classList.toggle("grid-mode", this.isGrid);
  }

  /** `.loading` on the pane (the bar under the column header), only for slow work. */
  setBusy(on: boolean) {
    clearTimeout(this.busyTimer);
    if (on) this.busyTimer = window.setTimeout(() => this.el.classList.add("loading"), BUSY_AFTER);
    else this.el.classList.remove("loading");
  }

  /** Account switch: drop the old account's rows at once and say what we're waiting for. */
  showConnecting() {
    this.entries = [];
    this.selected.clear();
    this.notice = { kind: "connecting" };
    this.render();
    this.setBusy(true);
  }

  /** Remote views: show why listing failed, with Try again (and Go to top in a subfolder). */
  showError(e: unknown) {
    this.entries = [];
    this.notice = { kind: "error", msg: String(e) };
    this.setBusy(false);
    this.render();
  }

  q<T extends HTMLElement = HTMLElement>(sel: string) {
    return this.el.querySelector(sel) as T;
  }

  get isRemote() {
    return this.kind === "remote";
  }

  /** Remote views: this view's account, or null when it has none (or it was removed). */
  profile(): Profile | null {
    return app.settings.profiles.find((p) => p.id === this.account) ?? null;
  }

  /** `account` + `bucket` for backend commands about this view. */
  where() {
    return { account: this.account || null, bucket: this.bucket };
  }

  /** Re-lists `cwd`. `keepSelection` keeps still-existing selected rows (used after refreshes). */
  async load(keepSelection = false): Promise<void> {
    if (this.search) this.endSearch();
    const seq = ++this.seq;
    this.setBusy(true);
    try {
      if (this.isRemote) await this.loadRemote(seq);
      else await this.loadLocal(seq);
      if (seq !== this.seq) return;
      this.notice = null;
    } catch (e) {
      if (seq !== this.seq) return;
      this.setBusy(false);
      // Remote: say what failed and let the user retry. Jumping to the root hid timeouts and made
      // a slow server look like a random folder change.
      if (this.isRemote) return this.showError(e);
      this.entries = [];
      if (this.cwd) {
        // Folder may have vanished (an unplugged disk): fall back to the parent.
        this.cwd = this.parent ?? "";
        return this.load();
      }
      fail(e);
    }
    this.setBusy(false);
    if (!keepSelection) {
      this.selected.clear();
      this.anchor = -1;
    } else {
      const keys = new Set(this.entries.map((e) => e.key));
      this.selected = new Set([...this.selected].filter((k) => keys.has(k)));
    }
    this.render();
    paneHooks.listed(this);
    paneHooks.visited(this);
  }

  private async loadRemote(seq: number) {
    paneHooks.moved(this);
    const kind = this.profile()?.kind ?? "s3";
    this.q(".scheme").textContent =
      kind === "s3"
        ? "s3://"
        : kind === "nextcloud"
          ? "nextcloud://"
          : kind === "box"
            ? "box://"
            : kind === "azure"
              ? "azure://"
              : kind === "sftp"
                ? "sftp://"
                : "dav://";
    this.crumbs = [{ name: this.bucket || "—", path: "" }];
    let acc = "";
    for (const p of this.cwd.split("/").filter(Boolean)) {
      acc += p + "/";
      this.crumbs.push({ name: p, path: acc });
    }
    this.renderCrumbs();
    if (!this.bucket) {
      this.entries = [];
      return;
    }
    this.q(".count").textContent = "· loading…";
    const list = await invoke<Item[]>("list", { ...this.where(), prefix: this.cwd });
    if (seq === this.seq) this.entries = list;
  }

  private async loadLocal(seq: number) {
    this.sizesSettled = false;
    const l = await invoke<LocalListing>("list_local", { path: this.cwd || null, hidden: app.showHidden });
    if (seq !== this.seq) return;
    this.cwd = l.path;
    this.parent = l.parent;
    this.crumbs = l.crumbs;
    this.entries = l.entries;
    store.set("localPath", this.cwd);
    this.roots = l.roots;
    // Chip subtitle: the place we're in (longest matching root), e.g. "Home", "Downloads", "T7 Shield".
    const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
    const here = norm(this.cwd);
    const place = l.roots
      .filter((r) => here === norm(r.path) || here.startsWith(norm(r.path) + "/"))
      .sort((a, b) => b.path.length - a.path.length)[0];
    // Root names come with an emoji prefix from the backend ("🏠 Home"); the chip has its own icon.
    $("places-sub").textContent = place ? place.name.replace(/^\p{Extended_Pictographic}\s*/u, "") : "";
    this.renderCrumbs();
  }

  /** Full location of `cwd`: an absolute local path, `s3://bucket/prefix`, or a WebDAV URL. */
  fullPath() {
    if (!this.isRemote) return this.cwd;
    const p = this.profile();
    if (!this.bucket || !p) return "";
    const base = p.endpoint.replace(/\/+$/, "");
    if (p.kind === "nextcloud") return `${base}/remote.php/dav/files/${encodeURIComponent(p.access_key)}/${this.cwd}`;
    if (p.kind === "webdav") return `${base}/${this.cwd}`;
    if (p.kind === "box") return `box://${this.cwd}`;
    if (p.kind === "azure") return `${azureEndpoint(p)}/${this.bucket}/${this.cwd}`;
    if (p.kind === "sftp") return `${base}/${this.cwd}`;
    return `s3://${this.bucket}/${this.cwd}`;
  }

  go(path: string, keepSelection = false, noteLeaving = true) {
    if (noteLeaving) paneHooks.leaving(this);
    this.cwd = path;
    this.q<HTMLInputElement>(".filter").value = "";
    this.list.scrollTop = 0;
    return this.load(keepSelection);
  }

  /** Leaves search results (stopping a search still running); the caller lists the folder. */
  endSearch() {
    if (this.search?.running) invoke("cancel_search", { id: this.search.id }).catch(() => {});
    this.search = null;
    this.el.querySelector(".search-bar")?.remove();
  }

  /** Shows/hides the filter box in the header (⌘F, the magnifier button); hiding clears it. */
  toggleFilter(on: boolean) {
    const f = this.q<HTMLInputElement>(".filter");
    f.hidden = !on;
    this.q(".filter-btn").classList.toggle("on", on);
    if (on) f.focus();
    else if (f.value) {
      f.value = "";
      this.render();
    }
  }

  /** Goes to the parent folder and re-selects the folder we came out of. */
  up() {
    if (this.isRemote) {
      if (!this.cwd) return;
      const parts = this.cwd.split("/").filter(Boolean);
      parts.pop();
      this.goUpTo(parts.length ? parts.join("/") + "/" : "");
    } else if (this.parent) {
      this.goUpTo(this.parent);
    }
  }

  /** Opens `parent` with the folder we came out of selected (Back may restore more, history.ts). */
  private goUpTo(parent: string) {
    paneHooks.leaving(this); // before the selection changes below
    paneHooks.goingUp(this, parent);
    this.selected = new Set([this.cwd]);
    this.go(parent, true, false);
  }

  /** Path bar row: one button per folder level (click = go there, drop target while dragging). */
  renderCrumbs() {
    const el = this.q(".crumbs");
    const sep = `<span class="crumb-sep">${icon("chevRight")}</span>`;
    el.innerHTML = this.crumbs
      .map((c, i) => {
        const last = i === this.crumbs.length - 1 ? " last" : "";
        // The filesystem root ("/" or "C:\\") reads better as a disk icon.
        const label =
          !this.isRemote && i === 0 && /^([/\\]|[A-Za-z]:[\\/]?)$/.test(c.name) ? icon("drive") : esc(c.name);
        return `${i ? sep : ""}<button class="crumb${last}" data-p="${esc(c.path)}" title="${esc(c.path || c.name)}">${label}</button>`;
      })
      .join("");
    el.scrollLeft = el.scrollWidth; // keep the deepest folder visible
    // ↑ opens the folder tree (useful at the top too, tree.ts); ← goes up (history.ts).
    this.q<HTMLButtonElement>(".back").disabled = this.isRemote ? !this.cwd : !this.parent;
    this.q(".dirsize").textContent = "Size"; // folder size resets per folder
    this.q(".calc-size").classList.remove("has");
  }

  render() {
    // Search results are already the matches: the box holds the query, not a filter.
    const q = this.search ? "" : this.q<HTMLInputElement>(".filter").value.trim().toLowerCase();
    this.view = this.entries.filter((e) => !q || e.name.toLowerCase().includes(q));
    const { sortBy, sortDir } = this;
    this.view.sort((a, b) => {
      if (a.dir !== b.dir) return a.dir ? -1 : 1; // folders first
      let r = 0;
      if (sortBy === "size") r = a.size - b.size;
      else if (sortBy === "modified") r = (a.modified ?? 0) - (b.modified ?? 0);
      if (r === 0) r = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
      return r * sortDir;
    });
    this.el.querySelectorAll<HTMLElement>(".cols span").forEach((s) => {
      s.className = s.dataset.sort === sortBy ? (sortDir > 0 ? "asc" : "desc") : "";
    });

    this.rowsEl = null;
    const n = this.isRemote ? this.notice : null;
    const p = this.isRemote ? this.profile() : null;
    if (n && p) {
      const host =
        p.kind === "box"
          ? "box.com"
          : (p.kind === "azure" ? azureEndpoint(p) : p.endpoint).replace(/^https?:\/\//, "").replace(/\/+$/, "") ||
            "Amazon S3";
      this.list.innerHTML =
        n.kind === "connecting"
          ? `<div class="empty notice"><span class="spin" aria-hidden="true"></span><b>Connecting to ${esc(p.name)}…</b>${esc(host)}</div>`
          : `<div class="empty notice err"><b>Couldn’t open ${this.cwd ? "this folder" : esc(p.name)}</b><span class="msg">${esc(n.msg)}</span>
             <span class="actions"><button data-notice="retry" class="primary">Try again</button>${this.cwd ? `<button data-notice="top">Go to top</button>` : ""}</span></div>`;
    } else if (this.isRemote && !p) {
      this.list.innerHTML = `<div class="empty"><b>No remote account yet</b>Click the account button above to add S3, Nextcloud or WebDAV storage.</div>`;
    } else if (!this.view.length) {
      const hint = this.isRemote
        ? "Drag files here from the left pane or from Finder / Explorer to upload."
        : "Drag files here from the bucket to download.";
      const s = this.search;
      this.list.innerHTML = s
        ? `<div class="empty">${s.running ? `<span class="spin" aria-hidden="true"></span><b>Searching…</b>` : `<b>No matches</b>Nothing named like “${esc(s.query)}” in this folder or below.`}</div>`
        : `<div class="empty">${q ? "<b>No matches</b>" : `<b>Empty folder</b>${hint}`}</div>`;
    } else {
      // Keep the scroll position (re-sorts, refreshes, sizes arriving); go() resets it itself.
      const keep = this.list.scrollTop;
      this.list.innerHTML = `<div class="rows"></div>`;
      this.rowsEl = this.list.firstElementChild as HTMLElement;
      if (this.rowH) this.rowsEl.style.height = `${this.view.length * this.rowH}px`;
      this.list.scrollTop = keep;
      this.drawRows(true);
    }
    this.updateStatus();
    this.el.dispatchEvent(new Event("panechange", { bubbles: true }));
  }

  /** Redraws the rows in place (new sizes) without re-sorting. */
  repaintRows() {
    if (this.rowsEl) this.drawRows(true);
    this.updateStatus();
  }

  private rowHtml(i: number) {
    const e = this.view[i];
    const known = !e.dir || e.calc;
    const skipped = !known && this.sizesSettled;
    const size = known ? fmtSize(e.size) : skipped ? "—" : "";
    return `<div class="row${this.selected.has(e.key) ? " sel" : ""}${this.pending.has(e.key) ? " deleting" : ""}" role="option" aria-selected="${this.selected.has(e.key)}" data-i="${i}"${e.dir ? ` data-dir="${esc(e.key)}"` : ""}>
      <span class="name"><span class="ico${e.dir ? " dir" : ""}">${icon(e.dir ? "folder" : FILE_ICON[kindOf(e.name)])}</span><span>${esc(e.name)}</span>${e.where !== undefined ? `<span class="where" title="${esc(e.where)}">${esc(e.where || "·")}</span>` : ""}</span>
      <span class="size${e.calc ? " calc" : ""}"${skipped ? ' title="Not scanned automatically (another disk, or a private folder you haven’t opened yet). Click Σ to scan it."' : e.partial ? ' title="Some folders inside were skipped. Click Σ to include them."' : ""}>${size}</span>
      <span class="date">${fmtDate(e.modified)}</span>
    </div>`;
  }

  /** Grid view tile: same `.row` element (selection, drag & drop, menus work unchanged). */
  private cellHtml(i: number) {
    const e = this.view[i];
    const k = e.dir ? "" : kindOf(e.name);
    const thumb = k === "image" || k === "video" ? ` data-thumb="${k}"` : "";
    return `<div class="row cell${this.selected.has(e.key) ? " sel" : ""}${this.pending.has(e.key) ? " deleting" : ""}" role="option" aria-selected="${this.selected.has(e.key)}" data-i="${i}"${e.dir ? ` data-dir="${esc(e.key)}"` : ""}${thumb}>
      <span class="tile"><span class="ico${e.dir ? " dir" : ""}">${icon(e.dir ? "folder" : FILE_ICON[k])}</span></span>
      <span class="cname" title="${esc(e.name)}">${esc(e.name)}</span>
      <span class="csize">${e.dir ? (e.calc ? fmtSize(e.size) : "") : fmtSize(e.size)}</span>
    </div>`;
  }

  /** Grid view: like `drawRows`, by rows of `cols` tiles. */
  private drawGrid(force: boolean) {
    const box = this.rowsEl!;
    const n = this.view.length;
    const cell = parseFloat(getComputedStyle(this.list).getPropertyValue("--cell")) || 132;
    const cols = Math.max(1, Math.floor((this.list.clientWidth - 16) / cell));
    if (cols !== this.cols) ((this.cols = cols), (force = true));
    box.style.setProperty("--cols", String(cols));
    if (!this.rowH) {
      box.innerHTML = this.cellHtml(0);
      this.rowH = box.firstElementChild!.getBoundingClientRect().height || cell;
      force = true;
    }
    const h = this.rowH;
    const rows = Math.ceil(n / cols);
    const top = this.list.scrollTop - box.offsetTop;
    const first = Math.max(0, Math.floor(top / h));
    const end = Math.min(rows, Math.ceil((top + this.list.clientHeight) / h));
    if (!force && first * cols >= this.win[0] && Math.min(n, end * cols) <= this.win[1]) return;
    const a = Math.max(0, first - GRID_OVERSCAN) * cols;
    const b = Math.min(n, (end + GRID_OVERSCAN) * cols);
    this.win = [a, b];
    box.style.height = `${rows * h}px`;
    box.style.paddingTop = `${(a / cols) * h}px`;
    let html = "";
    for (let i = a; i < b; i++) html += this.cellHtml(i);
    box.innerHTML = html;
    paneHooks.drawn(this);
  }

  /** Puts the rows around the viewport in the DOM. Skipped while the visible rows are already
   * there, so scrolling rebuilds the window only every ~OVERSCAN rows. `force` = rows changed. */
  private drawRows(force = false) {
    const box = this.rowsEl;
    if (!box) return;
    if (this.isGrid) return this.drawGrid(force);
    const n = this.view.length;
    if (!this.rowH) {
      box.innerHTML = this.rowHtml(0);
      this.rowH = box.firstElementChild!.getBoundingClientRect().height || 36;
      force = true;
    }
    const h = this.rowH;
    const top = this.list.scrollTop - box.offsetTop;
    const first = Math.max(0, Math.floor(top / h));
    const end = Math.min(n, Math.ceil((top + this.list.clientHeight) / h));
    if (!force && first >= this.win[0] && end <= this.win[1]) return;
    const a = Math.max(0, first - OVERSCAN);
    const b = Math.min(n, end + OVERSCAN);
    this.win = [a, b];
    box.style.height = `${n * h}px`;
    box.style.paddingTop = `${a * h}px`;
    let html = "";
    for (let i = a; i < b; i++) html += this.rowHtml(i);
    box.innerHTML = html;
  }

  /** Scrolls row `i` into view (it may not be in the DOM yet). */
  private reveal(i: number) {
    if (!this.rowsEl || !this.rowH) return;
    const l = this.list;
    const top = this.rowsEl.offsetTop + (this.isGrid ? Math.floor(i / this.cols) : i) * this.rowH;
    if (top < l.scrollTop) l.scrollTop = top;
    else if (top + this.rowH > l.scrollTop + l.clientHeight) l.scrollTop = top + this.rowH - l.clientHeight;
    this.drawRows();
  }

  selectedItems() {
    return this.view.filter((e) => this.selected.has(e.key));
  }

  /** Column-header summary: item count and size, or the selection's count and size. */
  updateStatus() {
    const sel = this.selectedItems();
    const n = this.view.length;
    if (sel.length) {
      const sz = sel.reduce((s, e) => s + e.size, 0);
      this.q(".count").textContent = `· ${sel.length} of ${n} selected${sz ? ` · ${fmtSize(sz)}` : ""}`;
    } else {
      const total = this.view.reduce((s, e) => s + e.size, 0);
      this.q(".count").textContent = `· ${plural(n, "item")}${total ? ` · ${fmtSize(total)}` : ""}`;
    }
  }

  /** Repaints selection highlight without re-rendering rows. */
  paint() {
    this.list.querySelectorAll<HTMLElement>(".row").forEach((r) => {
      const sel = this.selected.has(this.view[+r.dataset.i!].key);
      r.classList.toggle("sel", sel);
      r.setAttribute("aria-selected", String(sel));
    });
    this.updateStatus();
  }

  selectIndex(i: number, extend = false) {
    if (!this.view.length) return;
    i = Math.max(0, Math.min(this.view.length - 1, i));
    if (extend && this.anchor >= 0) {
      const [a, b] = [Math.min(this.anchor, i), Math.max(this.anchor, i)];
      this.selected = new Set(this.view.slice(a, b + 1).map((e) => e.key));
    } else {
      this.selected = new Set([this.view[i].key]);
      this.anchor = i;
    }
    this.paint();
    this.reveal(i);
  }

  /** Index of the first selected row in `view`, or -1. */
  cursor() {
    return this.view.findIndex((e) => this.selected.has(e.key));
  }

  /** Enter / double-click: open folders here, hand files to the app. */
  activate(e: Item) {
    if (e.dir) this.go(e.key);
    else paneHooks.openFile(this, e);
  }
}
