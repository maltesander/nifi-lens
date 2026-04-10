#!/usr/bin/env bash
# release.sh — convenience wrapper around cargo-release.
#
# Usage:
#   release/release.sh patch              # dry-run a patch release
#   release/release.sh minor              # dry-run a minor release
#   release/release.sh major              # dry-run a major release
#   release/release.sh patch --execute    # actually perform the release
#
# cargo-release is dry-run by default; --execute is required to mutate state.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: release.sh <patch|minor|major> [--execute]" >&2
  exit 2
fi

BUMP="$1"
shift

exec cargo release "$BUMP" "$@"
