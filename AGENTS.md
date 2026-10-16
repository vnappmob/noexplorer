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
| Version | `0.4.1` released 2026-10-16 ([release](https://github.com/vnappmob/noexplorer/releases/tag/v0.4.1)): live folder sizes (ADR-022), Move to Trash (ADR-023), tray account switcher. 0.4.0: secrets in the OS keychain, virtualized lists. Releases bump the patch number by default |
| Last updated | 2026-10-16 |
| Works and is verified | Dual-pane browse; drag both ways; drop files from the OS; multipart upload (tested with 2 GB); folder up/download; quick look (image/video/audio/pdf/text, local and remote); presigned links; delete; mkdir; transfer progress/cancel; **multiple S3 accounts** (picker + manager, v1 config migration); **Σ folder size** (remote and local) and full-path bar; **theme + text size**; **resizable split**; **auto-update** (signed, GitHub `latest.json`; real OTA 0.2.0 → 0.2.1 confirmed on macOS 2026-10-15); **tray + background running** (hide on close, live transfer rows, notifications, launch at login: macOS Login Items shows "NoExplorer", confirmed 2026-10-15 after re-registration; single instance); **Settings → General / About**; **v0.3 UI** (two-row pane headers with account/bucket dropdowns, path bar editing, activity card, transfer beam, ⌘K palette, watermarks, light/dark), checked in the app against SeaweedFS; **live folder sizes** (own scanner, faster than ncdu, ADR-022) + live folder view; **Move to Trash** (macOS, ADR-023); **tray account switcher** (event path; native menu to be eyeballed) |
| Tested on | macOS (Apple Silicon) against SeaweedFS, the maintainer's MinIO, and real AWS S3 (read-only: ap-southeast-1, dotted bucket names, cross-region detection). Windows and Linux are only checked for compilation in CI |
| Next up | Eyeball the tray account menu on macOS/Windows/Linux and Move to Trash on Windows/Linux (CI compiles and unit-tests them). Then Tier 1 in docs/ROADMAP.md. Confirm on the maintainer's Mac that the OTA update to 0.4.0 moves the secret into Keychain without a prompt and the account still connects. Confirm on the maintainer's Mac that Login Items still shows "NoExplorer" (the plist was restored after a dev build overwrote it; BTM couldn't be read while the screen was locked). Smoke-test the new UI and tray on Windows/Linux. See [docs/ROADMAP.md](docs/ROADMAP.md) |

## Map

```
src/                 Frontend (TypeScript, no framework). Entry: main.ts
  main.ts            Wires DOM events to modules and boots the app
  pane.ts            Pane class: listing, sort, filter, selection, rendering (shared by both panes)
  panes.ts           The `local` + `remote` instances and the active pane
  actions.ts         transfer / open / link / delete / mkdir + paneAction() dispatcher
  dnd.ts             Drag between panes (mouse events) + drops from the OS (Tauri event)
  preview.ts         Quick-look modal
  jobs.ts            Transfers panel, listens to the "job" event
  settings.ts        Settings dialog (Accounts | Appearance), account + bucket pickers
  appearance.ts      Theme, text size, pane splitter (localStorage)
  providers.ts       Provider presets (AWS, R2, B2, Wasabi, DO…) for the account form
  icons.ts           Stroke icons + watermarks; `data-icon="name"` in markup, `icon()` in code
  popover.ts         Dropdown menus anchored to a chip/button (account, bucket, this computer)
  palette.ts         ⌘K command palette + path editing (⌘L)
  updater.ts         Auto-update: check, banner, install + relaunch
  sizes.ts           Local folder sizes: request, apply "dir-sizes", size bars, live re-list
  menu.ts            Right-click menu      keyboard.ts  All shortcuts
  types.ts           Types mirrored from Rust     util.ts / files.ts / state.ts  helpers and global state
index.html           All markup (panes, dialogs, preview)      src/styles.css  All styles (light and dark)
src-tauri/src/       Backend (Rust). Entry: lib.rs run() registers every command
  config.rs          Accounts: Profile/Settings (config.json, v1→v2 migration) + S3 client (path-style, checksums)
  secrets.rs         Secret keys in the OS credential store (Keychain / Credential Manager / Secret Service)
  remote.rs          S3 commands: list, presign, read_text, mkdir, delete
  transfer.rs        upload / download / open_object engine (multipart, concurrency)
  jobs.rs            Background Job + "job" progress events + cancel
  local.rs           Local filesystem commands for the left pane
  scan.rs            Parallel folder-size scanner (macOS getattrlistbulk fast path)
  sizes.rs           Live size index: background scans, watchers, "dir-sizes" events
  trash.rs           Move to Trash (macOS / Windows / freedesktop), no extra crates
  tray.rs            Tray/menu-bar icon: live transfer status, Show/Updates/Quit, hide-to-tray
  autostart.rs       Launch at login (LaunchAgent with bundle id, so macOS shows "NoExplorer")
  state.rs           AppState (config, client, cancel flags)    util.rs  pure helpers (tested)
src-tauri/tauri.conf.json   Window, bundle and security settings (version comes from package.json)
site/                Static website → Cloudflare Workers static assets (config: wrangler.jsonc)
assets/icon.svg      Icon source → icon.png (render 1024px), then `npx tauri icon assets/icon.png`
scripts/dev-s3.sh    Local S3 server in Docker for dev and tests
scripts/bump-version.sh   Sets the version in every place it lives
.github/workflows/   ci.yml · release.yml (signed builds → draft release) (website deploys via Cloudflare Workers Builds, not Actions)
docs/                ARCHITECTURE · DECISIONS (ADRs) · DEVELOPMENT · RELEASING · ROADMAP
```

## Commands

| Task | Command |
|---|---|
| Install | `npm install` |
| Run the app (dev, hot reload) | `npm run tauri dev` (Vite on port **14321**) |
| Local S3 for testing | `./scripts/dev-s3.sh up && ./scripts/dev-s3.sh seed` → endpoint `http://localhost:8333`, key `devkey` / `devsecret123` |
| Frontend typecheck + format check | `npm run check` |
| Format everything | `npm run fmt` |
| Rust lint | `cd src-tauri && cargo clippy --all-targets -- -D warnings` |
| Rust tests (unit) | `cd src-tauri && cargo test` |
| Rust tests (including the S3 round-trip) | `NOEXPLORER_TEST_ENDPOINT=http://localhost:8333 NOEXPLORER_TEST_KEY=devkey NOEXPLORER_TEST_SECRET=devsecret123 cargo test` |
| Release build | `npm run tauri build` |
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
- **Checksums are `WhenRequired`.** Don't remove this from `make_client`; it breaks MinIO and older servers ([ADR-004](docs/DECISIONS.md#adr-004)).
- **The app never permanently deletes or overwrites local files.** Downloads go to `name (n).ext` and stream through `.part` files ([ADR-006](docs/DECISIONS.md#adr-006)). The only way local files leave is **Move to Trash** (`trash.rs`), recoverable from the OS Trash ([ADR-023](docs/DECISIONS.md#adr-023)).
- **Automatic folder scans stay polite** ([ADR-022](docs/DECISIONS.md#adr-022)): low-priority threads, same volume as home, and never into macOS privacy-protected folders the user hasn't opened (`sizes.rs::protected`). Otherwise just opening Home makes macOS ask for Documents/Desktop/Downloads access.
- **Talk to a bucket through `state.client_for(&bucket)`, never `state.client()`.** On AWS each bucket may live in another region, and signing with the wrong region fails ([ADR-017](docs/DECISIONS.md#adr-017)).
- **`safe_rel()` must wrap every object key that becomes a local path.** It is the path-traversal guard.
- **Drag between panes uses mouse events, not HTML5 drag-and-drop.** HTML5 DnD is broken on Windows while Tauri's file drop is enabled ([ADR-003](docs/DECISIONS.md#adr-003)).
- **Escape every untrusted string before it goes into `innerHTML`** (file names, keys, errors), using `esc()`. Text previews use `textContent`.
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
- Menu item: add it in `menu.ts`. Shortcut: add it in `keyboard.ts`, then update the shortcut table in `README.md` and in `site/index.html`.

**Add a preview type:** add extensions in `files.ts` (`KIND`) and a render branch in `preview.ts`. Text types must also be added to `util::is_text_like` in Rust.

**Add a per-account setting:** add the field to `Profile` in `config.rs` (`#[serde(default)]` keeps old files loading), to `types.ts`, to `#acct-form` in `index.html`, and to `FIELDS`/`readForm` in `settings.ts`.
**Add a look/layout preference:** keep it per device in `appearance.ts` (localStorage), add a control to the Appearance tab, and drive it with a CSS variable or `data-*` attribute. Size new CSS from `--fs`/`--fs-sm`/`--fs-xs`, never fixed px font sizes.

**Tune transfer speed:** change the constants at the top of `transfer.rs` and record the reason in ADR-005.

## Keep docs in sync

| If you change… | …also update |
|---|---|
| A command (add, rename, change args) | `lib.rs` handler list, ARCHITECTURE command table, `types.ts` |
| A `"job"` event field | `jobs.rs` `JobEvent`, `types.ts` `JobEvent`, ARCHITECTURE event schema |
| A shortcut | `keyboard.ts`, README table, website shortcuts section |
| Something a user would notice | `CHANGELOG.md` → `## [Unreleased]` |
| An architectural choice or trade-off | Add an ADR to `docs/DECISIONS.md` |
| Known limitations or the plan | `docs/ROADMAP.md` and the **Status** block above |

## Gotchas (each of these has cost time before)

- **TypeScript 7's DOM types make `el.hidden` `boolean | "until-found"`.** Compare explicitly (`el.hidden !== false`) when you need a boolean.
- **The `hidden` attribute loses to CSS `display: flex`.** `styles.css` has `[hidden]{display:none!important}`. Keep that rule.
- **`requestAnimationFrame` pauses when the window is hidden or covered.** Progress rendering uses `setTimeout` for that reason.
- **`tauri::generate_context!()` embeds `../dist`.** Run `npm run build` once before any bare `cargo build/test/clippy` on a fresh clone. CI does this.
- **The `minio/minio` Docker image is no longer published.** `scripts/dev-s3.sh` uses SeaweedFS, which speaks the same S3 API.
- **Right-click is captured by the app's context menu.** To open devtools in dev builds use **⌥⌘I** (macOS) or **Ctrl+Shift+I**.
- **`npm run tauri dev` restarts the app when any Rust file changes.** Frontend edits hot-reload.
- **CI lints on Linux, but you probably develop on macOS.** Anything used only inside `#[cfg(target_os = "macos")]` (or another OS) becomes an unused import elsewhere and fails `-D warnings`. Use full paths (`tauri::RunEvent::Reopen`) inside cfg blocks instead of top-level `use`.
- **macOS keeps a login item's attribution from its first registration.** Rewriting the LaunchAgent plist doesn't change it; check with `sfltool dumpbtm` and re-register (remove, wait ~10 s, add) by bumping `ENTRY_VERSION` in `autostart.rs` ([ADR-018](docs/DECISIONS.md#adr-018)).
- **Debug builds register "Launch at login" as `NoExplorer Dev`** (`autostart.rs`), never as `NoExplorer`. Before that rule, a dev build's startup `refresh()` rewrote the installed app's login item to `target/debug`. Don't remove the debug-name split.
- **Run dev builds with `--config '{"identifier":"com.vnappmob.noexplorer.dev"}'` when NoExplorer is installed and running.** Otherwise single-instance hands every launch to the installed app, and both share one `config.json`.
- **`"dir-sizes"` only carries the local pane's current folder** (`folder_sizes` sets the focus). To show sizes elsewhere, call `folder_sizes` for that folder. On Linux only a root's top level is watched (inotify limits).
- **Benchmark the scanner:** `cd src-tauri && NOEXPLORER_BENCH_DIR=~/some/big/folder cargo test --release bench_scan -- --ignored --nocapture`. Compare with `du -sk -x` and `ncdu -x -t 8 -0 -o /dev/null` (warm cache; run twice).
- **File lists are virtualized** ([ADR-021](docs/DECISIONS.md#adr-021)). Only rows near the viewport exist as `.row` elements. Read rows from `pane.view` / `pane.selected`, scroll with `selectIndex`, and keep every row the same height.
- **Dev builds keep their secrets under the `com.vnappmob.noexplorer.dev` keychain service.** macOS may ask for keychain access after a rebuild (ad-hoc signature); release builds don't.
- **Tauri drop positions are physical pixels.** Divide by `devicePixelRatio` before calling `elementFromPoint`.
- **The website deploys from Cloudflare, not GitHub Actions.** Workers Builds watches the repo. Build logs live in the Cloudflare dashboard (Workers & Pages → `noexplorer-site` → Deployments), not under Actions.
- **The workflow token can't create releases in the vnappmob org** (403 "Resource not accessible by integration"). Create the draft with `gh release create vX.Y.Z --draft --verify-tag` right after pushing the tag; `tauri-action` uploads into it.
- **macOS signing secrets live in GitHub Actions.** Without them `release.yml` still builds, but unsigned ([docs/RELEASING.md](docs/RELEASING.md)).

## Conventions

- Formatting: Prettier (width 120) for TS, `rustfmt.toml` (width 120) for Rust. Run `npm run fmt`.
- Comments explain *why*, not *what*. Each module starts with a header comment that says what it owns.
- Commits use Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `ci:`, `release:`).
- No new runtime dependencies without a reason in the PR. Bundle size and simplicity are features.
