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

## Box accounts

Box in the account form, with your own Box app. It works against our test double but hasn't been tried on a real Box account yet. See [Connect your storage](/docs/accounts/#box-experimental).

## Self-signed HTTPS

Lets an account trust its server's own certificate, after you compare its fingerprint. See [Connect your storage](/docs/accounts/#self-signed-https-experimental).
