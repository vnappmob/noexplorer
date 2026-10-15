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
| `config.rs` | Connection settings | `Config`, `load_config`, `make_client` (path-style, checksum policy, endpoint normalization) |
| `state.rs` | Shared state | `AppState { cfg, client, cancels }` behind `Mutex`es. `set_config()` rebuilds the client |
| `jobs.rs` | Background jobs | `Job` (atomic counters, errors, cancel flag), `Job::spawn` (250 ms ticker + final event), `cancel` |
| `remote.rs` | S3 browsing | `Entry` row type, `list` (one level, delimiter `/`), `list_all` (recursive), `presign`, `read_text`, `mkdir`, `delete` |
| `transfer.rs` | Moving bytes | `upload`/`upload_file` (PUT or multipart), `download`/`download_file` (stream to `.part`), `open_object`, tuning constants |
| `local.rs` | Local disk (left pane) | `list_local` (+ crumbs and Places roots), `read_local_text`, `mkdir_local`, `open_local`, `reveal` |
| `util.rs` | Pure helpers | `err` (full SDK error chain), `safe_rel` (traversal guard), `unique_path`, `job_label`, `file_name`, `is_text_like` |

## IPC commands

Arguments are passed from JS as an object. Tauri converts camelCase keys to snake_case.
Every command returns `Result<T, String>`, and the error string is shown to the user as-is.

| Command | Args | Returns | Module | Notes |
|---|---|---|---|---|
| `get_config` | – | `Config \| null` | config | `null` on first run → the UI opens Settings |
| `test_config` | `cfg` | `string[]` buckets | config | Falls back to `HeadBucket(cfg.bucket)` if ListBuckets is denied |
| `save_config` | `cfg` | – | config | Writes `config.json` (0600) and rebuilds the client |
| `list_buckets` | – | `string[]` | config | `test_config` with the saved config |
| `list` | `bucket, prefix` | `Entry[]` | remote | One folder level. Folders have `dir: true` and a key ending in `/` |
| `presign` | `bucket, key, secs, inline` | URL | remote | `secs` is clamped to 60 s–7 d. `inline` forces Content-Type/Disposition for preview |
| `read_text` | `bucket, key, max` | string | remote | Range GET `0..max`, decoded as lossy UTF-8 |
| `mkdir` | `bucket, prefix, name` | – | remote | Writes an empty `prefix+name/` marker object |
| `delete` | `bucket, keys` | count | remote | A key ending in `/` deletes recursively. Bulk delete, then one-by-one as a fallback |
| `upload` | `bucket, prefix, paths` | job id | transfer | `paths` are absolute local files or folders. Folders keep their structure |
| `download` | `bucket, prefix, items[{key,size}], dest?` | job id | transfer | Writes to `dest` (the left pane's folder), else to the configured or OS Downloads folder |
| `open_object` | `bucket, key` | – | transfer | Downloads to `$TMP/noexplorer/<uuid>/` and opens with the default app |
| `cancel` | `id` | – | jobs | Sets the job's cancel flag. In-flight multipart uploads are aborted |
| `list_local` | `path?, hidden` | `LocalListing` | local | No `path` means the home folder. Includes `crumbs` and `roots` (Places) |
| `read_local_text` | `path, max` | string | local | |
| `mkdir_local` | `path, name` | – | local | Rejects `/`, `\`, `.`, `..` |
| `open_local` | `path` | – | local | Opens with the OS default app |
| `reveal` | `path` | – | local | Shows the item in Finder, Explorer or the Linux file manager |

## Events

**`"job"`** is emitted by `jobs.rs` every 250 ms while a job runs, plus once at the end.
The TS mirror is `JobEvent` in `src/types.ts`.

```jsonc
{
  "id": "uuid", "kind": "up" | "down", "label": "a.txt, b.txt +3 more",
  "bucket": "demo", "prefix": "backups/",          // remote side
  "files_done": 3, "files_total": 5, "bytes_done": 123, "bytes_total": 456,
  "status": "running" | "done" | "error" | "cancelled",
  "error": null | "2 errors. First: x.bin: …",     // per-file failures don't stop the job
  "dest": "/Users/me/Downloads/x.bin",             // download: what "Show" reveals
  "local_dir": "/Users/me/Downloads"               // download: refresh the left pane if it shows this folder
}
```

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
Local media: `convertFileSrc(path)` (asset protocol). Text, local or remote: first 512 KB as a
`<pre>` filled through `textContent`. Anything else shows "Open with default app"
(`open_object`/`open_local`).

**Settings**
Save runs `test_config` first, and only on success calls `save_config` → `loadBuckets()`. If
the endpoint or access key changed, the remembered bucket and prefix are reset.

## Frontend modules (`src/`)

| Module | Owns |
|---|---|
| `main.ts` | Boot sequence and per-pane DOM bindings (row selection, dblclick, crumbs, sort headers, footer buttons) |
| `pane.ts` | `Pane` class: `cwd`, `entries` → `view` (filter + sort), `selected`, `load/go/up/render/paint/selectIndex`. Calls `paneHooks.openFile` for files |
| `panes.ts` | `local`, `remote`, `active`, `setActive`, `switchTo` |
| `actions.ts` | `transfer`, `openItem`, `copyLink`, `deleteRemote`, `mkdir`, `paneAction(pane, name)` |
| `dnd.ts` | Mouse-event drag between panes (`armDrag` on mousedown, threshold 5 px, ghost label, drop-target highlight) and OS drops |
| `preview.ts` | Modal state `pv`, `openPreview`, `previewStep`, `closePreview` |
| `jobs.ts` | `"job"` listener, speed smoothing, Transfers panel, cancel and "Show" buttons |
| `settings.ts` | Settings dialog and bucket `<select>` |
| `menu.ts` / `keyboard.ts` | Context menu / every shortcut |
| `types.ts`, `files.ts`, `util.ts`, `state.ts` | Shared types, file kinds and icons, DOM and format helpers, global `app` state (`cfg`, `bucket`, `showHidden`) |

Imports go one way to avoid cycles: `main → (actions, dnd, keyboard, menu, settings, jobs, preview) → panes → pane → (state, util, files, types)`.
`pane.ts` never imports actions or preview. It calls `paneHooks.openFile`, which `main.ts` assigns.

## Persistence

| What | Where |
|---|---|
| Connection config (including the **secret key**, plain JSON, mode 0600) | `<app config dir>/config.json`: macOS `~/Library/Application Support/com.vnappmob.noexplorer/`, Windows `%APPDATA%\com.vnappmob.noexplorer\`, Linux `~/.config/com.vnappmob.noexplorer/` |
| UI preferences (`localStorage`) | `bucket`, `prefix`, `localPath`, `hidden`, `local.sortBy`, `local.sortDir`, `remote.sortBy`, `remote.sortDir` |
| "Open" temp files | `$TMP/noexplorer/` (wiped at startup) |

## Security model

- Credentials never leave the Rust side except inside the config file. The UI gets them back
  only to pre-fill the Settings form.
- The webview only loads local app assets, presigned S3 URLs and `asset://` local files.
  `csp` is `null` and the asset protocol scope is `**` so local preview works anywhere. That is
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
`website.yml`: deploys `/site` to Cloudflare Workers static assets (`wrangler.jsonc`, custom
domain `noexplorer.vnappmob.com`). The site reads the latest *published* release
from the GitHub API to build its download buttons. Details: [RELEASING.md](RELEASING.md).
