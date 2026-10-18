---
title: Privacy
description: What NoExplorer sends, and where.
sidebar:
  order: 8
---

NoExplorer has no accounts, no analytics and no tracking. It talks only to the places below.

## What the app connects to

- **The storage you add** (your S3 server, Nextcloud, WebDAV server, Box, and the experimental Google Cloud Storage, Azure and SFTP). Only what you ask for: listing folders, moving files, previews, links.
- **GitHub**, to check for updates (at startup and every few hours) and to download them. This sends no personal data. Turn it off in **Settings → General → Check for updates automatically**.
- **Your browser**, when you sign in to Nextcloud or Box. You approve the sign-in on their site, and NoExplorer receives an app password or token.

Nothing else: no usage statistics, no crash reports, no information sent to VNAppMob.

## What stays on your computer

- Passwords, secret keys and tokens, in your system's credential store.
- Account settings, in the app's settings file (readable only by your user).
- Layout and look preferences, in the app's local storage.

## This website

The site is hosted on Cloudflare and uses no cookies or analytics. The download button asks GitHub's public API which release is the latest, straight from your browser.

## Questions

The code is open, so you can check all of this: [github.com/vnappmob/noexplorer](https://github.com/vnappmob/noexplorer). Questions go to [GitHub issues](https://github.com/vnappmob/noexplorer/issues).
