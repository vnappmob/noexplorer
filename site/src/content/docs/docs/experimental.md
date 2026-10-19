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

## Back up a level, as you left it

<kbd>⌘ [</kbd>, the ← button in the path bar, or your mouse's back button go up one level, and the folder comes back scrolled and with the same files selected as when you left it. With this on, ↑, <kbd>⌘ ↑</kbd> and <kbd>Backspace</kbd> do the same.

## Folder tree

The ↑ button in the path bar opens a tree of the folders you've opened in that pane: the path you're on, the folders next to it, and a ▸ on every folder you've been into. Click one to jump there, or type in the box at the top to find any folder you've opened. <kbd>⌘ ↑</kbd> and <kbd>Backspace</kbd> still go up one level. The tree only knows folders you've opened since NoExplorer started.

## Box accounts

Box in the account form, with your own Box app. It works against our test double but hasn't been tried on a real Box account yet. See [Connect your storage](/docs/accounts/#box-experimental).

## Self-signed HTTPS

Lets an account trust its server's own certificate, after you compare its fingerprint. See [Connect your storage](/docs/accounts/#self-signed-https-experimental).
