---
title: Moving files
description: Upload, download, copy between accounts, and follow progress.
sidebar:
  order: 3
---

## Upload and download

- Drag from the left pane to the right one to **upload**, and the other way to **download**.
- Drop on a folder row or a part of the path bar to put the files there. Anywhere else, they go to the folder you're in.
- Or select files and press <kbd>F5</kbd> (or <kbd>⌘ ↵</kbd>, <kbd>Ctrl ↵</kbd> on Windows and Linux).
- You can also drop files from Finder or Explorer straight onto the right pane.

Folders keep their structure. Six files move at a time, and big files go up in parallel parts.

## Progress

A card at the bottom of the window shows what's moving, the speed and what's left. Click it for the full list, or ✕ to cancel. When a transfer finishes, the pane showing that folder refreshes on its own. Deleting on a remote works the same way: the card counts the items deleted so far, and the rows stay dimmed until it's done.

Close the window and NoExplorer keeps going from the menu bar (macOS) or system tray (Windows, Linux). Quit from the tray menu. You can turn this off in **Settings → General**.

If you open other folders while files are moving, a strip above the panes keeps showing each transfer: the full path on each side, lined up with its pane, and the files flowing between them (one strip per transfer). Click a side to open that folder, or the middle to open both.

## Pause, resume and speed limit

Each running transfer has a pause button in the transfer card. Failed files retry on their own when the network drops. If NoExplorer is closed (or crashes) in the middle of a transfer, the card offers **Resume** the next time you open it: files already transferred are skipped, unfinished downloads continue where they stopped, and big S3 uploads continue from the parts already on the server. To cap the bandwidth, set a speed limit in **Settings → General**.

## Rename, move and duplicate

<kbd>F2</kbd> renames the selected file or folder. The right-click menu has **Move to…** and **Duplicate**, and dragging onto a folder in the same pane moves there. <kbd>⌘ Z</kbd> undoes the last one. It works on this computer and on remotes, and never replaces a file that's already there.

## Your local files are safe

- Downloads never overwrite: if a file exists, the new one becomes `name (1).ext`.
- An unfinished download never looks finished: it's written to a temporary file first.
- Removing a local file moves it to the Trash or Recycle Bin, where you can put it back.

## Several accounts at once

Click the split button at the top of the right pane (or press <kbd>⌘ \\</kbd>) to add a second view below it, with its own account and folder. You can have up to three. Drag the line between them to resize, or press <kbd>⌘ W</kbd> to close the one you're in.

Drag files from one view to another to **copy between accounts**, for example from S3 to Nextcloud. Inside one S3 account the server copies the files itself (up to 5 GB each). Otherwise each file passes through a temporary file on your computer, which is deleted afterwards.

From the left pane, <kbd>F5</kbd> sends files to the view you used last. It has a colored bar when several are open.

:::note
You can't drag files from NoExplorer out to Finder or Explorer yet. Drop them on the left pane instead.
:::
