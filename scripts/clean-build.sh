#!/usr/bin/env bash
# Keeps NoExplorer's build output within the storage policy (CLAUDE.md → "Storage policy").
#
# Limits: src-tauri/target ≤ 12 GB, and ≥ 60 GB free on its disk.
# Cleans in tiers, cheapest rebuild first, and never touches dependency artifacts
# (no full `cargo clean`; that only happens when the maintainer asks for it):
#   tier 0 (always)    leftovers of the old crate name "noback"
#   tier 1 (if over)   incremental caches (target/**/incremental)
#   tier 2 (if over)   this crate's own artifacts: the noexplorer binary, libnoexplorer_lib*, test
#                      binaries, fingerprints and build-script output. The next build only
#                      recompiles the app crate; dependencies stay.
# Exit: 0 = within limits, 1 = still over after cleaning (stop and ask before building), 2 = usage.
#
# Usage: scripts/clean-build.sh [--dry-run]     (npm run clean:build)
# Env:   MAX_TARGET_GB (12), MIN_FREE_GB (60), NOEXPLORER_TARGET_DIR (src-tauri/target)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T="${NOEXPLORER_TARGET_DIR:-$ROOT/src-tauri/target}"
MAX_TARGET_GB="${MAX_TARGET_GB:-12}"
MIN_FREE_GB="${MIN_FREE_GB:-60}"
DRY=0
case "${1:-}" in
  "") ;;
  --dry-run) DRY=1 ;;
  *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

size_kb() { if [ -e "$1" ]; then du -sk "$1" 2>/dev/null | awk '{s+=$1} END {print s+0}'; else echo 0; fi; }
free_kb() { df -Pk "$ROOT" | awk 'NR==2 {print $4}'; }
gb() { awk -v k="$1" 'BEGIN {printf "%.1f GB", k / 1048576}'; }
over() {
  awk -v t="$(size_kb "$T")" -v f="$(free_kb)" -v mt="$MAX_TARGET_GB" -v mf="$MIN_FREE_GB" \
    'BEGIN {exit !(t > mt * 1048576 || f < mf * 1048576)}'
}
status() { echo "$1 target $(gb "$(size_kb "$T")") (limit $MAX_TARGET_GB GB) · free $(gb "$(free_kb)") (minimum $MIN_FREE_GB GB)"; }

# Removes the given paths and prints what that freed.
remove() {
  local label="$1"; shift
  [ "$#" -eq 0 ] && { echo "  $label: nothing to remove"; return; }
  local kb=0 p
  for p in "$@"; do kb=$((kb + $(size_kb "$p"))); done
  if [ "$DRY" -eq 1 ]; then
    echo "  $label: would remove $# item(s), $(gb "$kb")"
  else
    rm -rf -- "$@"
    echo "  $label: removed $# item(s), $(gb "$kb")"
  fi
}

# Artifacts of a crate of this package (bin "$1", lib "$1_lib"), in every profile and target triple.
own_artifacts() {
  local n="$1"
  find "$T" -maxdepth 4 \( \
    -path "*/deps/$n-*" -o -path "*/deps/${n}_lib-*" -o -path "*/deps/lib${n}_lib-*" -o \
    -path "*/.fingerprint/$n-*" -o -path "*/build/$n-*" -o \
    -name "lib${n}_lib.*" -o -name "$n" -o -name "$n.d" -o -name "$n.exe" -o -name "$n.pdb" -o -name "$n.dSYM" \
    \) -prune -print 2>/dev/null
}

# Reads paths (one per line) into the global array ITEMS (bash 3.2 compatible: no mapfile/namerefs).
read_items() { ITEMS=(); local l; while IFS= read -r l; do [ -n "$l" ] && ITEMS+=("$l"); done; }

echo "NoExplorer build storage — $T"
status "before:"
if [ ! -d "$T" ]; then
  echo "  (no target directory yet)"
else
  read_items < <(own_artifacts noback)
  remove 'tier 0 · old "noback" artifacts' "${ITEMS[@]+"${ITEMS[@]}"}"

  if over; then
    read_items < <(find "$T" -maxdepth 3 -type d -name incremental -prune -print 2>/dev/null)
    remove "tier 1 · incremental caches" "${ITEMS[@]+"${ITEMS[@]}"}"
  fi
  if over; then
    read_items < <(own_artifacts noexplorer)
    remove "tier 2 · noexplorer's own artifacts (deps kept)" "${ITEMS[@]+"${ITEMS[@]}"}"
  fi
fi
status "after: "

if [ "$DRY" -eq 0 ] && over; then
  echo "STILL OVER THE LIMIT. Stop and ask the maintainer before building again" \
    "(a full \`cargo clean\` only on their request; free space may be taken by other things)."
  exit 1
fi
echo "OK"
