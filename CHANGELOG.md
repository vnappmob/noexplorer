# Changelog

All notable user-visible changes. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versions: [SemVer](https://semver.org/).

## [Unreleased]

### Fixed
- Uploads and downloads in 0.4.4 showed no progress, never refreshed the pane, and after a few
  transfers stopped working: every transfer's progress report deadlocked a background thread.
- A single remote view could be shorter than the left pane after splitting, resizing and closing
  views.

## [0.4.4] - 2026-10-16

### Added
- **Split remote views.** The right side can show up to 3 accounts stacked top and bottom, each
  with its own account, bucket and folder. Use the split button, ⌘\ or the palette; ⌘W or ✕
  closes a view. Drag the handle between views to resize them; double-click it to even them
  out. The layout is remembered.
- **Copy between accounts.** Drag files from one remote view to another (S3 → Nextcloud, Box →
  S3, …). Within one S3 account the copy happens on the server (files up to 5 GB); otherwise
  each file is relayed through a temp file and deleted after. Progress shows in the activity card.

### Changed
- Tab cycles through the local pane and every remote view. F5 from the local pane uploads to
  the remote view you used last (it has an accent bar when several are open).
- Narrow pane headers hide secondary buttons and columns instead of squashing the chips.

## [0.4.3] - 2026-10-16

### Added
- **Compare folders** (⌘K → Compare, or right-click → Compare with remote…): after a backup, see what's missing on either side and what differs. **Verify** checks contents by checksum without downloading anything (S3 ETags, including multipart uploads; Box SHA-1; Nextcloud checksums). Count gaps are explained (system junk like `.DS_Store` that uploads skip on purpose, symlinks), and one click uploads or downloads the difference.
- **Box (beta)**, with your own Box app: paste its Client ID and secret and sign in through the browser. Browse, preview, upload (big files in sessions, re-uploads become new versions), download, delete (to the Box trash), Copy link. See README → Connect for the 2-minute app setup.
- **Nextcloud and WebDAV accounts.** Pick *Nextcloud* in Settings → Accounts and click **Sign in with Nextcloud…**: you approve in the browser, and NoExplorer gets its own app password (kept in your system keychain). Or use *WebDAV* for any WebDAV server or NAS. Browse, preview (images, PDF, video with seeking), upload (big files in chunks), download, create folders, delete, and Copy link (a public share that expires, on Nextcloud).

### Changed
- Narrow panes keep file names readable: the Modified column hides and account details shorten.
- Menus and prompts name the account ("Upload to Home NAS", "Delete from R2") instead of saying "S3"; the tray lists "Accounts".

## [0.4.2] - 2026-10-16

### Changed
- A **storage bar** at the bottom of the local pane shows the disk like macOS System Settings → Storage: the current folder, other used space and what's available (the same "available" figure macOS shows, purgeable space included). It replaces the bar under each folder size.
- Sizes no longer get a "≥" prefix when some folders inside were skipped; hover for the explanation.

## [0.4.1] - 2026-10-16

### Added
- **Folder sizes as you browse** (this computer). Every folder gets its size and an ncdu-style bar, filled in while a background scan runs, and remembered so going back is instant. Sizes follow changes on disk, and the folder list itself updates live when files are added or removed elsewhere. The scanner is our own: about 1.5× faster than `ncdu -t 8` and 6× faster than `du` in our tests. It stays off other disks and doesn't open private macOS folders (Documents, Desktop, Downloads, Library) until you've opened them yourself, so no surprise permission prompts; those show "—", and Σ scans everything. Turn it off in Settings → General.
- **Move to Trash** for files and folders on this computer (⌘⌫ / Delete, right-click, ⌘K). Uses the real Trash / Recycle Bin, so you can put things back. NoExplorer still never deletes local files permanently.
- **Accounts in the tray / menu bar.** The tray menu lists your S3 accounts; choosing one opens NoExplorer on that account.

### Changed
- Σ on the local pane scans the whole folder and fills every subfolder's size at once.

### Fixed
- The VNAppMob link in Settings → About (and on the website) now opens github.com/vnappmob instead of a domain that doesn't resolve.

## [0.4.0] - 2026-10-16

### Security
- Secret keys are now kept in your system's credential store (macOS Keychain, Windows Credential Manager, Secret Service on Linux) instead of plain text in `config.json`. Existing accounts move over automatically on first launch. If no credential store is available, the old 0600 file is used as before.
- The secret key is never sent to the app's UI. In Settings the field stays empty; leave it empty to keep the saved key, or type a new one.

### Fixed
- Resizing the panes is smooth again in big folders. Lists now render only the rows on screen, so dragging the splitter, scrolling and refreshing stay fast with thousands of files.

## [0.3.2] - 2026-10-16

### Changed
- The computer and cloud watermarks are now a fixed background at the bottom of each pane: file rows scroll over them, so they stay visible in long folders.

## [0.3.1] - 2026-10-16

### Changed
- Removed the floating selection bar. It got in the way and was easy to click by accident; every action is in the right-click menu (and F5, Space, ⌘K). The column header now shows how many items are selected and their size.
- The computer and cloud watermarks now sit below the last row, so they stay visible (scroll to the end) in long folders instead of disappearing.

## [0.3.0] - 2026-10-15

### Changed
- **New look.** Each pane has a two-row header: **account › bucket** dropdowns (or **This computer** with your places) above a separate **path bar**. Click a folder in the path to go there, or click empty space (⌘L) to type a path. Bundled Geist fonts, stroke icons, refreshed light and dark themes.
- Actions for the selection now appear in a **floating bar** at the bottom of the pane (Upload / Download, Quick look, Open, Link, Delete, ⋯).
- Transfers show in a compact **activity card** (click it for the full list), and an **animated beam** links the source to the destination while files move.
- Faint watermarks (a computer, clouds) tell the local and remote panes apart at a glance.
- macOS: the search bar sits in the title bar next to the window buttons.

### Added
- **⌘K command palette**: search commands, accounts, buckets, places and the files in both panes.
- The bucket menu is searchable and shows each bucket's region (AWS).

## [0.2.3] - 2026-10-15

### Fixed
- macOS: the login item could still be listed under the developer's name after 0.2.2, because macOS only decides that when the item is first registered. NoExplorer now re-registers an old entry once on launch (about 12 s after start), so it shows up as **NoExplorer**.

## [0.2.2] - 2026-10-15

### Fixed
- macOS: with "Launch at login" on, System Settings → Login Items showed the background item under the developer's name instead of **NoExplorer**. Existing entries are repaired automatically on the next launch.

## [0.2.1] - 2026-10-15

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
