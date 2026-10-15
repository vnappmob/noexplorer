#!/usr/bin/env bash
# Local S3-compatible server for development and tests (SeaweedFS in Docker).
#
#   ./scripts/dev-s3.sh up     # start on http://localhost:8333 (key: devkey / secret: devsecret123)
#   ./scripts/dev-s3.sh seed   # create bucket "demo" with a few sample files
#   ./scripts/dev-s3.sh down   # stop and remove
#
# Why SeaweedFS and not MinIO: the official minio/minio Docker image is no longer published.
# Both speak the same S3 API with path-style addressing, which is what NoExplorer targets.
set -euo pipefail

NAME=noexplorer-s3
PORT=${PORT:-8333}
KEY=devkey
SECRET=devsecret123
DIR="$(cd "$(dirname "$0")" && pwd)"
CFG="$DIR/.dev-s3.json"
EP="http://localhost:$PORT"

s3() { # s3 <method> <path> [curl args...]
  local m=$1 p=$2; shift 2
  curl -fsS -X "$m" --aws-sigv4 "aws:amz:us-east-1:s3" --user "$KEY:$SECRET" "$@" "$EP/$p"
}

case "${1:-up}" in
  up)
    cat > "$CFG" <<JSON
{"identities":[{"name":"dev","credentials":[{"accessKey":"$KEY","secretKey":"$SECRET"}],"actions":["Admin","Read","Write","List","Tagging"]}]}
JSON
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" -p "$PORT:8333" -v "$CFG:/etc/s3.json:ro" \
      chrislusf/seaweedfs server -s3 -s3.config=/etc/s3.json -dir=/data >/dev/null
    printf 'Waiting for S3 on %s' "$EP"
    for _ in $(seq 1 60); do
      code=$(curl -s -o /dev/null -w '%{http_code}' "$EP/" || true)
      if [ "$code" = 403 ] || [ "$code" = 200 ]; then echo " ready."; break; fi
      printf .; sleep 1
    done
    echo "Endpoint: $EP   Access key: $KEY   Secret key: $SECRET   (path-style ON)"
    ;;
  seed)
    s3 PUT demo >/dev/null 2>&1 || true
    tmp=$(mktemp -d)
    printf '# Hello from NoExplorer\n\nThis bucket was seeded by scripts/dev-s3.sh.\n' > "$tmp/README.md"
    printf '{"hello":"world"}\n' > "$tmp/data.json"
    head -c 20971520 /dev/urandom > "$tmp/big.bin" # 20 MB, exercises multipart on re-upload
    s3 PUT demo/README.md -T "$tmp/README.md" >/dev/null
    s3 PUT demo/docs/data.json -T "$tmp/data.json" >/dev/null
    s3 PUT demo/backups/big.bin -T "$tmp/big.bin" >/dev/null
    rm -rf "$tmp"
    echo "Seeded bucket 'demo'."
    ;;
  down)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    rm -f "$CFG"
    echo "Stopped."
    ;;
  *) echo "usage: $0 up|seed|down" >&2; exit 2 ;;
esac
