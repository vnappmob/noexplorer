---
title: Experimental features
description: Features you can try in Settings → Experimental. They're off until you turn them on.
sidebar:
  order: 5
---

New features arrive in **Settings → Experimental** first. They're off until you turn them on, and may still change.

## Backup pairs

Save “this folder ↔ that remote folder” once, then back it up in one click from the **Backups** button at the top right (or <kbd>⌘ K</kbd>). It compares the two folders and uploads only what's missing or changed.

## Create buckets

Adds **New bucket…** to the bucket menu and <kbd>⌘ K</kbd> for S3-compatible storage (AWS, MinIO, R2, B2, Wasabi, Google Cloud Storage…) and **New container…** for Azure. Names are 3 to 63 lowercase letters, digits and hyphens (S3 also allows dots). It works when your key is allowed to create buckets; otherwise the server's answer is shown. On AWS the bucket goes in the account's region; on Google Cloud Storage, in the account's project.

## More storage types

Google Cloud Storage and SFTP in the account form. See [Connect your storage](/docs/accounts/#more-storage-types-experimental).

## Self-signed HTTPS

Lets an account trust its server's own certificate, after you compare its fingerprint. See [Connect your storage](/docs/accounts/#self-signed-https).
