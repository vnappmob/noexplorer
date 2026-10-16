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

## Tier 1: daily-driver parity (next)

**Done in 0.4.1:** live folder sizes (ADR-022) · Move to Trash (ADR-023) · live folder view ·
virtualized lists (ADR-021) · tray account switcher.

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

- More backends behind the same pane: SFTP, WebDAV, Google Cloud Storage (XML API), Azure Blob.
- **Windows code signing** via SignPath (OSS), in `release.yml`.
- **i18n** (Vietnamese first, ADR-011) and full keyboard/screen-reader accessibility.
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
