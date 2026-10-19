# Changelog

All notable user-visible changes. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versions: [SemVer](https://semver.org/).

## [Unreleased]

## [0.4.20] - 2026-10-19

### Changed
- **Azure Blob Storage** is a standard provider (no longer in Settings → Experimental), and only
  needs the storage account name and a key: the endpoint fills itself in.
- macOS: the **delete** key deletes the selection (Move to Trash on this computer, with a
  confirmation on remotes), like ⌘⌫. ⌘↑ goes up a folder.

### Fixed
- Google Cloud Storage: new **Project ID** field, so the bucket list works ("Project number must be
  specified"). The error explains what to enter when it's missing.

### Website
- "Fast where it counts" on the home page: folder sizes vs `du` and `ncdu`, and Nextcloud folder
  listings with and without session reuse (measured numbers).

## [0.4.19] - 2026-10-19

### Added
- Error messages include the server's own explanation when it gives one (Google Cloud Storage puts
  it in a separate field). Failed requests are also written to a log file, without passwords, keys
  or tokens: **Settings → About → Show log file**, to attach to a bug report.

## [0.4.18] - 2026-10-19

### Fixed
- Google Cloud Storage accounts failed with "Invalid argument": requests no longer carry the
  `x-id` query parameter that Google's XML API rejects.

## [0.4.17] - 2026-10-19

### Changed
- **Rename, move and duplicate**, **resumable transfers** (pause, retry, resume after quitting),
  the **grid view** and **favorites** are now standard features, no longer in Settings →
  Experimental. The speed limit moved to **Settings → General**.

### Removed
- Tabs and back/forward history (⌘T, Ctrl+Tab, ⌘[ ⌘]). Favorites stay: the star in the path bar.

### Website
- Livelier home page: icons on the main points and features, a "works with" row, and a
  screenshot preview when the link is shared.

## [0.4.16] - 2026-10-19

### Fixed
- Copying between accounts keeps each file's type (a PDF stays `application/pdf` instead of
  `application/octet-stream`), and S3 copies over 5 GB keep the source's Content-Type.

## [0.4.15] - 2026-10-19

### Changed
- Grid view: a selected tile shows only its light blue background, without the extra bold border.

## [0.4.14] - 2026-10-19

### Fixed
- NoExplorer's cache folder no longer grows past ~70 MB: photos and videos opened in quick look
  aren't kept on disk any more, leftovers are cleaned at launch, and grid thumbnails use at most 48 MB.

## [0.4.13] - 2026-10-18

### Changed
- Grid view: image thumbnails are made once and cached (16 MB in memory, 64 MB on disk), pictures
  stay put while you scroll, and the next screen loads ahead.

### Fixed
- The app could quit suddenly while scrolling the grid view (a preview request cancelled at the
  wrong moment).

## [0.4.12] - 2026-10-18

### Changed
- Grid view: file and folder icons are bigger, so they fill their tiles.

### Removed
- The experimental native window look (Windows 11 Mica, macOS translucency). The window is
  opaque again on every platform.

## [0.4.11] - 2026-10-18

### Fixed
- Settings → Experimental lists one feature per row again (the items ran together on shared lines).

### Removed
- The experimental Vietnamese interface. Turning it on froze the window; the app is English only.

## [0.4.10] - 2026-10-18

### Added
- **Settings → Experimental**: new features ship here, off until you turn them on.
- Experimental **rename, move and duplicate**, on this computer and on remotes: F2 renames, the
  right-click menu has Move to… and Duplicate, dragging onto a folder in the same pane moves
  there, and ⌘Z undoes the last one. Nothing is ever overwritten; remote moves run on the
  server (S3 copies objects over 5 GB in parts).
- Experimental **resumable transfers**: failed files retry on their own (network trouble only),
  running transfers can be paused, an optional speed limit applies to everything, and transfers
  cut short by quitting or a crash can be resumed on the next launch. Resumed downloads continue
  their `.part` files, and big S3 uploads continue from the parts already on the server.
- Experimental **self-signed HTTPS**: an S3, Nextcloud or WebDAV account can trust its server's
  own certificate (a NAS, a home server) after you compare its fingerprint. Only that exact
  certificate is accepted; everything else is still checked normally.
- Experimental **backup pairs**: save "this folder ↔ that remote folder" and back it up in one
  click from the Backups button or ⌘K. It compares the two and uploads what's missing or changed.
- Experimental **thumbnail grid**: a list/grid button in each pane header. Images and videos
  show thumbnails, on this computer and remote; arrow keys move around the grid.
- Experimental **tabs, history and favorites**: back and forward (⌘[ ⌘], or the mouse's side
  buttons), tabs in each pane (⌘T, Ctrl+Tab, ⌘W), and a star in the path bar to save favorite
  folders, listed in the places menu, the account menu and ⌘K.
- Experimental **more storage types**: Google Cloud Storage (S3-compatible API with HMAC keys),
  Azure Blob Storage, and SFTP (through your `ssh` setup: keys, agent and ~/.ssh/config).
- Experimental **native window look** (Windows 11 Mica, macOS translucency) and Vietnamese
  interface (removed again in the next release).

### Website
- The docs moved to a faster docs site with search (Starlight): pages load as you point at a link.
- The home page lists the main features at the bottom, and has better search-engine metadata.

### Accessibility
- File lists are announced as lists with their selection, notifications are read out, and
  keyboard focus is always visible.

## [0.4.9] - 2026-10-17

### Changed
- Deleting on a remote is now a background task: the activity card shows how many items are
  deleted so far (and can stop it), and the rows being deleted stay dimmed until it's done.
- Transfer lanes are two boxes lined up with the two panes, each with the full path, and the
  transfer flowing between them.

### Fixed
- The transfer beam followed whatever rows were selected instead of the files being sent. It now
  starts at the files that were sent (or the list edge on their side when they're scrolled away),
  and hands over to a lane as soon as you leave the folder.

## [0.4.8] - 2026-10-17

### Added
- Transfer lanes: while files are moving and you open other folders (on either side), a strip
  above the panes keeps showing where each transfer goes, one strip per transfer. Click it to go
  back to those folders.

### Changed
- The transfer beam only connects what's on screen. Switch a view to another account mid-transfer
  and it fades out instead of pointing at unrelated files; its ends now glide instead of jumping.
- No more system notifications (and no permission prompt). Progress stays in the window and in
  the menu bar / tray.

## [0.4.7] - 2026-10-17

### Changed
- Switching accounts shows what's happening: the account button changes right away, the list says
  "Connecting to …" instead of showing the previous account's files, and a thin bar under the
  column header appears whenever a folder takes a moment to load.
- If a remote folder can't be listed, the error shows in the list with **Try again** (and **Go to
  top**) instead of a toast, and the view no longer jumps to the top folder on its own.
- Nextcloud is much snappier: NoExplorer reuses the server's session instead of logging in on
  every request (about 8× faster per folder listing on a test server).
- Switching back to an account you've used lists it straight away.

### Fixed
- Switching accounts quickly could show the wrong bucket or an error, when the previous account's
  answer arrived late.

## [0.4.6] - 2026-10-17

### Fixed
- New folder: pressing Enter in the name box cancelled instead of creating the folder. Only
  clicking OK worked.

## [0.4.5] - 2026-10-17

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
