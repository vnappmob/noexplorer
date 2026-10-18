---
title: Code signing
description: How downloads and updates are signed, and installing on Windows.
sidebar:
  order: 9
---

How NoExplorer's downloads and updates are signed, and what that means when you install it.

| Platform | Signature |
| --- | --- |
| macOS | Signed with an Apple Developer ID and notarized by Apple. It opens without warnings. |
| Windows | **Not signed.** A Windows code signing certificate costs money every year, and NoExplorer is free. We'll buy one when the project can afford it. |
| Linux | The `.AppImage`, `.deb` and `.rpm` files aren't signed, like most apps outside the distro repositories. |
| Updates | Every update, on every platform, is signed with NoExplorer's own update key. An installed app refuses any update that isn't. |

## Installing on Windows

Because the installer isn't signed, Windows SmartScreen says “Windows protected your PC” the first time. Click **More info → Run anyway**. After that, updates install from inside the app without the warning.

Only download NoExplorer from this website or the [GitHub releases page](https://github.com/vnappmob/noexplorer/releases). Every installer is built by GitHub's own servers straight from the source code of a tagged release ([release.yml](https://github.com/vnappmob/noexplorer/blob/main/.github/workflows/release.yml)), so you can check what went into it.

## Privacy

NoExplorer doesn't send information anywhere you didn't ask for, except the update check, which you can turn off. Details: [Privacy](/docs/privacy/).
