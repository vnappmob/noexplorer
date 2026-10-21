---
title: Experimental features
description: Features you can try in Settings → Experimental. They're off until you turn them on.
sidebar:
  order: 5
---

New features arrive in **Settings → Experimental** first. They're off until you turn them on, and may still change.

## Backup pairs

Save “this folder ↔ that remote folder” once, then back it up in one click from the **Backups** button at the top right (or <kbd>⌘ K</kbd>). It compares the two folders and uploads only what's missing or changed.

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
