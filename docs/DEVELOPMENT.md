# Development

## Prerequisites

- **Node 20+** and npm
- **Rust stable** (`rustup`)
- Tauri system dependencies for your OS: <https://tauri.app/start/prerequisites/>
  - macOS: Xcode Command Line Tools
  - Windows: MSVC Build Tools and WebView2 (preinstalled on Windows 11)
  - Linux: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`
- **Docker**, for the local S3 server used in manual and automated tests

## First run

```bash
npm install
./scripts/dev-s3.sh up      # S3 on http://localhost:8333
./scripts/dev-s3.sh seed    # bucket "demo" with sample files
npm run tauri dev
```

On first launch Settings → Accounts opens. Enter:

| Field | Value |
|---|---|
| Endpoint | `http://localhost:8333` |
| Access key / Secret key | `devkey` / `devsecret123` |
| Path-style | ✓ |

Vite serves the UI on port **14321**. The usual 1420 was moved because it often clashes with
other Tauri projects. Frontend edits hot-reload. A change to any Rust file rebuilds and
restarts the app.

To reset the app to first-run state, delete the config file (paths in
[ARCHITECTURE.md → Persistence](ARCHITECTURE.md#persistence)) and clear localStorage (in
devtools: `localStorage.clear()`).

Website: `npm run site` serves `/site` exactly as Cloudflare will (headers, 404 page) on
http://localhost:8787. `python3 -m http.server 8765 --directory site` also works
(`.claude/launch.json`). It's plain HTML/CSS/JS with no build step.

## Checks (the same as CI)

```bash
npm run check                                          # tsc + prettier --check
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
NOEXPLORER_TEST_ENDPOINT=http://localhost:8333 NOEXPLORER_TEST_KEY=devkey NOEXPLORER_TEST_SECRET=devsecret123 cargo test
```

Nextcloud/WebDAV: `./scripts/dev-nextcloud.sh up && ./scripts/dev-nextcloud.sh seed`, then
`NOEXPLORER_TEST_NEXTCLOUD=http://localhost:8080 NOEXPLORER_TEST_DAV_USER=dev NOEXPLORER_TEST_DAV_PASS='NoExplorer-dev-2026!' cargo test nextcloud_roundtrip`
(lists, 40 MB chunked upload, download, ranges, shares, mkdir, delete). In the app, add a
*Nextcloud* account with `http://localhost:8080` and the same user, or use **Sign in with
Nextcloud…** and log in as `dev` in the browser.

Without the `NOEXPLORER_TEST_*` variables, `s3_roundtrip` and `nextcloud_roundtrip` are
skipped and the other tests still run. On a fresh clone, run `npm run build` once before bare `cargo` commands:
`generate_context!()` embeds `../dist`.

### What the tests cover

| Test | Covers |
|---|---|
| `util::tests::*` | Path traversal guard, never-overwrite naming, labels, error truncation on UTF-8 boundaries |
| `config::tests::*` | Endpoint normalization; path-style defaulting to on for partial or old configs |
| `local::tests::*` | Local listing (hidden files, crumbs), mkdir name validation |
| `transfer::tests::collect_uploads_*` | Folder walk keeps structure and skips junk files |
| `transfer::tests::s3_roundtrip` | Real server: create bucket → multipart upload of 40 MB → list → download and byte-compare → path-style presign → delete |

The frontend has no automated tests yet (roadmap). Use the manual checklist below for UI changes.

## Manual test checklist (before a release, or after UI changes)

Run against `dev-s3.sh` **and**, before a release, against a real MinIO.

- [ ] First launch: Settings → Accounts opens; **Test** shows the bucket list; **Save & use** loads the bucket
- [ ] Add a second account; switch with the picker (each account remembers its bucket/folder); remove one
- [ ] An old single-connection `config.json` still loads (migrated to one account)
- [ ] Σ Size on the current folder (path bar) and on selected folders (Size column), both panes
- [ ] Appearance: System/Light/Dark and all text sizes; the splitter drags and double-click resets it
- [ ] Wrong secret: Test shows a readable error and Save is blocked
- [ ] Secrets: after a launch `config.json` has `"secret_key": ""` and `"keychain": true`, the account still connects, and `security find-generic-password -s com.vnappmob.noexplorer.dev` lists it (macOS). Editing an account with the secret field left empty keeps the key
- [ ] Folder sizes: open a big folder; sizes and bars fill in while the Σ icon spins, the path bar shows the total. Create/delete a file in Finder: the size and the list update within ~1 s. Home shows "—" for Documents/Desktop/Downloads/Library until you open them (no macOS prompt); Σ scans them
- [ ] ⌘⌫ on a local file/folder: it lands in the Trash (Finder "Put Back" works) and the next row is selected
- [ ] Tray: lists the accounts (✓ on the active one); choosing another opens the window on it; with no account it offers "Add S3 Account…"
- [ ] Nextcloud: **Sign in with Nextcloud…** opens the browser; after approving, the account appears and lists files. Space previews an image, a PDF and a video (seeking works). Upload a folder and a > 16 MB file, download them back, Copy link gives a `/s/…` share, delete
- [ ] WebDAV (any server): list, preview, upload into a new sub-folder, download, delete
- [ ] A folder with thousands of files (`for i in $(seq 3000); do : > f$i; done`): splitter drag, scrolling to the end and arrow-key selection stay smooth
- [ ] Left pane: Places menu, breadcrumbs, ↑, filter, sorting, show hidden files
- [ ] Drag a file and a folder left → right; drop on a folder row and on a breadcrumb
- [ ] Drag right → left (file and folder). A name collision gives `name (1).ext`
- [ ] Drag a file from Finder/Explorer onto the right pane; dropping on the left pane shows a hint and does nothing
- [ ] Multi-select (⌘/Ctrl-click, Shift-click, ⌘A) and drag the selection
- [ ] A big file (> 1 GB): progress, speed, **Cancel** (no leftover `.part`, multipart aborted)
- [ ] Space preview: image, video, PDF, text (local and remote); ←/→ steps; Esc closes
- [ ] ↗ Open (remote file opens in the default app); 🔗 link copied and opens in a browser
- [ ] Delete a file and a folder on S3 (a confirmation dialog appears)
- [ ] New folder, local and remote
- [ ] Keyboard: Tab / ← → switch panes, Enter, Backspace, F5, ⌘↵
- [ ] Dark mode looks right

## Debugging

- **Devtools** are only available in dev builds: **⌥⌘I** on macOS, **Ctrl+Shift+I** on Windows and Linux. Right-click is taken by the app menu.
- **Rust logs:** `eprintln!` shows up in the terminal running `npm run tauri dev`.
- **S3 errors:** every error goes through `util::err`, which prints the full cause chain.
  "service error" alone usually means a wrong endpoint, the wrong path-style setting, or a
  clock-skew/signature problem.
- **Watching S3 directly:** `curl --aws-sigv4 "aws:amz:us-east-1:s3" --user devkey:devsecret123 "http://localhost:8333/demo?list-type=2"`

## Code style

- TypeScript: Prettier (120 cols), strict `tsc` with no unused variables. Add `src/<module>.ts`
  with a header comment that says what it owns, and keep the import direction described in
  [ARCHITECTURE.md](ARCHITECTURE.md#frontend-modules-src).
- Rust: `rustfmt.toml` (120 cols), clippy clean with `-D warnings`. Commands return
  `Result<_, String>` and map SDK errors with `util::err`.
- Write user-facing strings in English, sentence case, and keep them short.
