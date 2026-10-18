---
title: Compare, sizes & more
description: Compare folders with checksums, folder sizes, quick look, share links and the command palette.
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

On your computer, folders show their size as you browse, scanned in the background and kept up to date. The bar at the bottom of the left pane shows the folder against your whole disk. For remote folders, select them and click **Σ Size**.

## Quick look

Press <kbd>Space</kbd> to preview images, video, audio, PDF and text files, on either side, without downloading them. <kbd>←</kbd> <kbd>→</kbd> go to the previous or next file.

## Share links

Right-click a remote file and choose **Copy link**. Anyone with the link can download the file for 7 days (S3), or until the share expires (Nextcloud, Box).

## Find anything

<kbd>⌘ K</kbd> opens one search box for commands, accounts, buckets, places and files in the current folders. <kbd>⌘ L</kbd> lets you type a path.
