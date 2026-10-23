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

**Update 2026-10-20 (per provider).** `Remote::file_concurrency()` sets files in flight per provider:
6 by default, 3 for OneDrive. The maintainer saw one file upload to OneDrive faster than several at
once; Graph throttles a user's concurrent requests with 429/503 + Retry-After, so extra parallel
files mostly add waiting. Copies between accounts use the smaller of the two. Throttling is
logged (`diag.rs`, path only) so slow runs can be told apart from throttled ones.

**Update 2026-10-22 (moves within a bucket).** S3 and Azure have no rename: moving a folder copies
every object on the server, then deletes the originals. That ran one object at a time, so a folder
of small files waited one round trip per object (500 objects on a local server: 0.9–1.4 s; over the
internet at ~50 ms a request, ~25 s). Copies now run `COPY_CONCURRENCY` = 16 at once (0.27 s on the
same server); the originals are still deleted only after every copy succeeded.

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
**Update 2026-10-18.** An experimental Vietnamese UI (0.4.10, a DOM-rewriting MutationObserver)
froze the window and was removed. The maintainer doesn't want translations: the UI stays English only.

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
macOS the Dock icon is hidden too (`ActivationPolicy::Accessory`). (Notifications when a job
finished were removed on 2026-10-17: the maintainer doesn't want them, since each platform
handles them differently and macOS asks for permission.) `single-instance` and launch-at-login
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

<a id="adr-026"></a>

## ADR-026: Nextcloud and WebDAV through our own small client, previews through `nxr://`
Accepted · 2026-10-16

**Context.** The maintainer wants Nextcloud (and NAS/WebDAV) accounts next to S3, without
sync-client folders. WebDAV has no presigned URLs, so `<img>`/`<video>` can't reach files that
need an auth header.
**Decision.**
- `webdav.rs` is a `Remote` variant (`ProviderKind::Nextcloud | Webdav`). It is a small client
  on the `reqwest`/`rustls` (ring)/`quick-xml`/`percent-encoding` builds already in the tree, so
  no new compile units, and it doesn't use a WebDAV crate. It covers PROPFIND Depth 1 (recursive
  listings walk level by level, 6 at a time, because Depth: infinity is often disabled), GET with
  Range, PUT (streamed, counting progress), MKCOL (parents created as needed), DELETE and MOVE.
  It uses Basic auth with the user name and an (app) password.
- Nextcloud extras: the root is `<server>/remote.php/dav/files/<user>/`; files over 16 MB use
  chunked upload v2 (MKCOL session → numbered PUTs → MOVE `.file`, session deleted on
  failure); "Copy link" creates an expiring public share through the OCS API.
- Sign-in uses **Login Flow v2** (`nextcloud_login`): the browser approves, the app polls,
  and the returned app password goes straight to the keychain (ADR-020). It never reaches the
  UI. Typing a user name and an app password also works.
- Previews use a custom scheme `nxr://localhost/<account>/<container>/<key>`
  (`http://nxr.localhost/…` on Windows), served by `proxy.rs` through `Remote::read_range`.
  Range requests are honored (video seeks; ranged answers are capped at 8 MB, whole files at
  512 MB). CORS headers are set because the page's origin differs. `Content-Length` is left to
  the webview, since a duplicate stops AVFoundation from playing media.
- WebDAV accounts have one container, "Files", and the UI hides the bucket picker for them.
**Consequences.** Tested end to end against Nextcloud 31 in Docker
(`scripts/dev-nextcloud.sh`): listing, 40 MB chunked upload, upload into new folders,
byte-exact download, ranged reads, shares, mkdir, delete, and in-app previews (image, video seek,
PDF). Plain WebDAV can't make share links. Listing a very large tree is one PROPFIND per folder.

<a id="adr-027"></a>

## ADR-027: Box through the user's own OAuth app (beta)
Accepted · 2026-10-16

**Context.** NoExplorer is open source, so it can't ship a Box client secret. The maintainer
chose bring-your-own-app, the way rclone and Cyberduck handle it. There was no Box account
available to test against while building it.
**Decision.**
- `box_api.rs` is a `Remote` variant (`ProviderKind::Box`). It talks to the REST API on the shared
  reqwest/rustls client, with SHA-1 and base64 from crates already in the tree.
- Sign-in (`box_login`): the user's Client ID and secret, the browser's authorize page, and a
  loopback listener on the fixed redirect `http://127.0.0.1:53682/callback` (registered in the
  Box app) with a `state` check. The code is exchanged for tokens.
- Tokens: the access token stays in memory. The refresh token rotates on every refresh, so it is
  saved to the keychain each time (`box-refresh:<profile id>`, via the global store that
  `secrets::init` sets up). Refreshes are serialized, a 401 refreshes once and retries, and a 429
  waits for Retry-After.
- Paths are mapped to IDs through a cache that listings fill; a missing path resolves by listing
  its parents (Box forbids duplicate names in a folder).
- Uploads under 50 MB use one multipart POST, built by hand and streamed (no `multipart`
  feature). Larger files use an upload session: parts read in order (whole-file SHA-1), 4 in
  flight, each with its own SHA-1, then a commit that retries on 202. An existing name gets a new
  version instead of a conflict. Deletes go to the Box trash. "Copy link" makes an open shared
  link that expires.
**Consequences.** Covered by `box_roundtrip_against_mock`, an in-memory Box API that checks part
and file digests, paging, token rotation, 401 retry, versions, ranges, shares and trash. It has
**not yet run against the real Box service**, so it is labeled beta in the UI until the
maintainer confirms it. Expiring links may be refused on free Box plans; Box's message is shown.

<a id="adr-028"></a>

## ADR-028: Compare folders with server-side hashes
Accepted · 2026-10-16

**Context.** After uploading a folder, the maintainer saw 1,932 files locally and 1,928 on the
remote. They needed a way to see exactly what differs and to prove that the files that are
there are intact.
**Decision.**
- `compare.rs` pairs files by path relative to the two roots: missing on the remote, missing
  locally, size differs. Junk the uploader skips on purpose (`SKIP_NAMES`: `.DS_Store`,
  `Thumbs.db`, `desktop.ini`) and symlinks (never followed) are reported separately, and the
  dialog explains the count gap in plain words.
- "Verify" checks content without downloading. The local file is hashed (in parallel, up to 6
  files) and compared with what the server already has: S3 ETag (MD5; multipart = MD5 of part
  MD5s, tried with NoExplorer's own part size first, then common sizes from aws-cli, rclone,
  s3cmd…), Box SHA-1, or Nextcloud `oc:checksums`. When no hash can be reproduced (SSE-KMS
  ETags, unknown part sizes, plain WebDAV), the file counts as "same size", never "verified".
- Fixes are one click: `upload_rel` (missing and changed files) and `download_rel` (files only
  on the remote; `safe_rel` on server paths, never overwriting locally).
**Consequences.** Tested on dev S3 (`compare_s3_roundtrip`: deleted, remote-only, same-size
edit, size change, junk, a 20 MB multipart file verified). Hashing reads every compared byte
once (twice for multipart files with two candidate part sizes).

<a id="adr-029"></a>

## ADR-029: Split remote views
Accepted · 2026-10-16

**Context.** People with several accounts (a NAS, AWS, Nextcloud) had to switch the single
remote pane back and forth, and couldn't copy between two remotes without going through a
local folder.
**Decision.**
- The right column holds 1–3 remote views stacked top/bottom (not tabs: both must be visible to
  drag between them). Each is a full `Pane` cloned from `#remote-tpl` with its own account,
  bucket and folder. The global `app.bucket` and "active profile as current client" were
  removed; every command takes the view's `account` (already possible since ADR-025). The
  account picked in a view still becomes the default for new windows and the tray.
- `panes.remote` stays as the target for local → remote actions and means "the remote view that
  last had focus", marked with an accent bar when several are open.
- Remote → remote drops call `copy_remote`. Same S3 account: `CopyObject` on the server (single
  call, so ≤ 5 GB; bigger files fall back to the relay). Anything else: download each object to
  a temp file under the app temp dir, upload it, delete it. The job counts bytes twice (down +
  up) so the progress bar is honest.
- The layout is per device (localStorage `"views"`), like the pane split. Views on a deleted
  account fall back to the default account.
**Consequences.** Copies between providers use local disk for one file at a time and are as slow
as download + upload. A direct stream (pipe the GET body into the PUT) is a later optimisation;
it needs a size-aware streaming upload for each provider. Tested in the dev app against
SeaweedFS + Nextcloud (split, drag S3 → Nextcloud folder, close, reload restores views) and by
`s3_roundtrip` (server-side copy of a key with spaces, Unicode and `#`).

<a id="adr-030"></a>

## ADR-030: Windows code signing through SignPath Foundation
Rejected · 2026-10-17 (SignPath declined the project; see the update at the end)

**Context.** Unsigned Windows installers show "Windows protected your PC" and "Unknown
publisher". A code signing certificate costs money every year and now needs a hardware token or
a cloud HSM. Azure Artifact Signing is cheap but only open to individuals in the US and Canada.
**Decision.**
- Use SignPath Foundation's free certificate for open-source projects. The publisher shown is
  "SignPath Foundation", not VNAppMob; that's the price of free.
- Sign twice per release, in the `windows` job of `release.yml`: the app binary first (built with
  `tauri build --no-bundle`), then the installers that `tauri bundle` builds around it. SignPath
  can't look inside an NSIS installer, so signing only the installers would leave the installed
  `noexplorer.exe` unsigned. Each round needs a manual approval (a SignPath Foundation rule).
- SignPath's GitHub connector only signs workflow artifacts, which proves the files came from
  this repository's CI. So signed Windows builds skip `tauri-action`. Updater signatures are made
  again after Authenticode signing, and a final job merges the Windows entries into
  `latest.json`, after every other job has written its own.
- Everything is behind the `SIGNPATH_ORGANIZATION_ID` repo variable, so releases keep working
  unsigned until the project is approved.
**Consequences.** Releases wait for two approvals (the timeout is one hour each). The signed
path can't run before approval, so it has only been checked statically (actionlint, the
`latest.json` merge script with sample signatures). The first signed release needs an eye on
the Windows job and a check of the installer's signature and the auto-update.

**Update (2026-10-17).** SignPath Foundation declined the application. The SignPath flow was
removed from `release.yml` (Windows builds in the matrix again, unsigned), along with
`.signpath/` and the `latest.json` merge script. Windows stays unsigned until there's budget for
a certificate; Tauri's own `bundle.windows` signing would be used then, which needs none of that
machinery (docs/RELEASING.md → Windows signing).

<a id="adr-031"></a>

## ADR-031: Experimental features behind Settings → Experimental
Accepted · 2026-10-18

**Context.** A batch of large features (file operations, resumable transfers, backup pairs, grid,
tabs, three providers, and a native look and a Vietnamese UI, both since removed) landed at once, mostly tested against local
servers and a browser harness rather than daily use.
**Decision.** Each ships behind its own toggle in Settings → Experimental (`general.experimental`
in config, `exp("key")` in the frontend), **off by default**. Turning one off hides its UI again;
data it created (favorites, backup pairs, pinned certificates) stays. A feature graduates when it
has had real-world use, by deleting its `exp()` checks.
**Consequences.** Existing users see no change until they opt in. Code paths stay in every build
(the toggle is a runtime switch, not a Cargo feature), so CI compiles and tests them all.

<a id="adr-032"></a>

## ADR-032: Self-signed HTTPS by certificate pinning
Accepted · 2026-10-18

**Context.** NAS boxes and home servers often use self-signed certificates. The usual "ignore
certificate errors" switch disables protection against interception for that account.
**Decision.** Pin one certificate per account: the form reads the server's SHA-256 fingerprint,
the user compares and confirms, `Profile.cert_pin` keeps it. A custom rustls verifier accepts that
exact certificate and sends everything else to the platform verifier (`tls.rs`). WebDAV uses it
directly; the AWS SDK's TLS can't take a verifier, so pinned S3 accounts swap the SDK's HTTP
client for reqwest (`PinnedHttpClient`). TLS is never switched off.
**Consequences.** A renewed self-signed certificate needs trusting again (by design). Tested
against a Caddy proxy with an untrusted CA, including a multipart upload through the swapped
client.

<a id="adr-033"></a>

## ADR-033: Google Cloud Storage, Azure Blob and SFTP
Accepted · 2026-10-18

**Decision.**
- GCS is an S3 preset (XML API with HMAC keys); nothing new in Rust.
- Azure Blob is a `Remote` variant over its REST API with Shared Key signing (`azure.rs`),
  reusing reqwest, hmac and sha2 from the tree. Tested against Azurite.
- SFTP runs the system `ssh` (`-s sftp`, BatchMode, `StrictHostKeyChecking=accept-new`) and speaks
  SFTP over its stdio with `russh-sftp`. A pure-Rust SSH stack (russh) would add 76 crates; this
  adds 5, and the user's keys, agent, `~/.ssh/config` and `known_hosts` apply as in a terminal.
**Consequences.** SFTP needs OpenSSH installed (macOS, Linux, Windows 10+ have it) and has no
password login yet. Tested against atmoz/sftp.

<a id="adr-034"></a>

## ADR-034: Website on Astro + Starlight
Accepted · 2026-10-18

**Context.** The docs were pre-built HTML from a small generator: fast, but a full page load per
click, no search, and hand-made SEO.
**Decision.** `site/` is an Astro project (its own `package.json`): the home page is an Astro page,
the docs are Markdown rendered by Starlight (Pagefind search, sitemap, dark mode), with link
prefetch on hover. `wrangler.jsonc`'s `build.command` builds it during `wrangler deploy`, so
Cloudflare Workers Builds needed no dashboard change. CI builds the site too.
**Consequences.** The site needs `npm ci` in `site/` to build (Cloudflare does it). Old
`/docs/<page>` URLs redirect to `/docs/<page>/`.

<a id="adr-035"></a>

## ADR-035: Custom-protocol answers go out on the main thread; local media through nxr://
Accepted · 2026-10-18

**Context.** 0.4.12 aborted while scrolling the grid. wry answers a custom-protocol request with
`didReceiveResponse`/`didReceiveData`/`didFinish` after checking the request is still running,
but WebKit stops requests on the main thread (a tile scrolled away) while the answer came from a
tokio worker. A stop between the check and the answer makes WebKit throw an Objective-C
exception; wry catches it with `objc2::exception::catch`, which can't work under
`panic = "abort"` (a foreign unwind into abort-mode Rust aborts). Tauri's own asset protocol,
used for local thumbnails, answers from a worker the same way.
**Decision.** `nxr://` hands its answer to the main thread (`proxy::respond`), so the check and
the answer can't be split by a stop. Local files are served by `nxr://localhost/~local/f/<path>`
(`files.ts` `localSrc`) with Range support; the asset protocol and the `protocol-asset` feature
are removed.
**Consequences.** One scheme for every preview. Answers wait for a free main-thread slot (they're
small and cheap to hand over). Switching the release profile to `panic = "unwind"` would make
wry's catch work too; that is a profile change for the maintainer to approve.

<a id="adr-036"></a>

## ADR-036: Grid thumbnails made by the backend, cached in memory and on disk
Accepted · 2026-10-18

**Context.** The grid showed each image file itself: whole photos downloaded and decoded per tile,
and because the grid is virtualized, every redraw while scrolling dropped them and loaded them
again (remote Nextcloud files had no HTTP cache at all through `nxr://`).
**Decision.** `thumbs.rs` makes 256 px thumbnails (JPEG q80, PNG when the image has transparency,
EXIF orientation applied) with the `image` crate, which Tauri already builds; only the JPEG, WebP
and GIF decoders are new. Two LRU caches: 16 MB in memory, 64 MB on disk in the OS cache folder,
trimmed by modified time (a disk hit refreshes it) down to 56 MB. The key hashes account, container,
key, size and modified time, so edits invalidate. At most 4 thumbnails are made at once. The
frontend keeps loaded elements (120 images, 8 video frames) and moves them into the new tiles on
redraw, and requests the next screen's thumbnails at low priority. Formats the crate can't read
(HEIC, TIFF) fall back to the file itself. Videos keep their live frame (no codec library).
**Consequences.** Scrolling back is instant and costs no network. Thumbnails of remote files sit
unencrypted in the user's cache folder, like any file manager's. 25 MB remote / 64 MB local caps
on originals.

<a id="adr-037"></a>

## ADR-037: Previews are `no-store`; WebKit's disk cache is trimmed at launch
Accepted · 2026-10-19

**Context.** After 0.4.13 the app's cache folder passed 100 MB. Not the thumbnails (ADR-036) but
WebKit's own HTTP cache (`Caches/<id>/WebKit/NetworkCache`): every photo and video opened in quick
look through a presigned link was stored, and WebKit sizes that cache from the disk size.
**Decision.** Preview responses say `Cache-Control: no-store`: S3 presigned links ask for it
(`response-cache-control`, inline links only), Azure SAS links sign `rscc=no-store`, and `nxr://`
sends it (thumbnails stay `immutable`). A tiny plugin registered right after single-instance
deletes WebKit's `NetworkCache` and `MediaCache` (macOS) at launch when they're over 16 MB; plugins
are set up before the window exists, and a second launch exits inside single-instance first, so a
running app's cache is never touched. The thumbnail disk cap drops to 48 MB (trimmed to 40 MB).
**Consequences.** On disk: ≤ 48 MB thumbnails + ≤ 16 MB WebKit cache at launch, plus whatever
WebKit keeps during one session (only non-preview responses now). Reopening a previewed file
downloads it again. WebView2 and WebKitGTK manage their own caches.

<a id="adr-038"></a>

## ADR-038: Four experiments become standard; tabs and history are removed
Accepted · 2026-10-19

**Context.** After a few days of use the maintainer judged rename / move / duplicate, resumable
transfers, the thumbnail grid and favorites good enough to keep, and tabs / back-forward history not
worth their space (ADR-031 batch).
**Decision.** Those four are always on: their `exp()` gates and the `file_ops`, `resume`, `grid`,
`tabs` fields of `Experimental` are gone (old configs still load; unknown fields are ignored).
Every tracked job is resumable (`Job::resume`), and the speed limit moves to Settings → General.
Favorites keep the path-bar star and live in `favorites.ts`; `nav.ts` (tabs, history, ⌘[ ⌘] ⌘T
Ctrl+Tab, mouse back/forward) is deleted, and ⌘W only closes a remote view again.
**Consequences.** Fewer header buttons and shortcuts. The pause button, resume banner, F2/⌘Z,
same-pane drag-to-move and the grid button show for everyone, so their docs moved from the
Experimental page into Moving files and Compare, sizes & more.

<a id="adr-039"></a>

## ADR-039: SFTP passwords and key passphrases through SSH_ASKPASS
Accepted · 2026-10-19

**Context.** SFTP ran `ssh` in batch mode (ADR-033), so servers that want a password and keys with
a passphrase simply failed ("Permission denied"), with no way in from the app.
**Decision.** Keep the system `ssh` (keys, agent, `~/.ssh/config`, known_hosts) and answer its
prompts with `SSH_ASKPASS` + `SSH_ASKPASS_REQUIRE=force`, pointing at the app's own binary. In that
mode `run()` exits before any UI: it connects to a one-shot loopback listener the app opened for
this `ssh` start (`NOEXPLORER_ASKPASS=<port> <256-bit token>`), sends the token and the prompt, and
prints the answer. The listener answers at most 4 times within 60 s and is dropped once the session
is up. The account's secret field holds the password or the key passphrase (keychain, ADR-020),
`key_file` adds `-i`. With no secret, batch mode stays on. `NumberOfPasswordPrompts=1` avoids
lockouts from retries. Clear hints for "Permission denied" with and without a saved secret.
**Consequences.** Password servers and locked keys work; the secret never sits in an environment
variable, a file or a command line. A pure-Rust SSH client (russh) would avoid the subprocess but
adds a crypto stack and loses `~/.ssh/config`.

<a id="adr-040"></a>

## ADR-040: Settings as a sidebar of card pages; a provider picker; Back without Forward
Accepted · 2026-10-19

**Context.** Settings had grown into a tab strip over one long column per page: a plain provider
dropdown with twelve entries, every account field in one list, checkboxes with long captions. The
maintainer asked for a modern layout. Separately, tabs and two-way history were removed (ADR-038)
but going back to the folder you were in, as you left it, was missed.
**Decision.**
- The dialog is a sidebar of sections (icons) and pages of cards. Rows put the label and a short
  explanation on the left and a switch (a styled `<input type=checkbox class="switch">`, so forms,
  keyboard and `change` handlers are unchanged) or control on the right.
- Accounts: a list with provider badges (monograms on a hue from `providers.ts` `LOOK`, not vendor
  logos) and a ✓ for the default account. A new account starts with a grid of storage types grouped
  as object storage / file servers / cloud drives; "Change provider" reopens it. The form is grouped
  (hint, Connection, Sign in, collapsible Options) with Test, Save and the test result in a fixed
  footer. The hidden `<select id="acct-provider">` stays the single source of the chosen provider,
  so the existing logic (`applyPreset`, `readForm`, certificates) didn't change.
- Back (experimental `back_history`, `history.ts`): per pane, a stack of left locations (account,
  bucket, path) with scroll and selection. `Pane.go` and the account/bucket/favorite switches call
  `paneHooks.leaving` to snapshot synchronously (scroll events arrive a frame late, and not at all
  in a hidden window). No Forward.
**Consequences.** Every settings page scrolls on its own inside a fixed-size dialog; at the minimum
window (820 × 480) the sidebar and account list narrow but stay. New per-account fields go into one
of the form's groups.

<a id="adr-041"></a>

## ADR-041: Back means "up, as you left it"; ↑ can open a folder tree
Accepted · 2026-10-19

**Context.** The first Back (ADR-040) followed history, like a browser. The maintainer expected it to
go up a level, with the parent the way they left it, and wanted ↑ to become a quick jump instead.
**Decision.**
- `history.ts` remembers, per pane, each folder's scroll and selection when it's left
  (`paneHooks.leaving`, from `Pane.go`, `Pane.up` and the account/bucket switches; at most 400
  folders). `Pane.up` calls `paneHooks.goingUp`; with `back_history` on, the next listing of the
  parent restores its spot. ⌘[, ← and the mouse's back button are `up()`; so are ↑ ⌘↑ Backspace.
- `tree.ts` (`folder_tree`): every listing records the folder's subfolders (folders only, per local
  disk or account + bucket, in memory, at most 5000 listings). ↑ opens a dropdown tree: the
  breadcrumb chain expanded with the siblings seen at each level, a ▸ on any folder that was
  listed, a filter over every known folder, keyboard (↑↓ ← → Enter Esc). ⌘↑ and Backspace still go
  up.
**Consequences.** The tree only shows folders opened since launch; persisting it (or seeding it from
the local size scanner, which already walks folders) is possible later. No Forward.

<a id="adr-042"></a>

## ADR-042: Search below a folder; the provider list as a dropdown
Accepted · 2026-10-19

**Context.** The filter box only filters the folder on screen, but it looks like a search, and the
maintainer wanted a real one, as fast as possible. Separately, the provider grid (ADR-040) made going
back a detour ("Change provider").
**Decision.**
- `search.rs`: a query is words (all must be in the name, any case), `*`/`?` globs over the whole
  name, and `ext:`. Locally, a parallel walker (up to 8 threads, a shared queue) reads folders with
  `scan::read_dir` (one `getattrlistbulk` per batch on macOS), stays on one disk, skips hidden
  entries unless shown and the same private folders as automatic size scans (`enter_policy`, no
  permission prompts), and streams hits every 100 ms as `"search"` events until 10,000. Remotes
  list everything below the folder (`Remote::list_files`) and match files plus the folders on their
  paths. `cancel_search` stops a walk.
- UI (`search.ts`): ↵ in the filter box runs the search in the pane: rows show the folder they're in,
  a bar shows progress, Stop / ✕ / Esc / any normal load end it, right-click → Show in its folder.
  Folder-level actions (rename, move, new folder…) are hidden while results are shown.
- Measured (debug build, warm cache): `/System/Library` 2.6–2.9 s vs 13–16 s for
  `find -xdev -iname`, first results after ~0.1 s; `/usr` same hits as `find`.
- The account form picks the provider from a dropdown (badges, groups) built inside the dialog: a
  modal `<dialog>` sits above the app's shared popover.
**Consequences.** Remote search costs a full listing (one request per 1,000 keys on S3); streaming
pages and server-side search (Nextcloud SEARCH, Box search API) are possible later. Spotlight isn't
used: it skips unindexed folders and would need its own permission story.

<a id="adr-043"></a>

## ADR-043: Three more features standard; Oracle Cloud as an S3 preset
Accepted · 2026-10-19

**Context.** The maintainer judged Create buckets, Back (up a level, as you left it) and the folder
tree ready, and asked whether Oracle Cloud Object Storage needs its own provider.
**Decision.**
- The three lose their toggles (ADR-038 recipe). The path bar's ← is always shown and goes up as you
  left it (so do ⌘[, ⌘↑, Backspace, the mouse's back button); ↑ always opens the folder tree.
- Oracle Cloud is an S3 preset, not a provider: its Amazon S3 Compatibility API takes SigV4, a
  Customer Secret Key, path-style `https://<namespace>.compat.objectstorage.<region>.oci.customer-oci.com`
  (or `.oraclecloud.com`), the OCI region id to sign with (read from the endpoint,
  `region_from_endpoint`), and wants checksums only when required (as ADR-004 already does; newer
  SDK defaults break it). Multi-delete, multipart and copies are supported; the compartment for S3
  buckets is set on Oracle's side.
**Consequences.** Not yet run against a real Oracle tenancy; the diagnostic log
(`diag.rs`) will show the server's answer if something differs (ListObjectsV2 isn't on Oracle's
list, though it's widely used with it).

## ADR-044: Simple account setup builds the endpoint

**Context.** Every S3 preset asked for a full endpoint URL, which users had to find in each
provider's docs. Azure already built its endpoint from the account name, and that was easier.

**Decision.** `SETUP` in `providers.ts` gives a preset a few fields (R2 account ID; B2, Wasabi,
DigitalOcean region; Oracle namespace + region; none for AWS and GCS), an `endpoint()` that builds
the URL and a `parse()` that reads one back. The form (`settings.ts` `renderSetup`) shows those
fields instead of the endpoint box and writes the endpoint into the hidden field, so the backend
and `config.json` still only see an endpoint. A saved endpoint that `parse()` can't show (an R2
jurisdiction, a private address) opens in the raw endpoint box. A pasted endpoint, in any field,
is recognized with `guessProvider` and fills the form.

**Consequences.** Nothing changes in Rust or in saved accounts. Region lists are suggestions
(`<datalist>`), so a new region still works by typing it. Saving an Oracle account rewrites an
old `…oraclecloud.com` endpoint to the `…oci.customer-oci.com` form, which Oracle documents as
equivalent.

## ADR-045: OneDrive through Microsoft Graph with NoExplorer's own public app

**Context.** Users asked for OneDrive. Google Drive was dropped because its file scopes need a paid
security assessment; Box needs each user to create their own OAuth app. Microsoft's identity
platform lets a desktop app be a *public client*: it ships its client ID and has no secret.

**Decision.** NoExplorer has one app registration in Microsoft Entra (multi-tenant + personal
accounts, "Mobile and desktop" redirect `http://localhost`, public client flows on; delegated
`Files.ReadWrite`, `offline_access`, `User.Read`). Its client ID is a constant in `onedrive.rs`.
Sign-in is authorization code + PKCE through the browser to a random loopback port (Entra ignores
the port of a localhost redirect). The refresh token lives in the keychain; Graph is addressed by
path (`/me/drive/root:/a/b:`), so no ID cache. Uploads ≤ 4 MB are one PUT; larger ones use an
upload session in 10 MiB chunks (a multiple of 320 KiB), sent without the bearer token. Copies are
asynchronous on Microsoft's side and followed through the monitor URL. OneNote notebooks and
items shared from other drives are left out of listings (they can't be addressed by path here).
Ships behind Settings → Experimental until it's been used on real accounts.

**Consequences.** Users click one button. Work and school tenants that block user consent need an
admin to approve the app; publisher verification (free, through Microsoft Partner Center with the
vnappmob.com domain) makes that rarer. Compare can only verify files on personal accounts (SHA-1);
work accounts report only quickXorHash, so they count as "same size". Tested against a mock Graph
(`onedrive_roundtrip_against_mock`); the diagnostic log drops query strings, which carry
pre-authorized upload and download tokens.

## ADR-046: OneDrive standard; bigger upload chunks, parallel downloads

**Context.** The maintainer confirmed OneDrive on a real account and verified VNAppMob as the app's
publisher (2026-10-20), but found OneDrive much slower than MinIO or Azure. Part of that is the
service: an internet round trip per request and Microsoft's throttling, against a server on the
LAN. Part was ours. An upload session takes its chunks strictly in order, one request at a time
(S3 and Azure send 4 parts at once), and we used 10 MiB chunks, so a 1 GB file waited on ~100
round trips plus Microsoft's processing between them. Downloads came down one connection, which
OneDrive's download servers often cap per connection.

**Decision.** `onedrive_beta` is removed (graduated as ADR-038 describes). Upload chunks are 50 MiB
(160 × 320 KiB, under Graph's 60 MiB per request). Files ≥ 32 MiB download as 16 MiB ranges, 4 at
once, from the item's pre-authorized `@microsoft.graph.downloadUrl`, written at their offsets in
the `.part`; a server that ignores Range falls back to one stream.

**Consequences.** A failed chunk costs up to 50 MiB to resend. A parallel download that fails
removes its `.part` instead of keeping it for resume (it has gaps); resumed downloads and files
under 32 MiB keep the single-stream path. Not measured against real OneDrive yet: the mock only
proves the ranges and chunk order.

## ADR-047: Bucket access policies as presets over a standard policy (experimental)

**Context.** The maintainer asked to manage S3 bucket permissions from the app. Most people want
one of a few things: keep a bucket private, publish it, or publish one folder (a website's assets,
shared photos). Bucket policies are the S3-standard way and work on AWS, MinIO, Wasabi, Ceph and
SeaweedFS. ACLs are disabled by default on new AWS buckets, and each provider's IAM/user policies
differ, so neither is used.

**Decision.** `policy.ts` offers Private (no policy), Public read for the whole bucket, Public read
for listed folders (one `s3:GetObject` statement for everyone, Sid `NoExplorerPublicRead`), and
Custom (raw JSON). An existing policy opens as the preset it matches, otherwise as Custom, so a
policy written elsewhere is never rewritten silently. The backend only fetches, validates as JSON,
stores and deletes (`GetBucketPolicy` / `PutBucketPolicy` / `DeleteBucketPolicy` on the bucket's
regional client); the server's own rules (AWS Block Public Access, providers without policies)
come back as its errors. A "Copy public link" builds the unsigned address of a file.

**Consequences.** Presets never grant listing (`s3:ListBucket`), so public buckets can't be
enumerated unless a custom policy says so. Checked against SeaweedFS (`s3_bucket_policy`: an
unsigned GET succeeds in the public folder only, and fails again once the policy is removed). R2,
GCS and Oracle manage public access in their consoles and will show the server's error.

## ADR-048: Remembered folder sizes; automatic remote sizes are capped and experimental

**Context.** Opening the app or a folder the index didn't cover (after a restart, or once its root
was evicted) showed "Scanning…" and empty sizes until the scan finished. Remote sizes came only
from Σ and were forgotten as soon as the folder was left. Counting remote folders automatically
is risky: one bucket folder can hold millions of objects.

**Decision.** Local: `sizes.rs` keeps the last fresh totals of folders the pane has shown
(`Seen`, at most 20,000, in `<OS cache dir>/folder-sizes.json`, written at most every 10 s and
when a scan ends). `folder_sizes` answers from the live index first and from `Seen` otherwise,
marked `stale`; the scan then sends fresh numbers. Only what the pane showed is kept, not the
whole index. Remote: `remotesizes.ts` remembers what Σ (or automatic sizes) counted, per device in
localStorage (3,000 folders), and fills listings from it, dimmed with the age. The experiment
`remote_sizes` counts a listing's folders in the background, 2 at a time, only on S3 and Azure
(flat listings: one request per 1,000 objects), at most 200,000 objects per folder
(`remote_size` `cap`; `partial` = "≥"), and recounts after 15 minutes. Walking providers
(WebDAV, SFTP, drives) cost a request per folder and stay Σ-only.

**Consequences.** Remembered sizes can be out of date until refreshed, so they're always dimmed
and say so. A capped folder shows a lower bound. The folder total is the sum of its folders and
files once every folder is known, so it needs no extra request.

## ADR-049: Cloud-synced folders are recognized and left out of size scans

**Context.** Automatic size scans went into folders kept by cloud sync clients. On macOS most are
File Provider folders (Google Drive, OneDrive, Dropbox, Box, iCloud Drive): listing one can fetch
it from the server, and access is a separate permission that can be revoked, after which opening
the folder bounced back to the parent with no explanation. Classic clients (Nextcloud, Synology
Drive, MEGA) keep real files, but they're a copy of a cloud, not this disk's own data.

**Decision.** `cloud.rs` recognizes them for free during a scan: by location
(`~/Library/CloudStorage/*`, `~/Library/Mobile Documents`, Windows OneDrive/iCloudDrive) and by
the marker file a sync client keeps in its folder's top level (Nextcloud `.sync_*.db`, `.dropbox`,
OneDrive's GUID file, `.SynologyWorkingDirectory`, Google Drive's `.tmp.drive*`, MEGA `.debris`),
which the scanner reads with the listing anyway. Scans don't enter them and report them
(`DirSize::cloud`), so the row shows ☁ and the service. Inside one, no automatic scan runs; Σ
counts it. CloudStorage and Mobile Documents left the "protected" list: they're never scanned
automatically, whatever the user opened. A local folder the system refuses now gets an
explanation (privacy settings, cloud access) before the pane goes back to the parent.

**Consequences.** Totals of folders containing a cloud folder are lower bounds (counted as
skipped, as before for protected folders). Cloud folders aren't added to Places: the maintainer
wants cloud services as remote accounts (OneDrive, Nextcloud), and here only recognized.

**Update 2026-10-23.** Marker files are gone: they caught sync clients that keep real copies on
disk (the maintainer's Nextcloud folder), which should count like any folder, and missed what
matters. Only online-only folders are cloud now, found by where the OS puts them: macOS File
Provider domains in `~/Library/CloudStorage` (OneDrive, Google Drive, Dropbox, Box, Nextcloud with
virtual files), iCloud Drive, Drive File Stream's `/Volumes/GoogleDrive`, and on Windows the
OneDrive / iCloudDrive folders and Google Drive's virtual drive (a drive with `My Drive` on top).
The Places menu lists them in a Cloud group (`local_roots` adds them before disks, so a Google
Drive volume isn't also a plain disk), at the maintainer's request.

## ADR-050: A speed test that moves data the way transfers do (experimental)

**Context.** The maintainer wants users to see which service (and which route: this computer ↔
remote, remote ↔ remote) is fastest for them. Generic speed tests measure the internet line, not
an S3 multipart upload, a Nextcloud chunked upload or a OneDrive session, nor the per-file cost
that dominates folders of small files.

**Decision.** `speedtest.rs` runs the real provider code (`upload_file` / `download_file`,
`copy_within` or a relay for remote → remote) with each provider's file concurrency, on random
(incompressible) data: one large file and many small files, plus the median of three listings as
response time. A probe job is cancellable but unthrottled and outside the activity card. Test
data lives in the OS temp folder and `<folder>/.noexplorer-speedtest-<id>/` on each remote, and
is deleted after every run, including failures and Cancel. Results stream as `"speedtest"`
events; the dialog (`speedtest.ts`) keeps the last 20 runs per device and stars the fastest
large-file route.

**Consequences.** Needs write access where it runs (the open view's folder, else the bucket's
top). A remote → remote relay counts what the user would wait for, not the raw line between the
two servers. Writing it surfaced a real Nextcloud bug: parallel uploads into a new folder raced
on MKCOL (423 / 403); `Dav::ensure_parents` now creates one folder at a time per account and
retries a 423.

**Update 2026-10-21.** The first version picked an account's first bucket by itself and could
leave test data behind. Each end is now an explicit account + bucket + optional folder (an open
view's folder by default); the dialog says where test data goes. Cleanup is checked (`remove`
lists the test folder again, retries what's left, and reports failures as notes), and `sweep`
removes `.noexplorer-speedtest-*` folders an interrupted run left in the same place. One cause
was a general bug: S3's one-by-one delete fallback (used on GCS, which has no bulk delete) stopped
at the folder's missing marker, because GCS answers 404 for a missing object. Results moved to a
History tab: one line per run, expandable to the numbers, notes and Run again.
