# Roadmap & known limitations

Ordered by priority. Each item says **where** it would be implemented, so whoever picks it up
can start right away. When you finish an item, move it to `CHANGELOG.md` and update the
**Status** block in `AGENTS.md`.

## Next (v0.1.x → v0.2)

1. **Release v0.2.0** with the Unreleased features. Check that the new .dmg notarization step in `release.yml` passes.
2. **Real screenshots and a GIF on the website.** `site/index.html` currently draws an HTML
   mock-up of the app in the hero.
3. **Smoke-test on Windows and Linux.** CI compiles both but nobody has used them yet. Pay
   attention to drag positions on HiDPI Windows (`dnd.ts`, `devicePixelRatio`) and to Places
   on Linux (`local.rs::local_roots`).

## Later

- **Friendlier S3 errors**: map `SignatureDoesNotMatch` → "Wrong secret key", `InvalidAccessKeyId`,
  `AccessDenied`, `NoSuchBucket`… using the SDK's `ProvideErrorMetadata` (`util::err` / a new `sdk_err`).
- **Copy between two accounts / two remote panes** (needs a client per pane, see ADR-014).

- **Keychain storage for secrets** with the `keyring` crate (supersedes ADR-008), in `config.rs`.
- **Windows code signing** via SignPath (OSS), in `release.yml`.
- **Drag out to the OS desktop/Finder.** Needs a native drag-start (for example the
  `tauri-plugin-drag` crate) that first downloads to temp (`dnd.ts` + a new command).
- **Parallel ranged downloads** for big files, mirroring multipart upload (`transfer.rs::download_file`).
- **Sync / overwrite mode**: "skip if same size+mtime / overwrite / rename" chooser
  (`transfer.rs`, new ADR superseding parts of ADR-006).
- **Rename / move / copy within S3** (CopyObject + Delete), in `remote.rs` and the context menu.
- **List virtualization** for folders with more than 10k entries (`pane.ts::render`).
- **Pagination UX:** `list` currently loads every page of a folder at once (`remote.rs::list`).
- **Frontend tests** (Vitest + happy-dom) for `Pane` sorting/selection and `files.ts`.
- **Tighter CSP** and asset-protocol scope (`tauri.conf.json`). See the security model in ARCHITECTURE.
- **i18n** (Vietnamese first). See ADR-011.
- **Self-signed TLS** support: an opt-in "trust this certificate" setting (custom
  `rustls` config in `config.rs::make_client`).
- **Bandwidth limit / pause-resume** for jobs (`jobs.rs`, `transfer.rs`).

## Known limitations (also in README)

- You can't drag from the app out to Finder/Explorer. Use the left pane instead.
- Self-signed HTTPS certificates are rejected.
- The app never deletes local files (on purpose, ADR-006).
- Re-uploading always re-sends the bytes; there is no dedup or sync logic.
