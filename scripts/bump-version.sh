#!/usr/bin/env bash
# Sets the app version everywhere it lives, then prints the release commands.
#   ./scripts/bump-version.sh 0.2.0
# Version sources: package.json (tauri.conf.json reads it via "version": "../package.json")
# and src-tauri/Cargo.toml (+ Cargo.lock).
set -euo pipefail
v=${1:?usage: $0 X.Y.Z}
[[ $v =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not semver: $v" >&2; exit 1; }
cd "$(dirname "$0")/.."
npm version "$v" --no-git-tag-version >/dev/null
# First `version = ...` line only (the [package] one). perl: same behavior on macOS and Linux.
perl -0pi -e "s/^version = \".*?\"/version = \"$v\"/m" src-tauri/Cargo.toml
(cd src-tauri && cargo update -p noexplorer --offline >/dev/null 2>&1 || cargo check -q)
echo "Version set to $v. Next:"
echo "  1. Move CHANGELOG.md 'Unreleased' entries under '## [$v] - $(date +%F)'"
echo "  2. git commit -am 'release: v$v' && git tag v$v && git push && git push origin v$v"
