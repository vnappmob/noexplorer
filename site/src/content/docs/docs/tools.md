---
title: Compare, sizes & more
description: Search, Compare folders with checksums, folder sizes, grid view, favorites, quick look, share links and the command palette.
sidebar:
  order: 4
---

## Compare folders

After a backup, check that everything arrived. Right-click a folder and choose **Compare**, or press <kbd>⌘ K</kbd> and type “compare”.

- Lists files missing on either side, and files whose size differs.
- **Verify contents** checks each file against the checksum the server already has, without downloading it.
- Explains odd counts: system files like `.DS_Store` and `Thumbs.db` are skipped on purpose, and shortcuts (symlinks) aren't followed.
- One click uploads or downloads whatever is missing.

## Folder sizes

On your computer, folders show their size as you browse, scanned in the background and kept up to date. The bar at the bottom of the left pane shows the folder against your whole disk. Sizes from earlier scans are remembered, even after you quit: a folder shows them at once, dimmed (and the total with **≈**), until the new scan confirms them.

Online-only cloud folders (iCloud Drive, OneDrive, Google Drive, Dropbox, Box, and Nextcloud with virtual files) are listed in their own **Cloud** group in the **This computer** menu, show a **☁** tag, and aren't counted: their files are placeholders that download when opened, and scanning can fetch them or ask for permission. Open one and press **Σ** to count it anyway. A sync folder that keeps real copies on your disk (Nextcloud or Synology without virtual files) is an ordinary folder and is counted. If a folder won't open, NoExplorer says why: usually macOS privacy settings (System Settings → Privacy & Security → Files & Folders) or a cloud folder whose access was turned off.

For remote folders, select them and click **Σ Size** (or click it with nothing selected for the current folder). Counting a remote folder lists every file in it, so it only happens when you ask. What you counted is remembered on this computer and shown dimmed next time, with its age when you hover. **Settings → Experimental → Remote folder sizes** counts the folders you open on S3 and Azure in the background, up to 200,000 files each (bigger folders show **≥** and a lower bound).

## Grid view

The list / grid button in each pane header switches to big tiles with thumbnails of images and videos, on this computer and remote. Arrow keys move around the grid.

Image thumbnails are made once and kept in your system's cache folder (at most 48 MB; the least recently seen go first), so scrolling back and reopening a folder is instant. Photos and videos you open in quick look aren't kept on disk; with the thumbnails, NoExplorer's cache stays under about 70 MB. Remote images over 25 MB keep their icon.

## Favorites

Click the star in the path bar to save the folder you're in. Favorites show up in the places menu (this computer), the account menu (remote) and <kbd>⌘ K</kbd>. Click the star again to remove it.

## Moving around

- **←** (or <kbd>⌘ [</kbd>, <kbd>⌘ ↑</kbd>, <kbd>Backspace</kbd>, your mouse's back button) goes up one level, and the folder comes back scrolled and with the same files selected as when you left it.
- **↑ (the tree icon)** opens a tree of the folders you've opened in that pane: the path you're on, the folders next to it, and a ▸ on every folder you've been into. Click one to jump there, or type in the box at the top to find any folder you've opened. The tree knows the folders you've opened since NoExplorer started.

## New buckets

The bucket menu (and <kbd>⌘ K</kbd>) has **New bucket…** for S3-compatible storage (AWS, MinIO, R2, B2, Wasabi, Google Cloud, Oracle Cloud…) and **New container…** for Azure. Names are 3 to 63 lowercase letters, digits and hyphens (S3 also allows dots). It works when your key is allowed to create buckets; otherwise the server's answer is shown. On AWS the bucket goes in the account's region; on Google Cloud Storage, in the account's project.

## Quick look

Press <kbd>Space</kbd> to preview images, video, audio, PDF and text files, on either side, without downloading them. <kbd>←</kbd> <kbd>→</kbd> go to the previous or next file.

## Share links

Right-click a remote file and choose **Copy link**. Anyone with the link can download the file for 7 days (S3), or until the share expires (Nextcloud, Box).

## Search

Click the magnifier (or press <kbd>⌘ F</kbd>) and type: the folder filters as you type. Press <kbd>↵</kbd> to **search this folder and every folder below it** instead, on this computer or on a remote.

- Results arrive as they're found, each with the folder it's in. On this computer, several folders are read at once: about 5× faster than `find` on a large system folder.
- All words must be in the name, in any case: `report 2026`. Use `*` and `?` as wildcards (`*.pdf`, `IMG_????.jpg`), or `ext:pdf`.
- Double-click a folder to open it, a file to open or preview it. Right-click → **Show in its folder**. Drag results to the other pane to copy them.
- **Stop** ends a long search; <kbd>Esc</kbd> or ✕ goes back to the folder.
- On this computer, other disks and private folders you haven't opened in NoExplorer (Desktop, Documents, Downloads…) are skipped, so a search never triggers a macOS permission prompt. Hidden files are included only when they're shown. A search stops at the first 10,000 matches.
- On a remote, NoExplorer lists everything below the folder first, so a very large bucket takes a moment.

## Find anything

<kbd>⌘ K</kbd> opens one search box for commands, accounts, buckets, places and files in the current folders. <kbd>⌘ L</kbd> lets you type a path.
