# Architecture

This is the deep reference. For orientation, read [AGENTS.md](../AGENTS.md) first.

## Overview

```
┌──────────────────────────── NoExplorer process ─────────────────────────────┐
│  Webview (src/*.ts, index.html)                Rust core (src-tauri/src)     │
│  ┌──────────────┐  ┌──────────────┐  invoke()  ┌─────────────────────────┐   │
│  │ local Pane   │  │ remote Pane  │ ─────────▶ │ #[tauri::command]s      │   │
│  │ (left)       │  │ (right)      │            │ config/remote/transfer/ │   │
│  └──────────────┘  └──────────────┘ ◀───────── │ local/jobs              │   │
│   dnd · preview · jobs · settings   "job" event└───────┬───────────┬─────┘   │
│                                                        │           │         │
└────────────────────────────────────────────────────────┼───────────┼─────────┘
            presigned GET (img/video/pdf src) ◀───┐      │ aws-sdk-s3│ std/tokio fs
                                                  │      ▼           ▼
                                          S3-compatible server   Local disk
                                          (MinIO, SeaweedFS, AWS)
```

- There is **one process and one window.** The Rust core owns every S3 call and all disk I/O.
  The webview talks to S3 directly in only one case: an `<img>`, `<video>`, `<audio>` or
  `<iframe>` loading a **presigned URL** for the preview.
- Long work (upload/download) runs as **jobs** on Tauri's async runtime (tokio). Short work
  (list, presign, read_text, …) is a plain async command that returns a value.
- **No framework on the frontend.** It uses DOM APIs and template strings. Every list
  re-render goes through `Pane.render()`, and selection changes use `Pane.paint()`, which
  doesn't rebuild the rows.

## Backend modules (`src-tauri/src`)

| Module | Owns | Key items |
|---|---|---|
| `lib.rs` | App bootstrap | `run()`: plugins (opener, dialog), loads the config, wipes the temp dir, registers commands |
| `config.rs` | Accounts | `Profile` (`kind` = provider), `Settings` (upsert/remove/active), `parse_settings` (v1→v2 migration), secrets in the keychain (ADR-020), `is_aws` / `region_from_endpoint` / `effective_region` (ADR-017) |
| `provider.rs` | Provider interface | `enum Remote` (static dispatch, ADR-025): `containers`, `list`, `list_all`, `link`, `read_head`, `mkdir`, `delete`, `upload_file`, `download_file`; `Entry`, `BucketInfo` |
| `s3.rs` | S3 provider | `make_client(_in)` (path-style, checksums), `detect_bucket_region`, `S3` (per-bucket region cache, `client_for`), multipart upload, streamed `.part` downloads, tuning constants |
| `webdav.rs` | Nextcloud / WebDAV provider | `Dav` (own client: PROPFIND/GET Range/PUT/MKCOL/DELETE/MOVE), Nextcloud chunked upload v2 and OCS shares, `parse_multistatus`, Login Flow v2 (`nextcloud_login`, `cancel_login`), shared `http()` client (ADR-026) |
| `proxy.rs` | Preview proxy | The `nxr://` scheme: streams `Remote::read_range` to the webview with Range + CORS for providers without presigned URLs, local files (`~local`) and grid thumbnails (`~thumb`); answers on the main thread (ADR-035) |
| `thumbs.rs` | Grid thumbnails | 256 px JPEG/PNG thumbnails, LRU caches in memory (16 MB) and on disk (48 MB); trims WebKit's disk cache at launch (ADR-036, ADR-037) |
| `box_api.rs` | Box provider (beta) | `BoxCom`: token refresh with rotation (keychain), path → ID cache, multipart and session uploads with SHA-1, versions, trash deletes, shared links; `box_login` (loopback OAuth) (ADR-027) |
| `compare.rs` | Compare folders | `compare` / `diff`: local walk (junk, symlinks), `Remote::list_files` with hashes, pairing, parallel hash checks (MD5 / multipart ETag / SHA-1), `"compare-progress"` (ADR-028) |
| `state.rs` | Shared state | `AppState { settings, remotes, cancels }`. **`remote(account)`** returns the cached `Arc<Remote>` for a profile id (active when `None`); `set_settings()` drops the clients of changed accounts only |
| `jobs.rs` | Background jobs | `Job` (atomic counters, errors, cancel flag), `Job::spawn` (250 ms ticker + final event), `cancel` |
| `remote.rs` | Browsing commands | Thin wrappers over `Remote`: `list_buckets`, `list`, `remote_size`, `presign`, `read_text`, `mkdir`, `delete` |
| `transfer.rs` | Transfer jobs | `upload`, `download`, `open_object`: expand files, schedule `FILE_CONCURRENCY` at a time, call `Remote::upload_file` / `download_file` |
| `local.rs` | Local disk (left pane) | `list_local` (+ crumbs and Places roots; tells `sizes` which private folders were opened), `local_size`, `read_local_text`, `mkdir_local`, `open_local`, `reveal` |
| `scan.rs` | Folder-size engine | Parallel scanner (`scan`, `Totals`, `Sink`), macOS `getattrlistbulk` fast path, generic `read_dir` (ADR-022) |
| `sizes.rs` | Live size index | `SizeIndex` (index of watched roots, focus folder, batched `"dir-sizes"` events), polite auto-scan policy, `notify` watchers, `folder_sizes` / `stop_sizes` |
| `trash.rs` | Move to Trash | `trash_local`: NSFileManager (macOS), SHFileOperationW (Windows), freedesktop spec (Linux) (ADR-023) |
| `autostart.rs` | Launch at login | `launcher()` via `auto-launch` (bundle id on macOS, AppImage path on Linux), `refresh()` rewrites an enabled entry at startup (ADR-018) |
| `tray.rs` | Tray / menu-bar icon | Menu (status line, up to 5 transfer rows, the S3 accounts (✓ = active) or "Add S3 Account…", Check for Updates / Quit), macOS % title, `show_main`/`hide_main` (Dock icon toggles via activation policy), `on_job` fed by `jobs.rs`, `set_accounts` fed by `config.rs` |
| `util.rs` | Pure helpers | `err`/`humanize` (S3 code + message + hint for common mistakes), `safe_rel` (traversal guard), `unique_path`, `job_label`, `file_name`, `is_text_like` |

## IPC commands

Arguments are passed from JS as an object. Tauri converts camelCase keys to snake_case.
Every command returns `Result<T, String>`, and the error string is shown to the user as-is.
Remote commands (`list_buckets`, `list`, `remote_size`, `presign`, `read_text`, `mkdir`, `delete`,
`upload`, `download`, `open_object`) also take an optional **`account`** (a profile id). Omitted
means the active account (ADR-025).

| Command | Args | Returns | Module | Notes |
|---|---|---|---|---|
| `get_settings` | – | `Settings` | config | All accounts plus the active id, with secrets blanked (`redacted`). No profiles means first run → the UI opens Settings |
| `test_profile` | `profile` | `string[]` buckets | config | An empty secret uses the saved one. Falls back to `HeadBucket(profile.bucket)` if ListBuckets is denied |
| `save_profile` | `profile` | `Settings` | config | Upsert (an empty `id` means new; an empty secret keeps the saved one), make it active, store the secret in the OS keychain, write `config.json` (0600), rebuild the client |
| `delete_profile` | `id` | `Settings` | config | Also deletes its keychain entry. If it was active, the first remaining account becomes active |
| `set_active_profile` | `id` | `Settings` | config | Switches the remote pane's account. Running jobs keep their own client |
| `save_general` | `general` | `Settings` | config | Settings → General (`run_in_background`, `check_updates`, `auto_sizes`) |
| `autostart_enabled` | – | `bool` | autostart | Is "Launch at login" registered with the OS |
| `set_autostart` | `enabled` | – | autostart | macOS: LaunchAgent **with `AssociatedBundleIdentifiers`**; Windows: Run key; Linux: `.desktop` autostart. Starts with `--hidden` |
| `list_buckets` | – | `BucketInfo[] {name, region}` | config | Buckets of the active account. `region` comes from AWS ListBuckets (null elsewhere) and is shown as a badge in the bucket menu |
| `remote_size` | `bucket, prefix` | `SizeInfo {bytes, files}` | remote | Recursive. `""` = whole bucket. Lists every object, so it's slow on huge folders |
| `local_size` | `path` | `SizeInfo` | local | Recursive walk; symlinks are not followed (single folder; the local pane uses `folder_sizes`) |
| `folder_sizes` | `path, dirs[], auto` | `DirSizes` | sizes | Sets the focus folder, answers `dirs` + `path` from the index, starts a background scan if needed (`auto` = polite rules; false = Σ, scans everything). More arrives as `"dir-sizes"` |
| `search_local` | `path, query, hidden` | `string` (id) | search | Searches every folder below `path` (parallel walker on `scan::read_dir`, polite folder rules); results stream as `"search"` events |
| `search_remote` | `account, bucket, prefix, query` | `string` (id) | search | Searches below `prefix` from the provider's full listing (files, and folders from their paths); one `"search"` event |
| `cancel_search` | `id` | – | search | Stops a running search |
| `stop_sizes` | – | – | sizes | Clears the focus and cancels the running scan (auto sizes turned off) |
| `volume_info` | `path` | `VolumeInfo {name, total, available}` | local | The disk under `path`, for the storage bar. macOS: NSURL volume resource values (available = "important usage", as System Settings shows); Windows: `GetDiskFreeSpaceExW`; Linux: `statvfs` |
| `nextcloud_login` | `server, id?, name` | `Settings` | webdav | Login Flow v2: opens the browser, polls until approved (20 min), saves the account with its app password in the keychain, makes it active |
| `cancel_login` | – | – | webdav | Stops a waiting `nextcloud_login` or `box_login` |
| `box_login` | `client_id, client_secret, id?, name` | `Settings` | box_api | Opens Box's authorize page, waits on `127.0.0.1:53682/callback`, exchanges the code, saves the account (client secret + refresh token in the keychain) |
| `compare` | `id, account?, bucket, prefix, local, verify` | `Diff` | compare | Local folder vs remote folder; emits `"compare-progress"`; `cancel(id)` stops it |
| `upload_rel` | `account?, bucket, prefix, root, rels[]` | job id | transfer | Uploads `root/rel` to `prefix + rel` (Compare → Upload) |
| `copy_remote` | `srcAccount?, srcBucket, srcPrefix, items[{key,size}], dstAccount?, dstBucket, dstPrefix` | job id | transfer | Remote → remote (split views, ADR-029). Same S3 account: server-side CopyObject (≤ 5 GB); otherwise relayed through a temp file per object |
| `download_rel` | `account?, bucket, prefix, root, items[{key: rel, size}]` | job id | transfer | Downloads `prefix + rel` to `root/rel` (`safe_rel`, never overwrites) |
| `trash_local` | `paths[]` | count | trash | Moves local items to the OS Trash / Recycle Bin. Refuses `/`, home and its parents |
| `list` | `bucket, prefix` | `Entry[]` | remote | One folder level. Folders have `dir: true` and a key ending in `/` |
| `presign` | `bucket, key, secs, inline` | URL | remote | `secs` is clamped to 60 s–7 d. `inline` forces Content-Type/Disposition for preview |
| `read_text` | `bucket, key, max` | string | remote | Range GET `0..max`, decoded as lossy UTF-8 |
| `create_bucket` | `account, name` | – | remote | Creates a bucket (S3-compatible, incl. GCS with its project) or an Azure container; name checked first (experimental `create_buckets`) |
| `mkdir` | `bucket, prefix, name` | – | remote | Writes an empty `prefix+name/` marker object |
| `delete` | `account?, bucket, keys[]` | job id | remote | Background job (kind `delete`): folders with everything inside; progress in `files_done/files_total` (S3: per batch of up to 1000, WebDAV/Box: per item); cancellable between steps |
| `remote_move` | `account?, bucket, pairs[[from,to]], copy` | job id | fileops | Rename / move (`copy` false) or duplicate (true) inside one container, server-side (`Remote::transfer_within`); job kind `move` / `dup`; never overwrites |
| `rename_local` | `path, name` | new path | fileops | Same-folder rename; refuses a taken name |
| `move_local` | `paths, dest` | `{done, errors}` | fileops | Same-disk move; refuses taken names, other disks and moving a folder into itself |
| `duplicate_local` | `paths` | `{done, errors}` | fileops | Copies next to the original as "name copy.ext" (symlinks skipped) |
| `pause` | `id, on` | – | resume | Pauses / resumes a running job (data flow waits in `Flow`) |
| `interrupted_jobs` | – | `PendingJob[]` | resume | Transfers journaled in `pending.json` that didn't finish (not running now) |
| `resume_job` | `id` | job id | resume | Starts it again with `resume: true`: uploads skip remote files with the same size, downloads skip complete files and continue `.part` files |
| `dismiss_interrupted` | `id` | – | resume | Forgets an interrupted transfer |
| `cert_fingerprint` | `endpoint` | `"AB:CD:…"` | tls | SHA-256 of the certificate an https:// server presents, read without trusting it (to pin it in `Profile.cert_pin`) |
| `upload` | `bucket, prefix, paths` | job id | transfer | `paths` are absolute local files or folders. Folders keep their structure |
| `download` | `bucket, prefix, items[{key,size}], dest?` | job id | transfer | Writes to `dest` (the left pane's folder), else to the configured or OS Downloads folder |
| `open_object` | `bucket, key` | – | transfer | Downloads to `$TMP/noexplorer/<uuid>/` and opens with the default app |
| `cancel` | `id` | – | jobs | Sets the job's cancel flag. In-flight multipart uploads are aborted |
| `list_local` | `path?, hidden` | `LocalListing` | local | No `path` means the home folder. Includes `crumbs` and `roots` (Places) |
| `read_local_text` | `path, max` | string | local | |
| `mkdir_local` | `path, name` | – | local | Rejects `/`, `\`, `.`, `..` |
| `open_local` | `path` | – | local | Opens with the OS default app |
| `reveal` | `path` | – | local | Shows the item in Finder, Explorer or the Linux file manager |
| `log_file` | – | `string` | diag | Path of the diagnostic log (created if missing), for Settings → About → Show log file |
| `show_after_relaunch` | – | – | tray | Leaves a marker so the next start shows the window even with `--hidden` (the updater relaunches with the same arguments) |

## Events

**`"job"`** is emitted by `jobs.rs` every 250 ms while a job runs, plus once at the end.
The TS mirror is `JobEvent` in `src/types.ts`.

```jsonc
{
  "id": "uuid", "kind": "up" | "down" | "copy" | "delete" | "move" | "dup", "label": "a.txt, b.txt +3 more",
  "account": "profile-id", "bucket": "demo", "prefix": "backups/",   // remote side (copy: the destination)
  "src_account": null, "src_bucket": null,         // copy: the source view (for the beam)
  "src_dir": "/Users/me/Projects",                 // upload: local folder; copy: source prefix (lanes)
  "paused": false,                                 // experimental resume: paused from the UI
  "files_done": 3, "files_total": 5, "bytes_done": 123, "bytes_total": 456,
  "status": "running" | "done" | "error" | "cancelled",
  "error": null | "2 errors. First: x.bin: …",     // per-file failures don't stop the job
  "dest": "/Users/me/Downloads/x.bin",             // download: what "Show" reveals
  "local_dir": "/Users/me/Downloads"               // download: refresh the left pane if it shows this folder
}
```

**`"dir-sizes"`** is emitted by `sizes.rs` (batched, ~100 ms) while the local pane's folder is
being scanned or changes on disk. The TS mirror is `DirSizes`; `folder_sizes` returns the same
shape.

```jsonc
{
  "folder": "/Users/me/Projects",                 // only sizes for the folder on screen
  "sizes": [{ "path": "/Users/me/Projects/app", "bytes": 1234, "files": 10, "dirs": 2, "skipped": 0 }],
  "total": { "bytes": 99, "files": 9, "dirs": 1, "skipped": 1 } | null,   // skipped > 0: a lower bound (tooltip)
  "scanning": true,                               // more will come
  "relist": false                                 // the folder's own entries changed: list it again
}
```

**`"compare-progress"`** `{id, phase: "listing" | "hashing" | "done", done, total}` (bytes while
hashing) is emitted by `compare.rs` for the Compare dialog.

**`"tray-check-updates"`** is emitted by `tray.rs` when the tray's "Check for Updates…" is
clicked. `updater.ts` runs a manual check. **`"tray-open-account"`** (payload: profile id) and
**`"tray-add-account"`** come from the tray's account items; `settings.ts` switches the remote
pane to that account (or opens the account form) and focuses it.

The UI also uses Tauri's built-in **drag-drop** events (`getCurrentWebview().onDragDropEvent`)
for files dragged in from the OS.

## Data flows

**Upload (drag local → remote, F5, or OS drop on the right pane)**
`dnd.ts`/`keyboard.ts` → `transfer(local, items, targetPrefix)` → `invoke("upload")` →
`collect_uploads` walks folders (skipping `.DS_Store`, `Thumbs.db`, `desktop.ini`) →
6 files in parallel → each file is either a single `PutObject` (≤ 16 MB) or a multipart upload
(parts ≥ 8 MB, ≤ 9 000 parts, 4 in flight; aborted on error) → `"job"` events →
`jobs.ts` re-lists the remote pane when the job's prefix is under the current one.

**Download (drag remote → local, F5)**
`transfer(remote, items, targetDir)` → `invoke("download", { dest })` → folders expand via
`list_all` → local paths come from `safe_rel(key − currentPrefix)` → a top-level name collision
becomes `unique_path` → `GetObject` is streamed into `name.part` and renamed on success →
`"job"` events → the left pane refreshes when `local_dir` is under its folder.

**Preview (Space)**
Remote image/video/audio/pdf: `presign(inline=true)` → a media element's `src`.
Local media: `localSrc(path)` (`nxr://localhost/~local/f/…`, served by `proxy.rs`; Tauri's asset
protocol is off, ADR-035). Text, local or remote: first 512 KB as a
`<pre>` filled through `textContent`. Anything else shows "Open with default app"
(`open_object`/`open_local`).

**Accounts (profiles)**
`config.json` holds `{profiles[], active, download_dir}` (`config::Settings`). The backend keeps
one S3 client, for the active profile. Settings → Accounts: **Save & use** runs `test_profile`
first, and only on success calls `save_profile`. That returns the new `Settings`, and
`applySettings()` then reloads the account picker, the bucket list and the remote pane. The last
bucket and prefix are remembered **per account** (localStorage `bucket@<id>`, `prefix@<id>`).
v1 files (one flat connection) are migrated in memory by `config::parse_settings` and written
back in v2 format on the next save.
Secret keys live in the OS credential store (`secrets.rs`, ADR-020): `config.json` keeps
`keychain: true` and an empty `secret_key`, unless no store is available. The backend fills
secrets in on load (`load_settings`, which also migrates secrets left in the file) and never
sends them to the UI: every command returns `redacted()` settings, and an empty secret from the
form means "keep the saved one".

**Folder sizes**
Local pane (ADR-022): after every listing, `paneHooks.listed` → `sizes.ts::requestSizes` →
`folder_sizes(path, dirs, auto: true)` (when Settings → General → auto sizes is on). Known
totals come back at once. The rest stream as `"dir-sizes"` into the rows (`Item.calc`,
`Item.partial` → a tooltip) and the path-bar total. The storage bar under the pane
(`footer.disk`, `sizes.ts::drawDisk`) shows the folder's total against its disk from
`volume_info`: this folder / other used / available. Rows are redrawn in place (`Pane.repaintRows`), and
re-sorted only when the scan ends if sorted by size. `relist` re-lists the folder (live view).
Σ on the local pane is `folder_sizes(auto: false)`: a full scan including skipped folders.
Remote pane: Σ calls `remote_size` for the selected folders (or the current one) as before.

**Move to Trash**
`actions.ts::trashLocal` (⌘⌫ / Delete, context menu, ⌘K) → `trash_local` → toast, re-list,
select the next row. The size index follows through its watcher.

**Background running and the tray**
`lib.rs` intercepts `CloseRequested`: when `general.run_in_background` is on (the default) it
calls `prevent_close()` + `tray::hide_main()`. On macOS that also switches to the `Accessory`
activation policy, so there's no Dock icon. Jobs keep running, and every tick goes to the UI
(`"job"`) **and** to `tray::on_job`, which updates the menu rows in place and rebuilds the menu
only when the set of rows changes. There are no system notifications (removed in 0.4.8: they
behave differently on every platform and ask for permission). Quit happens only from the tray, ⌘Q, or with
run_in_background off. `single-instance` focuses the running app on a second launch, and
`RunEvent::Reopen` (Dock click) shows the window. Launch at login (`autostart.rs`) passes
`--hidden`, so the app starts in the tray. The tray lists the S3 accounts (`tray::set_accounts`,
called on every settings commit). Choosing one shows the window and opens that account in the
right pane.

**Auto-update (OTA)**
The release workflow signs the update bundles (`*.app.tar.gz`, `*-setup.exe`, `*.AppImage`
→ `*.sig`) with the updater private key (secret `TAURI_SIGNING_PRIVATE_KEY`) and uploads
`latest.json`. Installed apps fetch
`https://github.com/vnappmob/noexplorer/releases/latest/download/latest.json`
(`tauri.conf.json → plugins.updater`), verify the signature against the embedded `pubkey`,
then `downloadAndInstall()` + `relaunch()` (`updater.ts`). Drafts are invisible to
`releases/latest`, so an update ships only when a release is **published**.

**Window and pane layout (v0.3)**
On macOS the title bar is an overlay (`titleBarStyle: Overlay`): the traffic lights sit in
`#topbar`'s `.tl-space`, and `data-tauri-drag-region` makes the bar draggable. Each pane has two
header rows. Row 1 (`.src`) is the source: the "This computer" chip and Places menu, or the
account chip › bucket chip. Row 2 (`.pathbar`) is the path: up, clickable crumbs (empty space
or ⌘L switches to typing a path), copy, Σ. The column header shows the item count, or the selection count and size.
Transfers show in the activity card, plus the beam: an SVG arc over `#panes`, computed from the
real element rects on every job tick and resize. Lists are virtualized (ADR-021): `.list > .rows`
is as tall as all rows, and `Pane.drawRows` keeps only the rows near the viewport in the DOM. The watermark (`.mark`) is added once by the `Pane` constructor
as an absolutely positioned background at the bottom of the pane; the transparent `.list` scrolls
over it.

**Split remote views (v0.4.4, ADR-029)**
`#remotes` stacks 1–3 remote `Pane`s cloned from `<template id="remote-tpl">`, separated by
`.hsplit` handles (heights are flex-grow ratios). Each view owns `account`, `bucket` and
`buckets`; every backend call passes `view.where()` (`{account, bucket}`), so nothing reads a
global account or bucket. `panes.remote` is an ES live binding to the view that last had focus:
local → remote actions (F5, OS drops onto the left pane's target, Compare) go there. A job
refreshes every view showing its destination, and the beam finds its ends by account + bucket.

**Appearance**
`appearance.ts` stores theme (`system`/`light`/`dark`), text size (13/14/16/18 px) and the pane
split (%) in localStorage. It applies them as `<html data-theme>`, the `--fs` CSS variable and
`#panes --left`. `styles.css` sizes everything from `--fs`. The native window theme follows via
`getCurrentWindow().setTheme()`.

## Frontend modules (`src/`)

| Module | Owns |
|---|---|
| `main.ts` | Boot sequence and per-pane DOM bindings (row selection, dblclick, crumbs, sort headers, footer buttons) |
| `pane.ts` | `Pane` class: `cwd`, `entries` → `view` (filter + sort), `selected`, `load/go/up/render/repaintRows/paint/selectIndex`, virtualized rows (ADR-021), size bars, loading bar (`setBusy`) and remote notices (`showConnecting`, `showError` with Try again → `paneHooks.retry`). Calls `paneHooks.openFile` for files and `paneHooks.listed` after each listing |
| `panes.ts` | `local`, the remote views `remotes[]` (cloned from `#remote-tpl`), `remote` (live binding to the last-focused view), `active`, `addView/closeView`, the `.hsplit` handles, saved layout (`"views"` in localStorage), `setActive`, `switchTo`, `cyclePane` |
| `actions.ts` | `transfer`, `openItem`, `copyLink`, `deleteRemote`, `trashLocal`, `mkdir`, `calcSize`, `paneAction(pane, name)` |
| `sizes.ts` | Local folder sizes: `requestSizes`, `stopSizes`, the `"dir-sizes"` listener (rows, path-bar total, live re-list), and the storage bar (`drawDisk`) |
| `dnd.ts` | Mouse-event drag between panes (`armDrag` on mousedown, threshold 5 px, ghost label, drop-target highlight) and OS drops |
| `preview.ts` | Modal state `pv`, `openPreview`, `previewStep`, `closePreview` |
| `jobs.ts` | `"job"` listener, speed smoothing, the activity card (#activity) with the expandable transfer list, the transfer beam (#beam, only between panes that show the job's folders, gliding ends) and the transfer lanes (#lanes, one per running job whose folders the panes have left) |
| `settings.ts` | Settings dialog (Accounts / General / Appearance / About tabs), account picker and bucket picker, `applySettings()`, autostart toggle |
| `appearance.ts` | Theme, text size, pane splitter (per-device prefs) |
| `providers.ts` | Provider presets (AWS, R2, B2, Wasabi, DO, self-hosted) for the account form; `describeAccount()` |
| `icons.ts` | Stroke icon set (`icon()`, `hydrateIcons()` for `data-icon` placeholders) + the pane watermarks |
| `popover.ts` | Anchored dropdown menus (account, bucket, this computer): search box, ↑/↓/Enter/Esc |
| `palette.ts` | ⌘K command palette (commands, accounts, buckets, places, files) and path editing (⌘L, `editPath`) |
| `updater.ts` | Update check (startup + every 6 h, tray, Settings), banner, download progress, relaunch |
| `menu.ts` / `keyboard.ts` | Right-click and "⋯" menu / every shortcut |
| `types.ts`, `files.ts`, `util.ts`, `state.ts` | Shared types, file kinds and icons, DOM and format helpers, global `app` state (`cfg`, `bucket`, `showHidden`) |

Imports go one way to avoid cycles: `main → (actions, dnd, keyboard, menu, settings, jobs, preview) → panes → pane → (state, util, files, types)`.
`pane.ts` never imports actions or preview. It calls `paneHooks.openFile`, which `main.ts` assigns.

## Persistence

| What | Where |
|---|---|
| Accounts + General (`Settings`, JSON, mode 0600; secret keys only when no OS store is available) | `<app config dir>/config.json`: macOS `~/Library/Application Support/com.vnappmob.noexplorer/`, Windows `%APPDATA%\com.vnappmob.noexplorer\`, Linux `~/.config/com.vnappmob.noexplorer/` |
| Box refresh tokens | OS credential store, `account = box-refresh:<profile id>` (rotated on every refresh) |
| Secret keys | OS credential store: Keychain / Credential Manager / Secret Service, `service = <app identifier>`, `account = <profile id>` |
| UI preferences (`localStorage`) | `bucket@<profileId>`, `prefix@<profileId>`, `localPath`, `hidden`, `local.sortBy`, `local.sortDir`, `remote.sortBy`, `remote.sortDir`, `theme`, `textSize`, `split` |
| "Open" temp files | `$TMP/noexplorer/` (wiped at startup) |
| Private folders opened in the app (macOS) | `<app config dir>/granted.json`: lets automatic size scans enter them (ADR-022) |
| Folder-size index | Memory only (rebuilt on demand, kept fresh by watchers) |

## Security model

- Secrets (S3 secret keys, Nextcloud app passwords, WebDAV passwords) live in the OS keychain
  (ADR-020) and never reach the UI: commands return redacted settings, and Nextcloud sign-in
  stores the app password without passing it through the webview (ADR-026).
- The webview only loads local app assets, presigned S3 URLs and `nxr://` previews (remote
  files streamed by `proxy.rs` for WebDAV/Nextcloud, ADR-026, and local files, ADR-035).
  `csp` is `null` and `nxr://` serves any local path so local preview works anywhere. That is
  acceptable because no third-party content is rendered as HTML: PDFs are forced to
  `application/pdf`, and text goes in through `textContent`. Tightening this is on the
  [roadmap](ROADMAP.md).
- Object keys are untrusted. `safe_rel()` strips `..` and empty segments before a key becomes
  a local path, and every name or key goes through `esc()` before it is put into `innerHTML`.

## Build and release pipeline

`ci.yml`: frontend check/build · Rust fmt/clippy/tests, including the S3 round-trip against
SeaweedFS · compile check on macOS and Windows.
`release.yml` runs on tag `vX.Y.Z`: verifies the version → `tauri-action` builds a universal
macOS build (signed + notarized when secrets exist), plus Windows (msi/nsis) and Linux
(deb/AppImage/rpm) → a **draft** GitHub Release.
Website: Cloudflare Workers Builds deploys `/site` as Workers static assets (`wrangler.jsonc`, custom
domain `noexplorer.vnappmob.com`). The site reads the latest *published* release
from the GitHub API to build its download buttons. Details: [RELEASING.md](RELEASING.md).
