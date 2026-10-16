# Architecture Decision Records

Short records of *why* things are the way they are. Each has a status: **Accepted**, or
**Superseded by ADR-x**. To change one of these decisions, add a new ADR. Don't rewrite history.

Template:

```
## ADR-NNN: Title
Status · Date
**Context** — the problem / forces.  **Decision** — what we do.  **Consequences** — trade-offs, what to watch.
```

---

<a id="adr-001"></a>

## ADR-001: Tauri 2 (Rust and the system webview) instead of Electron
Accepted · 2026-10-15

**Context.** The app has to be cross-platform, start fast, and stay small. The tool it
replaces (Cyberduck) is heavy. Node and Rust were both available.
**Decision.** Use Tauri 2: a Rust core with `aws-sdk-s3`, and the UI in the OS webview
(WKWebView, WebView2, WebKitGTK).
**Consequences.** The macOS bundle is about 10 MB instead of about 150 MB. S3 and file I/O
happen natively in Rust, with real multipart concurrency. Webview engines differ between
OSes (see ADR-003). A Linux build needs the webkit2gtk-4.1 system packages.

<a id="adr-002"></a>

## ADR-002: No frontend framework
Accepted · 2026-10-15

**Context.** The UI is two lists, a few dialogs and a progress panel.
**Decision.** Use plain TypeScript and DOM APIs, with Vite as the dev server and bundler.
State lives in the `Pane` instances plus a small `app` object.
**Consequences.** The JS bundle is about 41 KB and there is no framework churn. Rendering is
manual: `Pane.render()` rebuilds rows and `Pane.paint()` only toggles selection classes.
Folders with tens of thousands of items render all rows at once (virtualization is on the
roadmap). If the UI grows a lot, revisit this (Svelte or Solid would be the light options).

<a id="adr-003"></a>

## ADR-003: Drag between panes uses mouse events, not HTML5 drag-and-drop
Accepted · 2026-10-15

**Context.** Files dropped from Finder/Explorer only arrive with absolute paths through
Tauri's native drag-drop handler (`dragDropEnabled: true`). On Windows that handler disables
HTML5 DnD inside the webview.
**Decision.** In-app drags are built on `mousedown` / `mousemove` / `mouseup` (`dnd.ts`):
a 5 px threshold, a floating ghost label, and drop targets found with `elementFromPoint`.
**Consequences.** The same behavior on every OS. Dragging *out* of the window to the OS
desktop isn't possible this way. That would need a native drag plugin (roadmap).

<a id="adr-004"></a>

## ADR-004: Checksums only `WhenRequired`
Accepted · 2026-10-15

**Context.** Recent `aws-sdk-s3` versions add CRC32 checksums to every request by default.
Some self-hosted or older S3-compatible servers reject them or mishandle them, especially
for multipart uploads.
**Decision.** `make_client` sets `request_checksum_calculation` and
`response_checksum_validation` to `WhenRequired`.
**Consequences.** Maximum compatibility. Integrity relies on TLS plus S3's own MD5/ETag
handling, as classic S3 clients do.

<a id="adr-005"></a>

## ADR-005: Transfer tuning — 6 files × 4 parts, 16 MB multipart threshold, parts ≥ 8 MB
Accepted · 2026-10-15

**Context.** The primary use is "back up as fast as possible" to a LAN or VPS MinIO.
**Decision.** Per job: `FILE_CONCURRENCY = 6`; multipart above 16 MB with
`PART_CONCURRENCY = 4`; part size `max(8 MB, size / 9000)`. Each part is read into memory
(not streamed).
**Consequences.** Worst-case memory is about 6 × 4 × part size, around 200 MB for typical
files. It grows with very large files, because part size scales to stay under 10 000 parts.
Many small files are handled well. Downloads stream with a 1 MB buffer and don't use ranged
parallel GETs yet (roadmap). Measured on a laptop against local SeaweedFS in Docker: about
70 MB/s, limited by the server.

<a id="adr-006"></a>

## ADR-006: The app never deletes or overwrites local files
Accepted · 2026-10-15 · amended by [ADR-023](#adr-023) (Move to Trash, never a permanent delete)

**Context.** This is a backup tool, so losing local data is the worst possible failure.
**Decision.** There is no local delete. A download that would collide gets the next free
`name (n).ext`. Bytes stream into `name.part` and are renamed only on success. Cancelled
or failed transfers remove their `.part` file.
**Consequences.** Re-downloading a folder creates `folder (1)/`. A "sync / overwrite" mode
would be an explicit opt-in feature.

<a id="adr-007"></a>

## ADR-007: Path-style addressing is on by default
Accepted · 2026-10-15

**Context.** MinIO and most self-hosted servers don't have wildcard DNS for
`bucket.host` (virtual-hosted style). This was the reason the original user had to tweak
Cyberduck.
**Decision.** `Config.path_style` defaults to `true` in Rust (`Default` + `#[serde(default)]`)
and in the Settings form.
**Consequences.** It works with self-hosted servers out of the box. AWS users can uncheck it.
AWS still accepts path style for existing buckets.

<a id="adr-008"></a>

## ADR-008: Credentials stored in a 0600 JSON file, not the OS keychain
Superseded by [ADR-020](#adr-020) · 2026-10-16

**Context.** The keychain is the gold standard, but cross-platform support is uneven: Linux
needs the Secret Service, and headless setups vary.
**Decision.** Store `config.json` in the app config dir with permissions 0600 on Unix.
**Consequences.** Simple and portable, and the same as most CLI tools (`~/.aws/credentials`).
On Windows the file inherits the user-profile ACLs. Moving to the keychain via the
`keyring` crate is on the roadmap.

<a id="adr-009"></a>

## ADR-009: SeaweedFS for local development and CI
Accepted · 2026-10-15

**Context.** The `minio/minio` Docker image is no longer published, and quay.io needs auth.
**Decision.** `scripts/dev-s3.sh` runs `chrislusf/seaweedfs` with an S3 identity file. The
CI round-trip test uses the same script.
**Consequences.** It is the same S3 API with path-style addressing. Behavior specific to
MinIO should still be smoke-tested by hand against a real MinIO before releases.

<a id="adr-010"></a>

## ADR-010: Progress rendering with `setTimeout`, not `requestAnimationFrame`
Accepted · 2026-10-15

**Context.** WebKit pauses rAF while the window is hidden or covered. Users start a big
backup and switch apps, and the panel looked frozen when they came back.
**Decision.** Throttle Transfers-panel renders with a 60 ms `setTimeout`. The backend
already throttles events to 4 per second.
**Consequences.** Progress is always current when the window comes back.

<a id="adr-011"></a>

## ADR-011: English UI, no i18n framework yet
Accepted · 2026-10-15

**Context.** The project is public and open source. The first user is Vietnamese.
**Decision.** All UI strings are English, inline in `index.html` and the TS modules.
**Consequences.** Adding i18n later means extracting strings into a dictionary module. They
are concentrated in `index.html`, `pane.ts`, `actions.ts`, `menu.ts`, `jobs.ts`,
`preview.ts` and `settings.ts`.

<a id="adr-012"></a>

## ADR-012: Releases are drafts and the website reads the latest release from the GitHub API
Accepted · 2026-10-15

**Context.** Signing and notarization can fail partway through. A broken release must never
reach the website.
**Decision.** `release.yml` creates a **draft**. A human checks the assets and then
publishes it. `site/` calls `api.github.com/repos/vnappmob/noexplorer/releases/latest`,
which ignores drafts, and picks the asset for the visitor's OS.
**Consequences.** The site never needs a redeploy for a new version. If the API rate limit
is hit, the buttons fall back to the Releases page.

<a id="adr-013"></a>

## ADR-013: Host the website on Cloudflare Workers static assets, not GitHub Pages
Accepted · 2026-10-15

**Context.** `vnappmob.com` already uses Cloudflare DNS. GitHub Pages needs manual DNS plus
a repo setting, has a slower CDN, and doesn't support custom response headers. Cloudflare
now recommends Workers static assets over Pages for new sites.
**Decision.** Serve `/site` with Workers static assets (`wrangler.jsonc`, no Worker script).
Deploys use Cloudflare **Workers Builds** (the Git integration): it watches the repo and runs
`npx wrangler deploy`. The custom domain is declared in config, so DNS and TLS are automatic.
**Consequences.** Faster global delivery, `_headers` for security and caching, a custom 404
page, preview URLs on pull requests, and an instant rollback (`wrangler rollback`). No API
tokens or secrets in GitHub. The trade-off: deploy logs and the repo connection live in the
Cloudflare dashboard, not in `.github/workflows`. (An earlier draft deployed through a GitHub
Action with `wrangler-action`. It was dropped because Workers Builds needs no secrets.)

<a id="adr-014"></a>

## ADR-014: Multiple S3 accounts live in one `config.json` (v2) with a single active client
Accepted · 2026-10-15 · extends ADR-008 · the single client is replaced by [ADR-025](#adr-025)

**Context.** Users back up to more than one server (home NAS, company MinIO, R2…).
**Decision.** `config.json` becomes `{profiles[], active, download_dir}`. The backend holds one
S3 client, for the active profile. Jobs clone it at start, so switching accounts mid-transfer is
safe. v1 files are migrated in memory on load (`parse_settings`) and written back as v2 on the
next save. Last bucket and prefix are remembered per profile in localStorage. Appearance
(theme, text size, split) is a per-device UI preference, so it lives in localStorage, not in
`config.json`.
**Consequences.** The remote pane always shows a single account. Two remote panes, or copying
between accounts, would need a client per pane (roadmap). Secrets for all accounts sit in the
same 0600 file. The keychain move (roadmap) should cover all profiles. (Done in ADR-020.)

<a id="adr-015"></a>

## ADR-015: Auto-update through Tauri's updater with GitHub Releases as the feed
Accepted · 2026-10-15

**Context.** Users shouldn't have to download and reinstall for every version.
**Decision.** `tauri-plugin-updater` with one endpoint:
`releases/latest/download/latest.json`. `tauri-action` generates that file and the `.sig` files
when `bundle.createUpdaterArtifacts` is on and `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` are set.
The app checks at startup and every 6 h (if `general.check_updates`), shows a banner, and
installs only after the user clicks (it asks first when transfers are running). The keypair was
generated on 2026-10-15. The private key and its password live in `~/.tauri/noexplorer-updater.*`
on the maintainer's machine and as repo secrets. The public key is embedded in `tauri.conf.json`.
**Consequences.** Shipping an update means publishing a release. Drafts are never served. **The
private key must be backed up.** Losing it means existing installs can't verify new updates, and
users must reinstall manually once. v0.1.0 has no updater, so its users need one manual upgrade.
Verified end-to-end on macOS: an installed v0.2.0 updated itself to v0.2.1 through the banner (2026-10-15).

<a id="adr-016"></a>

## ADR-016: Tray / menu-bar app with hide-on-close
Accepted · 2026-10-15

**Context.** Long backups should keep running when the user closes the window, and they should
be able to glance at progress.
**Decision.** A tray icon (template icon on macOS) with live transfer rows and a % title on
macOS. Closing the window hides it when `general.run_in_background` is on (the default). On
macOS the Dock icon is hidden too (`ActivationPolicy::Accessory`). Notifications fire when a
job finishes while the window isn't focused. `single-instance` and launch-at-login
(`--hidden`) complete the background-app behavior.
**Consequences.** "Quit" is now an explicit action (tray menu or ⌘Q). The tray menu is rebuilt
only when its rows change and texts update in place, so an open menu doesn't flicker. Linux
tray support depends on the desktop environment (AppIndicator).

<a id="adr-017"></a>

## ADR-017: AWS S3 is special-cased: SDK endpoint resolution and a region per bucket
Accepted · 2026-10-15

**Context.** A user pointed an account at `s3.amazonaws.com` with Region `ap-southeast-1` and
got `AuthorizationHeaderMalformed … expecting 'us-east-1'`. AWS signs per region, the global
endpoint only accepts us-east-1, and one AWS account's buckets can live in different regions.
Generic "endpoint + region" settings can't express that.
**Decision.**
- `config::is_aws()`: an empty endpoint or `*.amazonaws.com(.cn)` means AWS. For AWS the
  endpoint field is ignored and the SDK resolves `s3.<region>.amazonaws.com` itself. Bucket
  names with dots automatically fall back to path-style there.
- `region_from_endpoint()` reads the region out of regional hosts (AWS, Wasabi, B2,
  DigitalOcean, Linode), and `effective_region()` lets it win over the Region field, because a
  mismatch can never work.
- `AppState::client_for(bucket)`: on AWS, each bucket's region comes from `ListBuckets`
  (`BucketRegion`) or a `HEAD Bucket` (`x-amz-bucket-region`, also present on the 301).
  It's cached and served by a client for that region. Other providers always use the single client.
- The account form has provider presets (AWS, R2, B2, Wasabi, DO Spaces, self-hosted) with hints.
  Errors go through `util::humanize` (S3 code + message + hint) instead of the SDK's debug chain.
**Consequences.** AWS works with an empty endpoint and any default region, across regions,
with no settings to get wrong. Every command that talks to a bucket must use
`state.client_for(&bucket)` (not `state.client()`). The region cache resets whenever settings change.

<a id="adr-018"></a>

## ADR-018: Own launch-at-login module instead of tauri-plugin-autostart
Accepted · 2026-10-15

**Context.** With the plugin's LaunchAgent, macOS System Settings → Login Items → *Allow in
the Background* listed the item under the **code-signing developer's name** ("Khoa Luu")
instead of "NoExplorer". The plist had an empty `AssociatedBundleIdentifiers`, and the plugin
gives no way to set it. `SMAppService.mainApp` would show the app properly but can't pass
`--hidden` (start in the tray).
**Decision.** `autostart.rs` uses the `auto-launch` crate directly (the same crate the plugin
wraps). It keeps the LaunchAgent and sets `AssociatedBundleIdentifiers =
[com.vnappmob.noexplorer]`, so macOS 13+ attributes the background item to the app. The
label and app name are unchanged, so enabling overwrites the old entry. `refresh()` rewrites
an enabled entry at every launch, which repairs entries made by ≤ 0.2.1 and follows a moved app.
**Consequences.** Two Tauri commands replace the plugin's JS API. One less plugin and
capability. macOS may show a "Background Items Added" notice once when the entry is rewritten.

**Correction (v0.2.3).** Rewriting the plist was not enough. `sfltool dumpbtm` showed the
item with `Assoc. Bundle IDs: [com.vnappmob.noexplorer]` but still `Parent Identifier: Khoa
Luu`, because Background Task Management picks the parent only when the item is first
*added*. Removing the plist, waiting until BTM dropped the record (about 10 s), and adding
it back re-parented it to `2.com.vnappmob.noexplorer` (type app). `refresh()` now does that
once per install (marker file `autostart-entry-version` = `ENTRY_VERSION`, on a background
thread with a 12 s delay). Bump `ENTRY_VERSION` whenever entries must be re-created from scratch.
Verified on the maintainer's Mac (2026-10-15): after the remove → re-add, System Settings →
Login Items shows "NoExplorer". The automatic one-time path in `refresh()` uses the same
steps but hadn't run on a real stale install yet when v0.2.3 shipped.

<a id="adr-019"></a>

## ADR-019: v0.3 UI: a modern skin on the classic dual-pane layout
Accepted · 2026-10-15

**Context.** The maintainer wanted a clearer, current-looking UI that keeps the two-pane split
(the "previous generation" idea). The design was agreed on the canvas "NoExplorer UI Redesign"
before implementation.
**Decision.**
- Two header rows per pane: source chips (account › bucket as dropdowns, or "This computer")
  over a separate path bar with editable crumbs, copy and Σ.
- No transfer buttons between the panes: they made the window narrower. Transfers come from
  drag, the right-click menu, F5, or ⌘K.
- Context lives in floating layers: the activity card and an animated beam while transferring.
- A ⌘K palette for commands, accounts, buckets, places and files.
- Faint watermarks (a computer / clouds) mark local vs remote at a glance.
- Geist and Geist Mono are bundled (`@fontsource-variable/*`, no network). Stroke icons replace
  emoji. Tokens are redefined for light and dark, and everything sizes from `--fs`.
- The macOS title bar is an overlay, so the search bar shares the row with the traffic lights.
**Consequences.** About 70 KB of JS (was about 45 KB) plus about 150 KB of fonts. Windows and
Linux keep their native title bar above `#topbar`. Animations stop under
`prefers-reduced-motion`. Dropdowns use our own `popover.ts` instead of native `<select>`, so
keyboard access is implemented there.
**Amendment (v0.3.1).** The floating selection bar is gone: it was noisy and easy to click by
accident, and the right-click menu already has every action. The selection count and size show in
the column header instead.
**Amendment (v0.3.2).** The watermark is a fixed background at the bottom of the pane, behind the
transparent list (`.mark` z-index 0, `.list` z-index 1). Rows scroll over it, so it is always
visible. (0.3.1 stacked it after the last row, which still hid it in long folders.)

<a id="adr-020"></a>

## ADR-020: Secret keys in the OS credential store
Accepted · 2026-10-16 · supersedes [ADR-008](#adr-008)

**Context.** The app replaces Cyberduck, which keeps secrets in the Keychain, and it is used for
production buckets. Plain-text secrets in a 0600 file block other OS users. They don't stop any
process running as the same user, and they end up readable in backups. The UI also received every
secret through `get_settings`, which it never needed.
**Decision.**
- `secrets.rs` stores each account's secret with the `keyring` crate (v4, default mode): macOS
  Keychain, Windows Credential Manager, Secret Service on Linux (zbus, pure Rust). Entry =
  `service: <app identifier>`, `account: <profile id>`. Dev builds run with the `….dev` identifier,
  so they never touch the installed app's entries.
- `config.json` keeps `keychain: true` and an empty `secret_key`. When the store is unavailable
  (a minimal Linux box), the secret stays in the 0600 file as before, and saving retries the store.
- Existing files are migrated on launch (`load_settings`). Deleting an account deletes its entry.
- Secrets never go to the UI: every command returns `config::redacted()` settings. The form's
  empty secret means "keep the saved one" (`keep_secret`).
**Consequences.** One new dependency (`keyring` plus its platform stores). On macOS the item is
tied to the app's code signature, so signed updates read it without a prompt. If the signing team
ever changes, macOS asks once per machine. Ad-hoc-signed dev builds may prompt after a rebuild.
Copying `config.json` to another machine no longer carries the secrets, and downgrading to 0.3.x
shows an empty secret: re-enter it in Settings.

<a id="adr-021"></a>

## ADR-021: Virtualized file lists
Accepted · 2026-10-16

**Context.** Dragging the splitter in a 3,000-file folder took about 55 ms per mouse move (under
20 fps). Every width change re-laid out every row in both panes. Disabling animations, shadows,
`contain` and `content-visibility` made no measurable difference; only fewer row elements did.
**Decision.** `Pane` keeps only the rows near the viewport in the DOM (`drawRows`, 15 rows of
overscan on each side) inside `.rows`, which is as tall as all rows (`padding-top` = rows above).
Row height is measured from a rendered row and re-measured on the `textsize` event. Scroll and
resize call `drawRows` directly (no rAF, which pauses in covered windows), and it returns early
while the visible rows are already present. `selectIndex` scrolls with `reveal(i)` because the
target row may not exist yet.
**Consequences.** About 1 ms per splitter move in the same folder. Scrolling and refreshes are
cheap for very large buckets too. Rows must keep one fixed height. Code must read rows from
`view`/`selected`, never by querying `.row` elements, which only cover the visible window.

<a id="adr-022"></a>

## ADR-022: Own folder-size scanner and a live size index
Accepted · 2026-10-16

**Context.** The maintainer used ncdu for disk usage and wanted the local pane to show folder
sizes on its own, faster than ncdu and without a slow, blocking scan. Re-walking each folder on
every visit (the old `local_size`) repeats work. Scanning everything automatically would read
the disk constantly, and on macOS it would raise privacy prompts for Desktop/Documents/Downloads
just from opening Home.
**Decision.**
- `scan.rs` is our own parallel scanner, with no walker crate. On macOS one `getattrlistbulk`
  call returns name, type, device, inode, link count and size for a batch of entries, instead of
  one `stat` per file. Elsewhere `read_dir` is used. Workers go depth-first on their own stack and
  share half of it only while another worker is idle. Totals bubble up lock-free (a `pending`
  counter per folder). Rules: apparent size, hard links counted once, symlinks not followed,
  mount points not entered. Threads are capped at 6, where scaling stops.
- `sizes.rs` keeps an index (`path → Totals`) of finished roots, at most 6 roots and 1.5 M
  folders, least recently used dropped first. Each root is watched with the `notify` crate
  (FSEvents / ReadDirectoryChangesW natively recursive; inotify only on the root, because of its
  per-folder watch limit). A change re-reads just that folder and applies the difference to its
  ancestors. Watched roots below a new scan are reused instead of being walked again. The
  `notify` crate is a thin layer over the OS APIs, so writing our own gives no speed.
- The UI asks for sizes when the local pane lists a folder (`folder_sizes`). Results stream as
  `"dir-sizes"` events, batched every 100 ms and only for the folder on screen. The same event
  carries `relist` when that folder's own entries changed, which gives a live folder view.
- Automatic scans are polite: utility-QoS threads on macOS, same volume as the home folder only,
  and they skip macOS privacy-protected folders (Desktop, Documents, Downloads, ~/Library and its
  app-data folders, the Photos library) until the user has opened them in the app (remembered
  in `granted.json`). Skipped folders show "—" (their ancestors' totals are lower bounds, explained in a tooltip). Σ scans everything.
  Can be turned off in Settings → General.
**Consequences.** Measured on an M-series Mac with a warm cache: 224k files take 1.2 s with 1
thread and 0.55 s with 6. `du -sk` takes 3.6 s, ncdu 2.8 takes 6.4 s (1 thread) and 0.73 s
(`-t 8`). In the app, the first sizes show after about 0.6 s, the full folder after 1.2 s, and
a revisit after 0.13 s. A whole home folder (1.3 M files) takes about 6 s in the background.
Memory is about 100 bytes per indexed folder. Linux only follows changes at a root's top level.
Remote (S3) folder sizes stay manual (Σ): listing a prefix costs requests.

**Amendment (0.4.2).** No per-row bars. A storage bar under the local pane puts the current
folder in context: this folder, other used space and available space on its disk
(`volume_info`; on macOS `NSURLVolumeAvailableCapacityForImportantUsageKey`, the figure System
Settings shows). Lower bounds no longer get a "≥" prefix (a tooltip explains them).

<a id="adr-023"></a>

## ADR-023: Move to Trash for local files
Accepted · 2026-10-16 · amends [ADR-006](#adr-006)

**Context.** Cleaning up after spotting big folders (the ncdu workflow) needs a way to remove
local files. ADR-006 promises that local data is never lost.
**Decision.** The local pane gets **Move to Trash** (⌘⌫ / Delete, the context menu, ⌘K), with
no confirmation, like Finder and Explorer, because it is reversible. It never deletes
permanently. `trash.rs` uses each OS's own mechanism with no extra crate:
`NSFileManager trashItemAtURL` on macOS (Put Back works), and `SHFileOperationW` with
`FOF_ALLOWUNDO | FOF_WANTNUKEWARNING` on Windows, so Windows asks before anything it can't
recycle. Elsewhere it follows the freedesktop.org Trash spec (home trash, or
`$topdir/.Trash-$uid` on other filesystems, `.trashinfo` reserved with `O_EXCL`). It refuses
`/`, the home folder and its parents.
**Consequences.** ADR-006 still holds: no download overwrites and no permanent delete.
Transfers never trash anything.

<a id="adr-024"></a>

## ADR-024: Lean dev builds (desktop-only crate type, no dependency debug info)
Accepted · 2026-10-16 · see the storage policy in `CLAUDE.md`

**Context.** `src-tauri/target` had reached 22 GB and grew by GB a day. With the template
profile, one clean dev build plus clippy left 5.1 GB, 3.7 GB of it dependency objects with full
debug info. The template's `crate-type = ["staticlib", "cdylib", "rlib"]` also linked a
`libnoexplorer_lib.a` (0.73 GB) and a dylib on every dev build, and those only serve iOS/Android.
**Decision.**
- `[lib] crate-type = ["rlib"]` (desktop only; add the others back with a mobile target).
- `[profile.dev] debug = "line-tables-only"` (backtraces keep file:line for our code) and
  `[profile.dev.package."*"] debug = false` (no debug info for dependencies).
- No `split-debuginfo`: macOS already defaults to `unpacked` in dev, and on stable Rust Windows
  MSVC rejects `-Csplit-debuginfo=unpacked` (checked with rustc).
- Release builds are unchanged: LTO, one codegen unit, `opt-level = 3`, abort on panic, stripped.
**Measurements** (clean target, `CARGO_INCREMENTAL=0`, `cargo build` then `cargo clippy`):

| Profile | build | clippy | target | deps |
|---|---|---|---|---|
| Template (before) | 44 s | 26 s | 5.1 GB | 3.7 GB |
| `line-tables-only` + `split-debuginfo = "unpacked"` | 39 s | 24 s | 4.1 GB | 2.9 GB |
| **This ADR** | **37 s** | **15 s** | **2.1 GB** | 1.6 GB |

**Consequences.** A debugger shows dependency frames without source. Set `debug = true`
locally to step into a crate. `scripts/clean-build.sh` still keeps the total bounded over time.

<a id="adr-025"></a>

## ADR-025: Providers behind one `Remote` interface, one client per account
Accepted · 2026-10-16 · replaces ADR-014's single active client

**Context.** Nextcloud/WebDAV and Box accounts are next, and the remote pane will split into
several views, each on its own account. Every command used to call `state.client_for(bucket)` on
one S3 client for the active profile.
**Decision.**
- `provider.rs` defines `enum Remote` with one variant per provider (`S3` today). It uses static
  dispatch: no `dyn` and no async-trait crate. Each method (`containers`, `list`, `list_all`,
  `link`, `read_head`, `mkdir`, `delete`, `upload_file`, `download_file`) is a `match` on the
  variant. Locations are `(container, key)`: S3 container = bucket, folder keys end with `/`.
- `s3.rs` owns everything S3: client construction (path-style ADR-007, checksums ADR-004), the
  per-bucket region cache on AWS (ADR-017), multipart upload, streamed `.part` downloads.
- `AppState::remote(account)` caches one `Arc<Remote>` per profile id. `None` means the active
  account. A settings change drops only the clients of accounts that changed. Running jobs hold
  their own `Arc`.
- Every remote command takes an optional `account`, and `"job"` events carry it.
  `Profile.kind` (default `s3`) selects the provider.
**Consequences.** No UI change yet; the S3 round-trip test passes unchanged. A new provider is a
new variant plus its file. Transfers stay generic: `transfer.rs` only schedules files.
