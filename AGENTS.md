# AGENTS.md — start here

> Handoff guide for AI agents and new maintainers. Read this file first. It should take
> about a minute and save you from reading the whole codebase. Go deeper only through the
> links below.

**NoExplorer** is a small cross-platform desktop app for moving files between your computer
and an S3-compatible bucket (self-hosted MinIO, SeaweedFS, AWS S3, …). It has two panes:
**local on the left, S3 on the right**. Dragging between the panes uploads or downloads.
It is built with **Tauri 2**: a Rust backend using `aws-sdk-s3`, and a framework-free
TypeScript UI. The UI is in English. License: MIT.

- Repo: `github.com/vnappmob/noexplorer` · Website: `https://noexplorer.vnappmob.com` (source in `/site`, hosted on Cloudflare Workers)
- Bundle id: `com.vnappmob.noexplorer` · Maintainer: Kris Luu (VNAppMob)

## Status (update this block when you finish a chunk of work)

| | |
|---|---|
| Version | `0.4.35` released 2026-10-23 ([release](https://github.com/vnappmob/noexplorer/releases/tag/v0.4.35)): only online-only cloud folders count as cloud (location, no markers); Cloud group in the Places menu (ADR-049 update). 0.4.34: folder move/copy within a bucket 16 objects at a time on S3 and Azure (ADR-005 update). 0.4.33: speed test redesigned (Run/History tabs, explicit bucket + folder per end, checked cleanup and sweep); GCS folder delete fixed (404 = gone). 0.4.32: experimental speed test between this computer and remotes or two remotes (ADR-050); Nextcloud MKCOL race fixed. 0.4.31: cloud-synced local folders recognized (☁) and left out of size scans; refused local folders explained (ADR-049). 0.4.30: remembered folder sizes shown at once (local across launches, remote Σ results); experimental capped remote folder sizes on S3/Azure (ADR-048). 0.4.29: experimental bucket access policies (presets + JSON, Copy public link, ADR-047). 0.4.28: WebDAV rejects web pages (needs 207), refused uploads not retried, OneDrive 3 files at a time + throttling logged (ADR-005 update). 0.4.27: OneDrive standard, VNAppMob a verified Microsoft publisher, 50 MiB upload chunks and 4 parallel ranged downloads (ADR-046). 0.4.26: simple account setup (ADR-044); experimental OneDrive (ADR-045). 0.4.25: new buckets, up a level as left and folder tree standard; Oracle Cloud preset (ADR-043). 0.4.24: search below a folder (ADR-042), provider dropdown, folder tree fixes, window reopens after an update relaunch. 0.4.23: Back = up as left, experimental folder tree on ↑ (ADR-041). 0.4.22: redesigned Settings (ADR-040), experimental Back, SFTP + GCS standard, Box experimental. 0.4.21: SFTP password / key-passphrase login via askpass + key file (ADR-039); experimental Create buckets. 0.4.20: Azure Blob standard with automatic endpoint; macOS delete key deletes (⌘↑ goes up); GCS Project ID field (x-amz-project-id; GCS confirmed working with the maintainer's real key on 2026-10-19); home page benchmarks. 0.4.19: server error details in messages, redacted diagnostic log (Settings → About → Show log file). GCS still being diagnosed with the maintainer's real key. 0.4.18: GCS fix (the SDK's x-id query parameter is dropped for storage.googleapis.com; not yet confirmed against real GCS). 0.4.17: rename/move/duplicate, resumable transfers, grid and favorites standard; tabs and history removed (ADR-038); README screenshot, livelier home page, relay-copy test. 0.4.16: copies between accounts keep the Content-Type. 0.4.15: grid selection without the bold border. 0.4.14: cache folder kept under ~70 MB (previews no-store, WebKit cache trimmed at launch, thumbnails ≤ 48 MB, ADR-037). 0.4.13: fix for 0.4.12's grid-scrolling crash (custom-protocol answers on the main thread, local media through nxr://, ADR-035), cached grid thumbnails (ADR-036). 0.4.12: experimental native window look removed (opaque window everywhere), bigger and lighter grid icons. 0.4.11: hotfix, the experimental Vietnamese UI removed (it froze the window), Experimental tab one feature per row. 0.4.10: Settings → Experimental with rename/move/duplicate, resumable transfers, self-signed HTTPS, backup pairs, grid, tabs/history/favorites, GCS/Azure/SFTP, native look (all off by default, ADR-031..033); accessibility; Astro/Starlight website (ADR-034). 0.4.9: beam starts at the sent files, lanes as two boxes with full paths, remote delete as a background job. 0.4.8: transfer lanes, beam only between panes showing the job, no system notifications. 0.4.7: account-switch feedback (connecting notice, loading bar, inline errors with Try again), wrong-bucket race fix, Nextcloud session cookies (~8× faster listings). 0.4.6: Enter in the New folder dialog no longer cancels. 0.4.5: hotfix for 0.4.4's hung transfers (Job::event deadlock) and the short lone remote view. 0.4.4: split remote views + copy between accounts (ADR-029), redesigned website. Earlier in 0.4.x: Nextcloud/WebDAV (ADR-026), Box beta (ADR-027), Compare with checksums (ADR-028), provider interface (ADR-025), lean dev builds (ADR-024), disk storage bar. Releases bump the patch number by default |
| Last updated | 2026-10-23 |
| Works and is verified | Dual-pane browse; drag both ways; drop files from the OS; multipart upload (tested with 2 GB); folder up/download; quick look (image/video/audio/pdf/text, local and remote); presigned links; delete; mkdir; transfer progress/cancel; **multiple S3 accounts** (picker + manager, v1 config migration); **Σ folder size** (remote and local) and full-path bar; **theme + text size**; **resizable split**; **auto-update** (signed, GitHub `latest.json`; real OTA 0.2.0 → 0.2.1 confirmed on macOS 2026-10-15); **tray + background running** (hide on close, live transfer rows, launch at login: macOS Login Items shows "NoExplorer", confirmed 2026-10-15 after re-registration; single instance); **Settings → General / About**; **v0.3 UI** (two-row pane headers with account/bucket dropdowns, path bar editing, activity card, transfer beam, ⌘K palette, watermarks, light/dark), checked in the app against SeaweedFS; **live folder sizes** (own scanner, faster than ncdu, ADR-022) + live folder view; **Move to Trash** (macOS, ADR-023); **tray account switcher** (event path; native menu to be eyeballed); **split remote views** (1–3 accounts, drag S3 → Nextcloud, reload restores layout; checked in the dev app); **Compare** with checksums; **Nextcloud/WebDAV** (dev container) |
| Tested on | macOS (Apple Silicon) against SeaweedFS, the maintainer's MinIO, and real AWS S3 (read-only: ap-southeast-1, dotted bucket names, cross-region detection). Windows and Linux are only checked for compilation in CI |
| Next up | Rename/move/duplicate, resumable transfers, grid and favorites graduated to standard features on 2026-10-19 (ADR-038; tabs and history removed). Standard since 2026-10-19: Azure, GCS and SFTP (confirmed by the maintainer on real servers). Standard since 2026-10-19 too: create buckets, Back (up as left), folder tree. Still experimental: backup pairs, self-signed HTTPS, Box (no real Box run yet), bucket access policies (ADR-047, 0.4.29; checked against SeaweedFS, not yet on AWS/MinIO). Remote folder sizes (ADR-048, 0.4.30): automatic, capped, S3/Azure; remembered local/remote sizes standard. Cloud-synced local folders skipped by size scans (ADR-049, 0.4.31). Speed test (ADR-050, 0.4.32; dev S3 + Nextcloud tested). OneDrive (ADR-045) confirmed by the maintainer and standard since 2026-10-20 (ADR-046, 0.4.27); VNAppMob is a verified Microsoft publisher; NoExplorer's Entra app 87fa48bc-…. OneDrive speed after ADR-046 not yet measured on a real account. Oracle Cloud preset (S3 Compatibility API) confirmed working by the maintainer on 2026-10-20. Simple account setup (ADR-044, 0.4.26): endpoints built from an account ID / namespace / region for R2, B2, Wasabi, DO, Oracle, AWS, GCS. Settings redesigned 2026-10-19 (ADR-040); graduate them after real-world use. Not yet seen in the real app window (the maintainer's screen was locked): pause button and resume banner. First real runs on the maintainer's setup: Box with their own Box app (beta), Nextcloud browser sign-in, split views with real accounts. Windows/Linux eyeballing. No donation buttons; Windows stays unsigned (ADR-030) |

## Map

```
src/                 Frontend (TypeScript, no framework). Entry: main.ts
  main.ts            Wires DOM events to modules and boots the app
  pane.ts            Pane class: listing, sort, filter, selection, rendering (shared by both panes)
  panes.ts           `local`, the 1–3 remote views (`remotes`, split top/bottom), `remote` = last-focused view, the active pane
  actions.ts         transfer / open / link / delete / mkdir + paneAction() dispatcher
  dnd.ts             Drag between panes (mouse events) + drops from the OS (Tauri event)
  preview.ts         Quick-look modal
  jobs.ts            "job" events → activity card, transfer beam, transfer lanes (above the panes)
  settings.ts        Settings dialog (sidebar: Accounts, General, Appearance, Experimental, About), provider picker, account form, account + bucket pickers
  appearance.ts      Theme, text size, pane splitter (localStorage)
  providers.ts       Provider presets (AWS, R2, B2, Wasabi, DO…), their badges (`LOOK`) and picker groups
  icons.ts           Stroke icons + watermarks; `data-icon="name"` in markup, `icon()` in code
  popover.ts         Dropdown menus anchored to a chip/button (account, bucket, this computer)
  palette.ts         ⌘K command palette + path editing (⌘L)
  updater.ts         Auto-update: check, banner, install + relaunch
  sizes.ts           Local folder sizes: request, apply "dir-sizes" (stale = remembered), live re-list, storage bar
  remotesizes.ts     Remote folder sizes: remembered Σ results (localStorage), experimental capped auto counting
  compare.ts         Compare dialog: run, tabs, verdict that explains count gaps, upload/download the diff
  menu.ts            Right-click menu      keyboard.ts  All shortcuts
  experimental.ts    Settings → Experimental (EXPERIMENTS list); features check `exp("key")` from state.ts
  fileops.ts         Rename / move / duplicate / undo
  thumbs.ts          Grid view thumbnails: backend thumbnails, loaded elements reused across redraws, next screen prefetched
  favorites.ts       Favorites: the path-bar star, places/account menus and ⌘K (per device, localStorage)
  history.ts         Back (←, ⌘[, ⌘↑): up a level, restoring the parent's remembered scroll + selection; ⌘[ / ← / mouse back
  search.ts          Search below the current folder (↵ in the filter box): results in the pane, progress bar, Show in its folder
  tree.ts            Folder tree: ↑ opens a tree built from the folders listed so far (folders only, in memory)
  speedtest.ts       Speed test dialog (experimental): From/To places, presets, results history
  policy.ts          Bucket access policies (experimental): presets ↔ policy JSON dialog, Copy public link
  backups.ts         Backup pairs (experimental): saved local ↔ remote folders, run = compare + upload_rel
  types.ts           Types mirrored from Rust     util.ts / files.ts / state.ts  helpers and global state
index.html           All markup (panes, dialogs, preview)      src/styles.css  All styles (light and dark)
src-tauri/src/       Backend (Rust). Entry: lib.rs run() registers every command
  config.rs          Accounts: Profile (kind = provider)/Settings (config.json, v1→v2 migration), endpoint/region helpers
  provider.rs        `enum Remote`: one interface for all providers (ADR-025); Entry/BucketInfo
  s3.rs              S3 provider: client (path-style, checksums, per-bucket region), list, multipart, downloads
  webdav.rs          Nextcloud/WebDAV provider: own small client, chunked upload v2, shares, Login Flow v2
  proxy.rs           nxr:// scheme: local files, remote previews without presigned URLs (Range, CORS), grid thumbnails; answers on the main thread
  thumbs.rs          Grid thumbnails: 256 px JPEG/PNG, LRU caches in memory (16 MB) and on disk (48 MB, OS cache folder); trims WebKit's cache at launch
  azure.rs           Azure Blob provider: Shared Key signing, block uploads, SAS links
  sftp.rs            SFTP provider: the system `ssh` carries russh-sftp (your keys/agent/config)
  askpass.rs         Passwords / key passphrases for that `ssh`: the app binary is its SSH_ASKPASS, fetching the secret over loopback with a one-time token
  onedrive.rs        OneDrive provider: Microsoft Graph by path, our public Entra app (PKCE), upload sessions, async copies
  box_api.rs         Box provider (beta): BYO OAuth app, loopback sign-in, rotating refresh token, sessions
  compare.rs         Compare local vs remote folders: missing/changed/junk, checksum verify (ETag/SHA-1)
  secrets.rs         Secret keys in the OS credential store (Keychain / Credential Manager / Secret Service)
  remote.rs          Browsing commands (thin wrappers over Remote): buckets, list, presign, read_text, mkdir, delete
  transfer.rs        upload / download / open_object jobs (file concurrency; per-file work is the provider's)
  jobs.rs            Background Job + "job" progress events + cancel
  resume.rs          Resumable transfers: speed limit (`throttle`), `retry`, journals (pending.json, multipart.json), pause
  tls.rs             Experimental self-signed HTTPS: certificate pinning verifier; reqwest as the S3 SDK's HTTP client for pinned accounts
  fileops.rs         Rename / move / duplicate: local (never overwrite, same disk) and remote jobs (`Remote::transfer_within`)
  local.rs           Local filesystem commands for the left pane (+ volume_info for the storage bar)
  scan.rs            Parallel folder-size scanner (macOS getattrlistbulk fast path)
  speedtest.rs       Speed test (experimental): real provider transfers on random data, latency, cleanup
  cloud.rs           Online-only cloud folders by location (CloudStorage, iCloud, OneDrive, Google Drive volume): never scanned, ☁, Places Cloud group
  search.rs          Name search: query matcher (words, globs, ext:), parallel local walker streaming "search" events, remote via list_files
  sizes.rs           Live size index: background scans, watchers, "dir-sizes" events, remembered totals (folder-sizes.json)
  trash.rs           Move to Trash (macOS / Windows / freedesktop), no extra crates
  diag.rs            Diagnostic log: failed server requests (redacted) in the OS log folder; `log_file` command
  tray.rs            Tray/menu-bar icon: live transfer status, Show/Updates/Quit, hide-to-tray
  autostart.rs       Launch at login (LaunchAgent with bundle id, so macOS shows "NoExplorer")
  state.rs           AppState (settings, one Remote per account, cancel flags)    util.rs  pure helpers (tested)
src-tauri/tauri.conf.json   Window, bundle and security settings (version comes from package.json)
site/                Website (Astro): home page src/pages/index.astro + docs src/content/docs/docs/*.md (Starlight) → site/dist
assets/icon.svg      Icon source → icon.png (render 1024px), then `npx tauri icon assets/icon.png`
scripts/dev-s3.sh    Local S3 server in Docker for dev and tests
scripts/dev-nextcloud.sh  Local Nextcloud in Docker (user dev) for the WebDAV provider
scripts/bump-version.sh   Sets the version in every place it lives
scripts/clean-build.sh    Keeps src-tauri/target ≤ 12 GB and ≥ 60 GB free, tiered cleaning (CLAUDE.md storage policy)
.github/workflows/   ci.yml · release.yml (signed builds → draft release) (website deploys via Cloudflare Workers Builds, not Actions)
docs/                ARCHITECTURE · DECISIONS (ADRs) · DEVELOPMENT · RELEASING · ROADMAP
```

## Commands

| Task | Command |
|---|---|
| Install | `npm install` |
| Run the app (dev, hot reload) | `npm run tauri dev` (Vite on port **14321**) |
| Local S3 for testing | `./scripts/dev-s3.sh up && ./scripts/dev-s3.sh seed` → endpoint `http://localhost:8333`, key `devkey` / `devsecret123` |
| Local Nextcloud for testing | `./scripts/dev-nextcloud.sh up && ./scripts/dev-nextcloud.sh seed` → `http://localhost:8080`, user `dev` / `NoExplorer-dev-2026!` |
| Nextcloud round-trip test | `NOEXPLORER_TEST_NEXTCLOUD=http://localhost:8080 NOEXPLORER_TEST_DAV_USER=dev NOEXPLORER_TEST_DAV_PASS='NoExplorer-dev-2026!' cargo test nextcloud_roundtrip` |
| Frontend typecheck + format check | `npm run check` |
| Format everything | `npm run fmt` |
| Rust lint | `cd src-tauri && cargo clippy --all-targets -- -D warnings` |
| Rust tests (unit) | `cd src-tauri && cargo test` |
| Rust tests (including the S3 round-trip) | `NOEXPLORER_TEST_ENDPOINT=http://localhost:8333 NOEXPLORER_TEST_KEY=devkey NOEXPLORER_TEST_SECRET=devsecret123 cargo test` |
| Release build | `npm run tauri build` |
| Keep build output within limits | `npm run clean:build` (tiered; deps are kept). Agents: see the storage policy in `CLAUDE.md` |
| Preview / deploy the website | `npm run site` (local, wrangler dev) · Cloudflare deploys automatically on push to `main` |
| Cut a release | `./scripts/bump-version.sh X.Y.Z`, then follow [docs/RELEASING.md](docs/RELEASING.md) |

Before you say a change is done, CI must pass locally: `npm run check`, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

## How it works (30 seconds)

1. The UI calls `invoke("command", args)`. Each command is a `#[tauri::command]` in
   `src-tauri/src/*.rs` and is listed in `lib.rs`.
2. Listing, preview and settings commands return their result directly.
3. `upload` and `download` return a **job id** right away and run in the background. A
   `Job` emits a `"job"` event every 250 ms plus a final one. `jobs.ts` renders those events
   and refreshes the pane that shows the destination folder.
4. The two panes are the same `Pane` class. `key` is the S3 key on the remote pane and the
   absolute path on the local pane. `transfer(fromPane, items, targetDir?)` in `actions.ts`
   is the one place that decides between upload and download.

Full detail, including the command table, event schema and data flows, is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Invariants: do not break these

- **Path-style addressing defaults to ON.** Self-hosted S3 needs it ([ADR-007](docs/DECISIONS.md#adr-007)).
- **Checksums are `WhenRequired`.** Don't remove this from `s3::make_client_in`; it breaks MinIO and older servers ([ADR-004](docs/DECISIONS.md#adr-004)).
- **The app never permanently deletes or overwrites local files.** Downloads go to `name (n).ext` and stream through `.part` files ([ADR-006](docs/DECISIONS.md#adr-006)). The only way local files leave is **Move to Trash** (`trash.rs`), recoverable from the OS Trash ([ADR-023](docs/DECISIONS.md#adr-023)).
- **Automatic folder scans stay polite** ([ADR-022](docs/DECISIONS.md#adr-022)): low-priority threads, same volume as home, and never into macOS privacy-protected folders the user hasn't opened (`sizes.rs::protected`). Otherwise just opening Home makes macOS ask for Documents/Desktop/Downloads access.
- **Reach remote storage only through `state.remote(account)?` → `Remote` methods** ([ADR-025](docs/DECISIONS.md#adr-025)). Never build an SDK client in a command: `Remote` carries per-account caches (on AWS, each bucket's region, [ADR-017](docs/DECISIONS.md#adr-017)). New providers are a new `Remote` variant.
- **`safe_rel()` must wrap every object key that becomes a local path.** It is the path-traversal guard.
- **Drag between panes uses mouse events, not HTML5 drag-and-drop.** HTML5 DnD is broken on Windows while Tauri's file drop is enabled ([ADR-003](docs/DECISIONS.md#adr-003)).
- **Escape every untrusted string before it goes into `innerHTML`** (file names, keys, errors), using `esc()`. Text previews use `textContent`.
- **TLS is never switched off.** Self-signed servers are handled by pinning one certificate per account (`tls.rs`); don't add an "accept invalid certificates" option.
- **Secrets never go into a child's environment, a file or a command line.** `ssh` gets SFTP passwords through `askpass.rs` (the app binary answers `SSH_ASKPASS` over 127.0.0.1 with a one-time token); `run()` checks for askpass mode before anything else.
- **The diagnostic log (`diag.rs`) gets server errors only through `diag::log`, which redacts signatures, key ids and tokens.** Never log a request's headers or a secret key; add new patterns to `redact` (and its test) when a provider echoes something sensitive.
- **Secrets stay in the backend.** Secret keys live in the OS credential store (`secrets.rs`, [ADR-020](docs/DECISIONS.md#adr-020)); `config.json` (mode 600) holds them only when no store exists. Every command that returns `Settings` must return `config::redacted()`, and none goes into logs or errors.
- **Never lose or rotate the updater signing key** (`~/.tauri/noexplorer-updater.key` on the maintainer's Mac, plus the repo secret `TAURI_SIGNING_PRIVATE_KEY`). Installed apps only accept updates signed by it; a new key means every user has to reinstall by hand ([ADR-015](docs/DECISIONS.md#adr-015)).
- **Closing the window hides to the tray; it doesn't quit** (when `general.run_in_background` is on). Code that must run "on exit" has to handle `app.exit()` from the tray, not window close.
- **The UI is in English.** User-facing strings live in the TS files and `index.html`.

## Recipes

**Add a backend command**
1. Write `#[tauri::command] pub async fn foo(state: State<'_, AppState>, …) -> Result<T, String>` in the right module. Map SDK errors with `util::err`.
2. Register it in `lib.rs` `generate_handler![]`.
3. Add a row to the command table in `docs/ARCHITECTURE.md`. If it returns a new struct, mirror the type in `src/types.ts`.
4. Call it with `invoke<T>("foo", { camelOrSnakeArgs })`. Tauri maps JS camelCase keys to Rust snake_case args.

**Add a UI action** (button, menu item, shortcut)
- Implement the action in `actions.ts` and add a case to `paneAction()`.
- There is no per-selection toolbar (removed in 0.3.1, ADR-019): selection actions belong in the right-click menu, a shortcut and the ⌘K palette (`palette.ts` `build()`). Pane-level tools (filter, refresh, …) go in the `.src` header row in `index.html`.
- Menu item: add it in `menu.ts`. Shortcut: add it in `keyboard.ts`, then update the shortcut table in `README.md` and in `site/src/content/docs/docs/shortcuts.md`.

**Add a preview type:** add extensions in `files.ts` (`KIND`) and a render branch in `preview.ts`. Text types must also be added to `util::is_text_like` in Rust.

**Add a per-account setting:** add the field to `Profile` in `config.rs` (`#[serde(default)]` keeps old files loading), to `types.ts`, to `#acct-form` in `index.html`, and to `FIELDS`/`readForm` in `settings.ts`.
**Add a look/layout preference:** keep it per device in `appearance.ts` (localStorage), add a control to the Appearance tab, and drive it with a CSS variable or `data-*` attribute. Size new CSS from `--fs`/`--fs-sm`/`--fs-xs`, never fixed px font sizes.

**Tune transfer speed:** files in flight: `FILE_CONCURRENCY` in `transfer.rs`; S3 part size/concurrency: the constants at the top of `s3.rs`. Record the reason in ADR-005.

**Add a provider:** add a `ProviderKind` variant (`config.rs`, `types.ts`), a module like `s3.rs` with the same methods, and a `Remote` variant whose `match` arms call it (`provider.rs`). Then the account form fields.

**Add an experimental feature:** add a field to `Experimental` in `config.rs` and `types.ts`, an
entry to `EXPERIMENTS` in `experimental.ts`, and gate the feature with `exp("key")`. Listen for the
`"experimental"` window event if the UI must react when it's flipped.
To graduate one, remove its field, its `EXPERIMENTS` entry and its `exp()` gates, and move its docs
from `experimental.md` into the regular pages (as ADR-038 did).

## Keep docs in sync

| If you change… | …also update |
|---|---|
| A command (add, rename, change args) | `lib.rs` handler list, ARCHITECTURE command table, `types.ts` |
| A `"job"` event field | `jobs.rs` `JobEvent`, `types.ts` `JobEvent`, ARCHITECTURE event schema |
| A shortcut | `keyboard.ts`, README table, `site/src/content/docs/docs/shortcuts.md` |
| A user-visible behavior or setting | The matching docs page in `site/src/content/docs/docs/` |
| Something a user would notice | `CHANGELOG.md` → `## [Unreleased]` |
| An architectural choice or trade-off | Add an ADR to `docs/DECISIONS.md` |
| Known limitations or the plan | `docs/ROADMAP.md` and the **Status** block above |

## Gotchas (each of these has cost time before)

- **TypeScript 7's DOM types make `el.hidden` `boolean | "until-found"`.** Compare explicitly (`el.hidden !== false`) when you need a boolean.
- **The `hidden` attribute loses to CSS `display: flex`.** `styles.css` has `[hidden]{display:none!important}`. Keep that rule.
- **`requestAnimationFrame` pauses when the window is hidden or covered.** Progress rendering uses `setTimeout` for that reason.
- **Dev builds are lean on purpose** ([ADR-024](docs/DECISIONS.md#adr-024)): `crate-type = ["rlib"]` (desktop only) and no debug info for dependencies. Don't restore the template's `staticlib`/`cdylib` without a mobile target, and don't add `split-debuginfo = "unpacked"` (Windows MSVC rejects it).
- **`tauri::generate_context!()` embeds `../dist`.** Run `npm run build` once before any bare `cargo build/test/clippy` on a fresh clone. CI does this.
- **The `minio/minio` Docker image is no longer published.** `scripts/dev-s3.sh` uses SeaweedFS, which speaks the same S3 API.
- **Right-click is captured by the app's context menu.** To open devtools in dev builds use **⌥⌘I** (macOS) or **Ctrl+Shift+I**.
- **`npm run tauri dev` restarts the app when any Rust file changes.** Frontend edits hot-reload.
- **CI lints on Linux, but you probably develop on macOS.** Anything used only inside `#[cfg(target_os = "macos")]` (or another OS) becomes an unused import elsewhere and fails `-D warnings`. Use full paths (`tauri::RunEvent::Reopen`) inside cfg blocks instead of top-level `use`.
- **macOS keeps a login item's attribution from its first registration.** Rewriting the LaunchAgent plist doesn't change it; check with `sfltool dumpbtm` and re-register (remove, wait ~10 s, add) by bumping `ENTRY_VERSION` in `autostart.rs` ([ADR-018](docs/DECISIONS.md#adr-018)).
- **An update relaunch reuses the process's arguments**, so a login start (`--hidden`) came back hidden after an update. The updater calls `show_after_relaunch` first, and startup also treats a changed version (`last-version` in the config folder) as a relaunch (`tray::take_relaunch_marker`).
- **Debug builds register "Launch at login" as `NoExplorer Dev`** (`autostart.rs`), never as `NoExplorer`. Before that rule, a dev build's startup `refresh()` rewrote the installed app's login item to `target/debug`. Don't remove the debug-name split.
- **Run dev builds with `--config '{"identifier":"com.vnappmob.noexplorer.dev"}'` when NoExplorer is installed and running.** Otherwise single-instance hands every launch to the installed app, and both share one `config.json`.
- **`"dir-sizes"` only carries the local pane's current folder** (`folder_sizes` sets the focus). To show sizes elsewhere, call `folder_sizes` for that folder. On Linux only a root's top level is watched (inotify limits).
- **Benchmark the scanner:** `cd src-tauri && NOEXPLORER_BENCH_DIR=~/some/big/folder cargo test --release bench_scan -- --ignored --nocapture`. Compare with `du -sk -x` and `ncdu -x -t 8 -0 -o /dev/null` (warm cache; run twice).
- **`reqwest` 0.13 hides `.form()` behind a feature**: `webdav.rs` builds form bodies itself (`FormExt::form_body`) so the shared reqwest build stays unchanged. Install the rustls ring provider before building a client (`webdav::http()` does).
- **Nextcloud: keep sending the session cookie** (`Dav::send` / `keep_cookies` in `webdav.rs`). Basic auth alone makes Nextcloud verify the app password on every request (~100 ms each); with the cookie a PROPFIND drops from ~115 ms to ~13 ms. Requests still carry basic auth, which is what lets writes pass Nextcloud's CSRF check. Route every new Dav request through `self.send(self.req(…))`.
- **Moving bytes goes through `Job::flow(n)`** (count + speed limit + pause) or `Job::gate(n)` before a whole request (then count). A new provider loop must use them, or pause and the speed limit won't apply to it. `.part` handling for downloads is shared: `provider::{part_path, resume_offset, open_part, drop_part}`.
- **No system notifications** (removed 2026-10-17 at the maintainer's request: each platform behaves differently and macOS asks for permission). Don't add them back; progress lives in the activity card, lanes, beam and tray.
- **The beam and lanes read panes at render time** (`jobs.ts`): `viewFor()` only returns a view that shows the job's account + bucket and isn't connecting/failed. `placement()` is the one rule: beam when the source pane is in the source folder and the destination pane is in the destination folder or above it, else a lane (never both). The beam starts at the job's own items (`noteJobItems`, from `transfer()`), never at the current selection. `Pane.render` fires a bubbling `panechange` event that re-aims them. Lanes are built once per job and updated in place, so clicks aren't lost to re-renders.
- **Remote views report their state in the list** (`Pane.notice`: connecting / error with Try again), and `Pane.setBusy` shows the loading bar after 150 ms. Don't `fail()` (toast) listing errors of a remote view; `load()` already shows them. `Pane.token` drops answers for an account the view has left.
- **`nxr://` responses must not set `Content-Length`**: the webview adds it, and a duplicate makes macOS video stay at `readyState 0`. Keep the CORS headers too: `fetch()` from the app's origin preflights.
- **Box rotates its refresh token on every refresh**: always persist the new one (`BoxCom::access` → `save_refresh`), or the account is signed out on the next launch. The redirect URI is fixed at `http://127.0.0.1:53682/callback` and must match the user's Box app.
- **Box has no test account here**: `box_roundtrip_against_mock` (in `box_api.rs`) is the safety net. Keep the mock in step with any API change, and say "beta" until a real run is confirmed.
- **Compare must never call a file "verified" without reproducing the server's hash.** Unknown multipart part sizes and KMS ETags count as "same size".
- **`quick-xml` 0.42 gives names as `&str`, and entities arrive as separate `GeneralRef` events** (`parse_multistatus` handles both).
- **The grid view is the same virtualized list** (`Pane.drawGrid`): tiles are `.row.cell` with `data-i`, so selection, drag & drop and menus work unchanged. Thumbnails never use a canvas (remote links usually have no CORS).
- **Answer custom-protocol requests on the main thread** (`proxy::respond`, [ADR-035](docs/DECISIONS.md#adr-035)). From a worker, a request WebKit stopped in between makes it throw an Objective-C exception, and release builds (`panic = "abort"`) die. Tauri's asset protocol does that, so it's off: local media goes through `nxr://` (`localSrc`).
- **File lists are virtualized** ([ADR-021](docs/DECISIONS.md#adr-021)). Only rows near the viewport exist as `.row` elements. Read rows from `pane.view` / `pane.selected`, scroll with `selectIndex`, and keep every row the same height.
- **There is no global account or bucket any more** ([ADR-029](docs/DECISIONS.md#adr-029)). Each remote view has its own; pass `view.where()` to every remote command. `remote` from `panes.ts` is a *live* binding to the last-focused view: read it at call time, never cache it in a module-level const.
- **Never take the same `Mutex` twice in one expression or struct literal.** The first guard lives until the end of the statement, so the second `lock()` deadlocks (std mutexes aren't reentrant). This shipped in 0.4.4 inside `Job::event` and hung every transfer; `jobs::tests` guards it.
- **Dev builds keep their secrets under the `com.vnappmob.noexplorer.dev` keychain service.** macOS may ask for keychain access after a rebuild (ad-hoc signature); release builds don't.
- **Tauri drop positions are physical pixels.** Divide by `devicePixelRatio` before calling `elementFromPoint`.
- **The website deploys from Cloudflare, not GitHub Actions.** Workers Builds watches the repo. Build logs live in the Cloudflare dashboard (Workers & Pages → `noexplorer-site` → Deployments), not under Actions.
- **"A timestamp was expected but was not found" in the macOS release job** is Apple's timestamp server failing, not our code. Re-run only that job (`gh run rerun <id> --failed`); it uploads into the same draft and tauri-action merges its entries into `latest.json`.
- **The home page stays short on purpose** (maintainer, 2026-10-17): one message (local ↔ remote transfer), the mockup, download, three points, measured benchmarks (only numbers we measured: ADR-022 scanner vs du/ncdu, Nextcloud session reuse). Feature detail goes into the docs pages, not `site/src/pages/index.astro`. Emoji and the app mockup keep it lively without more text.
- **The workflow token can't create releases in the vnappmob org** (403 "Resource not accessible by integration"). Create the draft with `gh release create vX.Y.Z --draft --verify-tag` right after pushing the tag; `tauri-action` uploads into it.
- **macOS signing secrets live in GitHub Actions.** Without them `release.yml` still builds, but unsigned ([docs/RELEASING.md](docs/RELEASING.md)).

## Conventions

- Formatting: Prettier (width 120) for TS, `rustfmt.toml` (width 120) for Rust. Run `npm run fmt`.
- Comments explain *why*, not *what*. Each module starts with a header comment that says what it owns.
- Commits use Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `ci:`, `release:`).
- No new runtime dependencies without a reason in the PR. Bundle size and simplicity are features.
