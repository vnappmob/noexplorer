<p align="center">
  <img src="assets/icon.png" width="112" alt="NoExplorer icon" />
</p>

<h1 align="center">NoExplorer</h1>

<p align="center">
  <b>No explorer needed: just drag it to your bucket.</b><br/>
  A tiny dual-pane file manager for S3-compatible storage (MinIO, SeaweedFS, AWS S3, …).<br/>
  macOS · Windows · Linux
</p>

<p align="center">
  <a href="https://noexplorer.vnappmob.com">Website</a> ·
  <a href="https://github.com/vnappmob/noexplorer/releases/latest">Download</a> ·
  <a href="docs/DEVELOPMENT.md">Develop</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

```
┌──────── This computer ─────────┬──────── my-bucket ────────────┐
│ ~ › Projects › website         │ my-bucket › backups            │
│ 📁 src                         │ 📁 2026-10                     │
│ 📄 dump.sql   ───── drag ─────────▶  upload                     │
│               ◀──── drag ────────── 📄 site.tar.gz  download    │
└────────────────────────────────┴────────────────────────────────┘
```

## Why

Backing up to a self-hosted MinIO shouldn't take a heavyweight client and a trip into
"advanced settings" to enable path-style requests. With NoExplorer you open the app, drag,
and you're done.

## Features

- **Two panes**: your computer on the left, the bucket on the right. Drag either way to
  upload or download. Drop onto a folder or a breadcrumb to target it.
- **Drop from Finder / Explorer** onto the bucket pane. Folders keep their structure.
- **Fast**: 6 files in parallel; large files use multipart upload with 4 parts in flight.
- **Quick look** with Space: images, video, audio, PDF and text, local or remote.
- **Safe by design**: the app never deletes or overwrites local files. Downloads that would
  collide become `name (1).ext`, and partial downloads never look complete.
- **Self-hosted friendly**: path-style requests are on by default, and the connection test
  works even when the key can't list buckets.
- Copy a 7-day share link, create folders, delete from S3, follow progress (with cancel)
  for every transfer. Light and dark mode. About 10 MB on macOS.

## Install

Download the installer for your OS from the
[latest release](https://github.com/vnappmob/noexplorer/releases/latest) or the
[website](https://noexplorer.vnappmob.com).

| OS | File |
|---|---|
| macOS (Apple Silicon and Intel) | `NoExplorer_x.y.z_universal.dmg`, signed and notarized |
| Windows 10/11 | `NoExplorer_x.y.z_x64-setup.exe` or `.msi` |
| Linux | `.AppImage`, `.deb` or `.rpm` |

> Windows builds aren't code-signed yet. If SmartScreen appears, click **More info → Run anyway**.

## Connect

The first time you launch the app it opens **Settings**:

| Field | Example |
|---|---|
| Endpoint | `https://minio.example.com` or `http://192.168.1.10:9000` |
| Access key / Secret key | Your MinIO / S3 credentials |
| Region | `us-east-1` (fine for MinIO) |
| Path-style requests | ✓ Leave it on for MinIO and other self-hosted servers |

Click **Test**, then **Save**. Credentials are stored only on your computer, in the app's
config folder (readable only by your user).

## Keyboard

| Key | Action |
|---|---|
| `Tab` / `←` `→` | Switch pane |
| `↑` `↓` (+`Shift`) | Move or extend the selection |
| `Enter` | Open a folder. Remote file: preview. Local file: open in its default app |
| `Space` | Quick look (inside it, `←` `→` go to the previous or next file) |
| `F5` or `⌘/Ctrl + Enter` | Copy the selection to the other pane (upload or download) |
| `Backspace` / `⌘↑` | Up one folder |
| `⌘⌫` / `Delete` | Delete the selection from S3 |
| `⌘R` · `⌘F` · `⌘,` | Refresh · Filter · Settings |

## Limitations

- You can't drag files from the app out to Finder/Explorer yet. Drop them in the left pane instead.
- Self-signed HTTPS certificates aren't accepted. Use HTTP on a LAN or a trusted certificate.
- Local files are never deleted by the app, on purpose.

See the [roadmap](docs/ROADMAP.md) for what's next.

## Contributing

Pull requests are welcome. Start with [AGENTS.md](AGENTS.md), a one-minute tour of the
codebase, then [CONTRIBUTING.md](CONTRIBUTING.md).

```bash
npm install
./scripts/dev-s3.sh up && ./scripts/dev-s3.sh seed   # local S3 in Docker
npm run tauri dev
```

## License

[MIT](LICENSE) © 2026 Kris Luu (VNAppMob)
