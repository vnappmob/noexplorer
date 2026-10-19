---
title: FAQ & troubleshooting
description: Answers to common questions and problems.
sidebar:
  order: 7
---

### Windows says “Windows protected your PC”

Windows builds aren't code-signed (a certificate costs money every year; see [Code signing](/docs/code-signing/)). Click **More info → Run anyway**. Updates after that install without the warning.

### My file counts don't match after a backup

Open [Compare](/docs/tools/#compare-folders). It lists exactly what's missing or different. Usually it's system files like `.DS_Store`, which uploads skip on purpose.

### Can NoExplorer delete or overwrite my local files?

No. Downloads never overwrite (they become `name (1).ext`), and removing local files only moves them to the Trash or Recycle Bin.

### The connection test fails on my self-hosted server

- Keep **Path-style requests** on.
- Check the port in the endpoint, e.g. `:9000` for MinIO.
- A self-signed HTTPS certificate: turn on **Settings → Experimental → Self-signed HTTPS** and trust the certificate after comparing its fingerprint, or use plain HTTP inside your network, or a trusted certificate (for example from Let's Encrypt).

### Where are my keys and passwords stored?

In your system's credential store (macOS Keychain, Windows Credential Manager or Linux Secret Service). They're never shown in the app and only sent to the server you configured.

### Why does Box need my own app?

Box sign-in needs a client secret, and an open-source app can't keep one secret. A free Box app takes two minutes, see [Connect your storage](/docs/accounts/#box-beta).

### Closing the window doesn't quit the app

That's so transfers can finish. Quit from the menu bar or tray icon, or turn off **Keep running in the background** in **Settings → General**.

### Something else is wrong

Please [open an issue on GitHub](https://github.com/vnappmob/noexplorer/issues) with your system, the NoExplorer version (Settings → About) and what you did.

When a server refuses something, its full answer is written to a log: **Settings → About → Show log file** opens its folder. Attach `noexplorer.log` to the issue. It never contains passwords or secret keys, and signatures, key ids and tokens are blanked out. It lists the names of the files involved, so look it over first.
