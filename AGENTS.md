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
| Version | `0.1.0`, not yet released |
| Last updated | 2026-10-15 |
| Works and is verified | Dual-pane browse; drag both ways; drop files from the OS; multipart upload (tested with 2 GB); folder up/download; quick look (image/video/audio/pdf/text, local and remote); presigned links; delete; mkdir; transfer progress/cancel; settings test/save |
| Tested on | macOS (Apple Silicon) against SeaweedFS. Windows and Linux are only checked for compilation in CI |
| Next up | First tagged release → see [docs/ROADMAP.md](docs/ROADMAP.md) |

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
  settings.ts        Connection dialog + bucket picker
  menu.ts            Right-click menu      keyboard.ts  All shortcuts
  types.ts           Types mirrored from Rust     util.ts / files.ts / state.ts  helpers and global state
index.html           All markup (panes, dialogs, preview)      src/styles.css  All styles (light and dark)
src-tauri/src/       Backend (Rust). Entry: lib.rs run() registers every command
  config.rs          Config (persisted JSON) + S3 client construction (path-style, checksums)
  remote.rs          S3 commands: list, presign, read_text, mkdir, delete
  transfer.rs        upload / download / open_object engine (multipart, concurrency)
  jobs.rs            Background Job + "job" progress events + cancel
  local.rs           Local filesystem commands for the left pane
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
- **The app never deletes or overwrites local files.** Downloads go to `name (n).ext` and stream through `.part` files ([ADR-006](docs/DECISIONS.md#adr-006)).
- **`safe_rel()` must wrap every object key that becomes a local path.** It is the path-traversal guard.
- **Drag between panes uses mouse events, not HTML5 drag-and-drop.** HTML5 DnD is broken on Windows while Tauri's file drop is enabled ([ADR-003](docs/DECISIONS.md#adr-003)).
- **Escape every untrusted string before it goes into `innerHTML`** (file names, keys, errors), using `esc()`. Text previews use `textContent`.
- **Keep the user's secrets out of logs and errors.** The secret key only lives in `config.json` (mode 600).
- **The UI is in English.** User-facing strings live in the TS files and `index.html`.

## Recipes

**Add a backend command**
1. Write `#[tauri::command] pub async fn foo(state: State<'_, AppState>, …) -> Result<T, String>` in the right module. Map SDK errors with `util::err`.
2. Register it in `lib.rs` `generate_handler![]`.
3. Add a row to the command table in `docs/ARCHITECTURE.md`. If it returns a new struct, mirror the type in `src/types.ts`.
4. Call it with `invoke<T>("foo", { camelOrSnakeArgs })`. Tauri maps JS camelCase keys to Rust snake_case args.

**Add a UI action** (button, menu item, shortcut)
- Implement the action in `actions.ts` and add a case to `paneAction()`.
- Button: add `<button data-act="name">` to the pane footer in `index.html`. Enable or disable it in `Pane.updateStatus()`.
- Menu item: add it in `menu.ts`. Shortcut: add it in `keyboard.ts`, then update the shortcut table in `README.md` and in `site/index.html`.

**Add a preview type:** add extensions in `files.ts` (`KIND`) and a render branch in `preview.ts`. Text types must also be added to `util::is_text_like` in Rust.

**Add a setting:** add the field to `Config` (Rust, `#[serde(default)]` keeps old configs loading), to `types.ts`, to the form in `index.html`, and to `settings.ts` (`readForm`/`openSettings`).

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

- **The `hidden` attribute loses to CSS `display: flex`.** `styles.css` has `[hidden]{display:none!important}`. Keep that rule.
- **`requestAnimationFrame` pauses when the window is hidden or covered.** Progress rendering uses `setTimeout` for that reason.
- **`tauri::generate_context!()` embeds `../dist`.** Run `npm run build` once before any bare `cargo build/test/clippy` on a fresh clone. CI does this.
- **The `minio/minio` Docker image is no longer published.** `scripts/dev-s3.sh` uses SeaweedFS, which speaks the same S3 API.
- **Right-click is captured by the app's context menu.** To open devtools in dev builds use **⌥⌘I** (macOS) or **Ctrl+Shift+I**.
- **`npm run tauri dev` restarts the app when any Rust file changes.** Frontend edits hot-reload.
- **Tauri drop positions are physical pixels.** Divide by `devicePixelRatio` before calling `elementFromPoint`.
- **The website deploys from Cloudflare, not GitHub Actions.** Workers Builds watches the repo. Build logs live in the Cloudflare dashboard (Workers & Pages → `noexplorer-site` → Deployments), not under Actions.
- **macOS signing secrets live in GitHub Actions.** Without them `release.yml` still builds, but unsigned ([docs/RELEASING.md](docs/RELEASING.md)).

## Conventions

- Formatting: Prettier (width 120) for TS, `rustfmt.toml` (width 120) for Rust. Run `npm run fmt`.
- Comments explain *why*, not *what*. Each module starts with a header comment that says what it owns.
- Commits use Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `ci:`, `release:`).
- No new runtime dependencies without a reason in the PR. Bundle size and simplicity are features.
