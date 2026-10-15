# Changelog

All notable user-visible changes. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versions: [SemVer](https://semver.org/).

## [Unreleased]

### Added
- Provider presets in Settings → Accounts (Amazon S3, Cloudflare R2, Backblaze B2, Wasabi, DigitalOcean Spaces, self-hosted) with setup hints.

### Fixed
- **Amazon S3**: `s3.amazonaws.com` with a non-us-east-1 region failed with "AuthorizationHeaderMalformed". The endpoint can now be left empty, the region is read from regional endpoints, and each bucket is used in its own region automatically (buckets in different regions work side by side).
- S3 errors are now short and readable, with a hint (wrong region, wrong secret key, access denied, unreachable server…).
- Long account names/endpoints no longer overflow the account list.
- The path bar showed `/` in the wrong place (e.g. `Users/krisluu/`).

## [0.2.0] - 2026-10-15

### Added
- Multiple S3 accounts: account picker in the S3 pane header, and an account manager in Settings (add, edit, remove, test).
- Σ Size: total size of the current folder (shown in the new path bar) or of the selected folders (shown in the Size column), local and remote.
- Path bar under each pane with the full current location (`/abs/path` or `s3://bucket/prefix`); click to copy.
- Settings → Appearance: theme (System / Light / Dark) and text size (Small / Default / Large / Extra large).
- Drag the divider between the panes to resize them; double-click to reset.
- **Auto-update**: NoExplorer checks for new versions (at startup and every few hours) and installs them in place with one click, so there's no reinstalling. Updates are signed and verified.
- **Runs in the background**: closing the window keeps NoExplorer in the menu bar / system tray, and transfers continue. The tray menu shows live transfer progress (and on macOS the overall % appears in the menu bar).
- Notifications when transfers finish while the app isn't in front.
- Settings → General: keep running in the background, launch at login, notifications, automatic update checks.
- Settings → About: version, credits, license and project links, plus "Check for updates".
- Opening NoExplorer again while it's running in the tray brings the existing window back.

### Changed
- Default text size is slightly larger (13 → 14 px).
- Existing single-connection settings are migrated automatically to the first account.
- The macOS `.dmg` itself is now notarized and stapled (v0.1.0 notarized only the app inside).

## [0.1.0] - 2026-10-15

### Added
- Dual-pane browser: local files (left) and an S3-compatible bucket (right).
- Drag & drop between panes to upload or download; drop on a folder or breadcrumb to target it.
- Drop files and folders from Finder / Explorer onto the S3 pane to upload.
- Fast transfers: 6 files in parallel, multipart upload for files over 16 MB with 4 parts in flight.
- Transfers panel with progress, speed, cancel, and "Show" for downloads.
- Quick look (Space) for images, video, audio, PDF and text, local and remote.
- Open with the default app, copy a 7-day presigned link, new folder, delete (S3).
- Settings with connection test; path-style addressing on by default (MinIO-friendly).
- Keyboard-driven workflow (Tab, arrows, Enter, Space, F5 / ⌘↵, Backspace, ⌘⌫).
- Light and dark mode.
