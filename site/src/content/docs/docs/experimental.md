---
title: Experimental features
description: Features you can try in Settings → Experimental. They're off until you turn them on.
sidebar:
  order: 5
---

New features arrive in **Settings → Experimental** first. They're off until you turn them on, and may still change.

## Rename, move and duplicate

<kbd>F2</kbd> renames the selected file or folder. The right-click menu adds **Move to…** and **Duplicate**, and dragging onto a folder in the same pane moves there. <kbd>⌘ Z</kbd> undoes the last one. It works on this computer and on remotes, and never replaces a file that's already there.

## Resumable transfers

Uploads and downloads cut short by a network drop, quitting or a crash can be resumed: the next time you open NoExplorer, the transfer card offers **Resume**. Files already transferred are skipped, unfinished downloads continue where they stopped, and big S3 uploads continue from the parts already on the server. Failed files retry on their own, running transfers have a pause button, and you can set a speed limit for everything.

## Backup pairs

Save “this folder ↔ that remote folder” once, then back it up in one click from the **Backups** button at the top right (or <kbd>⌘ K</kbd>). It compares the two folders and uploads only what's missing or changed.

## Thumbnail grid

A list / grid button in each pane header. Images and videos show thumbnails, on this computer and remote. Arrow keys move around the grid.

## Tabs, history and favorites

Back and forward with <kbd>⌘ [</kbd> <kbd>⌘ ]</kbd> (or your mouse's side buttons), tabs in each pane with <kbd>⌘ T</kbd> and <kbd>Ctrl Tab</kbd>, and a star in the path bar to save favorite folders. Favorites show up in the places menu, the account menu and <kbd>⌘ K</kbd>.

## More storage types

Google Cloud Storage, Azure Blob Storage and SFTP in the account form. See [Connect your storage](/docs/accounts/#more-storage-types-experimental).

## Self-signed HTTPS

Lets an account trust its server's own certificate, after you compare its fingerprint. See [Connect your storage](/docs/accounts/#self-signed-https).
