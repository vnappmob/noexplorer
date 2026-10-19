# Roadmap: toward a tier-1 file explorer

The goal: a two-pane explorer for **your computer and your object storage** that is as fast and
complete as the best native file managers (Finder, Directory Opus, Total Commander, ForkLift,
ncdu for disk usage), open source, and small. Every item says **where** it would be implemented,
so whoever picks it up can start right away. When you finish one, tick it, add it to
`CHANGELOG.md` and update the **Status** block in `AGENTS.md`.

Ground rules for everything below:
- **Fast first.** Measure before and after (see the scanner benchmark in AGENTS.md). Prefer our
  own small Rust code on hot paths over general-purpose crates; use OS APIs directly when they're
  faster (as `scan.rs` does with `getattrlistbulk`).
- **Never lose data** (ADR-006/023): no permanent local delete, no silent overwrite.
- **Polite by default**: background work at low priority, no surprise OS permission prompts.

## Now: cloud accounts and multi-account views (agreed 2026-10-16)

One patch release per step, released only when the maintainer approves.

0. ✅ **Provider abstraction + a client per account** (ADR-025). A `Provider` interface in Rust (list, stat, ranged
   read, streamed upload/download, mkdir, delete, rename/copy, share link, size, capability
   flags) with S3 as the first implementation. Every remote command takes the account (no single
   "active client" any more, which lifts ADR-014's limit). `Profile.kind` = `s3 | webdav | box`, and OAuth tokens live in the OS keychain like secret keys (ADR-020). Ships as a
   refactor with no UI change; the S3 round-trip test must pass unchanged.
1. ✅ **Nextcloud / WebDAV** (ADR-026). Later: a Nextcloud job in CI (slow to start), and
   Nextcloud `oc:size` for instant folder sizes. our own small WebDAV client (PROPFIND, GET with Range, PUT, MKCOL,
   MOVE, COPY, DELETE), Nextcloud chunked upload v2 for big files, and sign-in with **Login
   Flow v2** (browser approval, app password into the keychain). Also covers ownCloud, NAS WebDAV
   and any WebDAV server.
2. ✅ (beta) **Box** (ADR-027): needs a first run against real Box (no test account here; covered by
   an in-memory mock). Original plan — REST API with **bring-your-own OAuth app** (Custom App, user authentication). Users
   paste the Client ID and secret; sign-in is PKCE through the system browser with a loopback
   redirect. A path↔ID cache (Box is ID-based) and chunked upload over 50 MB.
3. ✅ **Split remote views** (ADR-029): the right column holds 1–3 views stacked top/bottom, each with its own
   account, bucket and folder (⌘\ split, ⌘W close, Tab cycles). Dragging between views copies
   server-side on the same S3 account (≤ 5 GB) and relays file by file through a temp file
   between different accounts or providers, with progress in the activity card. Later: stream
   without the temp file, and server-side copy within one Nextcloud/Box account.

Not planned: Google Drive (dropped by the maintainer on 2026-10-16), and exposing sync-client
folders (iCloud Drive, `~/Library/CloudStorage`) as Places. iCloud Drive has no public API for
third-party desktop apps.

**Also done:** Compare folders with checksums (ADR-028). Docs site (`/docs` on the website).

## Tier 1: daily-driver parity (next)

**Done in 0.4.1:** live folder sizes (ADR-022) · Move to Trash (ADR-023) · live folder view ·
virtualized lists (ADR-021) · tray account switcher.

**Standard since 2026-10-19 (ADR-038):** rename / move / duplicate with undo on both sides (parts of
1 and 2), favorites, resumable transfers with pause and a speed limit, thumbnail grid. Tabs and
back/forward history were tried and removed (not useful enough).
**Still experimental (Settings → Experimental, off by default, ADR-031):** backup pairs, self-signed
HTTPS (ADR-032), Google Cloud Storage / SFTP (ADR-033). Azure Blob became standard on 2026-10-19. (A native window look and a
Vietnamese UI shipped too and were removed at the maintainer's request.)
Next for them: real-world runs, then graduating each one out of Experimental.

1. **Local file operations**: rename (F2 / click-to-rename), duplicate, copy/move between local
   folders (drag within the left pane, ⌘C/⌘V), new file, with a conflict dialog
   (skip / keep both / replace-into-Trash) and **undo** for the last operation (rename, move, trash
   via Put Back). `local.rs` + a new `ops.rs` job type so big copies show in the activity card.
2. **Rename / move / copy within S3** (CopyObject + Delete, server-side; multipart copy for > 5 GB)
   in `remote.rs` and the context menu. Copy **between two accounts** needs a client per pane
   (ADR-014).
3. **Back / forward history and tabs** per pane (⌘[ ⌘], ⌘T), plus a favorites sidebar
   (`pane.ts`, `main.ts`; history in `Pane`, tabs as a list of `{cwd, scroll, selection}`).
4. **Fast recursive search** in the current folder: name / glob / size / date filters, results
   streamed into the list. Reuse `scan.rs` workers (`read_dir` already gives names and sizes); for
   S3, a prefix listing with a client-side filter.
5. **Disk-usage views** on top of the size index: "Largest files" in this folder (top-N heap kept
   during the scan), a treemap (canvas, no library), and "Reveal in Trash list".
6. **Friendlier S3 errors**: map `SignatureDoesNotMatch` → "Wrong secret key",
   `InvalidAccessKeyId`, `AccessDenied`, `NoSuchBucket`… via `ProvideErrorMetadata` (`util::err`).
7. **Verify on Windows and Linux**: OTA (NSIS/MSI, AppImage), tray accounts, Recycle Bin and
   freedesktop Trash, drag positions on HiDPI (`dnd.ts`), Places (`local.rs::local_roots`).

## Tier 2: power-user features

- **Sync / mirror** a local folder with a prefix (both directions, dry-run preview, "skip if same
  size + mtime / ETag"), resumable. New ADR superseding parts of ADR-006 for the opt-in overwrite.
- **Parallel ranged downloads** for big files, mirroring multipart upload
  (`transfer.rs::download_file`). **Pause / resume / bandwidth limit** (`jobs.rs`).
- **Thumbnails / grid view** with a background thumbnail cache (`files.ts`, a Rust decoder for
  local images; presigned GETs for remote).
- **Inspector pane**: metadata, S3 headers/tags/storage class, versions (and restoring one).
- **Persist the size index** between launches (compact binary file, validated by folder mtimes).
- **Drag out to Finder/Explorer** (native drag-start, downloads to temp first; `dnd.ts`).
- **Pagination UX** for huge S3 prefixes: stream pages into the virtualized list
  (`remote.rs::list`).
- **Linux live updates below a root** (fanotify where allowed, or a polling fallback), so the size
  index and live view match macOS/Windows.

## Tier 3: platform & polish

- More providers behind the same trait: SFTP, Dropbox, OneDrive, Google Cloud Storage (XML API), Azure Blob.
- **Windows code signing** when there's budget for a certificate (SignPath declined, ADR-030; see docs/RELEASING.md).
- Full keyboard/screen-reader accessibility. (No translations: the UI stays English, ADR-011.)
- **Frontend tests** (Vitest + happy-dom) for `Pane` sorting/selection/virtualization and `files.ts`.
- **Tighter CSP** and asset-protocol scope (`tauri.conf.json`), see ARCHITECTURE's security model.
- **Self-signed TLS**: an opt-in "trust this certificate" (`config.rs::make_client`).
- Real screenshots and a GIF on the website (`site/index.html` draws a mock-up).

## Known limitations (also in README)

- You can't drag from the app out to Finder/Explorer. Use the left pane instead.
- Self-signed HTTPS certificates are rejected.
- Local files are only ever moved to the Trash, never deleted permanently (ADR-006/023).
- Re-uploading always re-sends the bytes; there is no dedup or sync logic yet.
- Linux: folder sizes and the live view only follow changes at the top of a scanned folder.
