---
title: Experimental features
description: Features you can try in Settings → Experimental. They're off until you turn them on.
sidebar:
  order: 5
---

New features arrive in **Settings → Experimental** first. They're off until you turn them on, and may still change.

## Backup pairs

Save “this folder ↔ that remote folder” once, then back it up in one click from the **Backups** button at the top right (or <kbd>⌘ K</kbd>). It compares the two folders and uploads only what's missing or changed.

## Speed test

Compare how fast your storage services really are, the way NoExplorer moves files. Open it with <kbd>⌘ K</kbd> → **Speed test…** or the right-click menu of a remote view, pick **From** and **To**: this computer or an account, then the bucket and (optionally) the folder to test in. Choose a size and **Run**:

- **One large file** shows throughput (multipart and chunked uploads, as in real transfers).
- **Many small files** shows the cost of each file (files per second): what a folder of photos or documents feels like.
- **Response time** is how long one listing takes on each remote.

**Both directions** measures the way back too. Between two remotes the data goes the way a copy between them goes: on the server within one account when the provider can, otherwise through this computer. Test data is random, goes in a temporary `.noexplorer-speedtest-…` folder in the place you picked, and is deleted afterwards (also when you cancel); the deletion is checked, and test folders an interrupted run left behind are removed before the next run there. **History** keeps your runs on this computer: one line each with the speeds, ★ for the fastest so far, and a ✓ or ! for the cleanup. Click one for every number, what was cleaned up, and **Run again**.

## Bucket access policies

Decide who can read an S3 bucket's files **without a key**. Open it from the bucket menu (**Access policy for “…”**), the right-click menu or <kbd>⌘ K</kbd>:

- **Private**: no policy. Only access keys can read or write.
- **Public read: whole bucket**: anyone with a file's address can download it.
- **Public read: some folders**: only files in the folders you list (one per line) are public. Right-click a folder to suggest it.
- **Custom policy**: edit the policy JSON yourself, for listing, uploads, conditions or other users.

Nobody can list a public bucket's files unless a custom policy allows it. **Copy public link** in the right-click menu copies a file's plain address, the one that opens once the file is public; unlike **Copy 7-day link**, it never expires.

Your access key needs permission to read and change bucket policies. Works with AWS S3, MinIO, Wasabi, DigitalOcean Spaces, Ceph, SeaweedFS and other servers that support S3 bucket policies. On AWS, **Block Public Access** (on by default for new buckets) refuses public policies: turn it off for the bucket in the AWS console first. Cloudflare R2, Google Cloud Storage and Oracle Cloud manage public access in their own consoles instead.

## iPhone and iPad

Plug in an iPhone or iPad and it shows up by itself in the account menu (the account name at the top of a remote view), under **Devices**. There's no account to add. Unplugging it takes it out of the list again, unless NoExplorer has a backup of it (below): then it stays there as **Not connected**, and you can still read that backup. Pick it, and it opens like a drive, with these folders at the top:

- **Media**: the phone's media area, the one Image Capture and Windows see. Photos and videos are in `DCIM`, along with `Downloads`, `Books` and others. You can copy files out, preview them and delete them. Files copied *into* it don't appear in the Photos app (only importing does that); use an app's folder for files you want to open on the phone.
- **Apps**: a folder per app that shares files, such as VLC or Documents: the files you see under Files → On My iPhone → that app, or in Finder's **Files** tab. Copy files in and out, make folders, rename and delete.

Over the cable iOS only shares these two areas. For the files of **any** app, back the phone up (below).

### Apps' files from a backup

Most apps (WhatsApp, Notes…) don't share their files. Their data is in the phone's backup, so NoExplorer can make one: right-click at the top of the iPhone and choose **Back up iPhone to read app files…** (or <kbd>⌘ K</kbd> → **Back up iPhone**). It's the same full backup Finder or Apple Devices makes, kept in NoExplorer's own folder.

Afterwards a **Backup** folder appears next to Media and Apps, with a folder per app (its shared app groups and extensions inside) and **Other** for the rest. These are read-only copies from the time of the backup: copy files out and preview them, even with the phone unplugged. Backing up again (right-click at the top or in Backup) brings them up to date: only what changed is copied. **Delete backup…** moves it to the Trash.

- The first backup can take a while and needs about as much free space as the data on the phone. Keep the phone connected and unlocked; if it asks for its passcode, enter it.
- **Encrypted backups can't be read yet.** If “Encrypt local backup” is on for this phone in Finder or Apple Devices, NoExplorer says so before starting; turning it off there asks for the backup password.
- If the backup stops (the cable comes out, the phone stops answering for 10 minutes, this computer's disk gets down to its last GB, or you cancel), it says why. iOS only keeps a backup once it finishes, so backing up again **starts over** (a later backup after a finished one only copies what changed). Until then the previous finished backup stays readable; a first backup that never finished takes space but can't be read: delete it from the right-click menu.
- **“Multiple errors uploading files” (104)**: the iPhone couldn't read some of its files. NoExplorer asks the iPhone to open its protected files for the backup, as Finder does; if it still happens, keep the iPhone unlocked on its Home Screen, close busy apps, restart it if needed, and back up again.
- Files that apps keep out of backups (caches, some downloads) aren't there.

**The first time**, unlock the phone and tap **Trust** when it asks about this computer. NoExplorer uses the trust your system already keeps: on macOS it's built in; on Windows, install **Apple Devices** from the Microsoft Store (or iTunes) and, if the phone doesn't ask, open it once with the phone connected; on Linux, install `usbmuxd`. Unplugging the phone stops what was going on within a few seconds and shows **Try again** in its view; a download continues where it stopped once the phone is back.

Not yet tried on a real iPhone: please report what works and what doesn't.

## Self-signed HTTPS

Lets an account trust its server's own certificate, after you compare its fingerprint. See [Connect your storage](/docs/accounts/#self-signed-https-experimental).
