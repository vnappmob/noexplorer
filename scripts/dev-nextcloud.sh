#!/usr/bin/env bash
# Local Nextcloud for developing and testing the WebDAV/Nextcloud provider (webdav.rs).
#
#   ./scripts/dev-nextcloud.sh up     # start on http://localhost:8080 (user: dev / NoExplorer-dev-2026!)
#   ./scripts/dev-nextcloud.sh seed   # a few folders and files under the dev user
#   ./scripts/dev-nextcloud.sh down   # stop and remove (data included)
#
# WebDAV root: http://localhost:8080/remote.php/dav/files/dev/  · SQLite, nothing persistent.
set -euo pipefail

NAME=noexplorer-nextcloud
PORT=${PORT:-8080}
USER_NAME=dev
PASS='NoExplorer-dev-2026!'
EP="http://localhost:$PORT"
DAV="$EP/remote.php/dav/files/$USER_NAME"

dav() { curl -fsS -u "$USER_NAME:$PASS" "$@"; }

case "${1:-up}" in
  up)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" -p "$PORT:80" \
      -e SQLITE_DATABASE=nextcloud -e NEXTCLOUD_ADMIN_USER="$USER_NAME" -e NEXTCLOUD_ADMIN_PASSWORD="$PASS" \
      -e NEXTCLOUD_TRUSTED_DOMAINS="localhost 127.0.0.1" nextcloud:31-apache >/dev/null
    printf 'Waiting for Nextcloud on %s' "$EP"
    for _ in $(seq 1 180); do
      if curl -fsS "$EP/status.php" 2>/dev/null | grep -q '"installed":true'; then echo " ready."; break; fi
      printf .; sleep 2
    done
    echo "Server: $EP   User: $USER_NAME   Password: $PASS"
    ;;
  seed)
    dav -X MKCOL "$DAV/Projects" >/dev/null || true
    dav -X MKCOL "$DAV/Projects/sub%20folder" >/dev/null || true
    printf 'hello from nextcloud\n' | dav -T - "$DAV/Projects/readme.txt" >/dev/null
    printf 'xin chào' | dav -T - "$DAV/Projects/sub%20folder/ti%E1%BA%BFng%20vi%E1%BB%87t.txt" >/dev/null
    echo "Seeded Projects/ under $USER_NAME."
    ;;
  down)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    echo "Removed $NAME."
    ;;
  *) echo "usage: $0 up|seed|down" >&2; exit 2 ;;
esac
