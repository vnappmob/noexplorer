# Changelog

All notable user-visible changes. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versions: [SemVer](https://semver.org/).

## [Unreleased]

## [0.1.0] - unreleased

### Added
- Dual-pane browser: local files (left) and an S3-compatible bucket (right).
- Drag & drop between panes to upload or download; drop on a folder or breadcrumb to target it.
- Drop files and folders from Finder / Explorer onto the S3 pane to upload.
- Fast transfers: 6 files in parallel, multipart upload for files over 16 MB with 4 parts in flight.
- Transfers panel with progress, speed, cancel, and "Show" for downloads.
- Quick look (Space) for images, video, audio, PDF and text, local and remote.
- Open with the default app, copy a 7-day presigned link, new folder, delete (S3).
- Settings with connection test; path-style addressing on by default (MinIO-friendly).
- Keyboard-driven workflow (Tab, arrows, Enter, Space, F5 / ⌘↵, Backspace, ⌘⌫).
- Light and dark mode.
