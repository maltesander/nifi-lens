#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NARS_DIR="$ROOT/scripts/nars"
mkdir -p "$NARS_DIR"

# Versions to fetch — keep aligned with versions.toml.
VERSIONS=(2.6.0 2.9.0)

# NAR artifacts to download per version.
# nifi-parquet-nar depends on nifi-hadoop-libraries-nar — both must be
# present in the autoload directory; the standard apache/nifi image ships
# neither.
ARTIFACTS=(
    nifi-hadoop-libraries-nar
    nifi-parquet-nar
)

for v in "${VERSIONS[@]}"; do
    for artifact in "${ARTIFACTS[@]}"; do
        target="$NARS_DIR/${artifact}-${v}.nar"
        if [[ -f "$target" ]]; then
            echo "✓ ${artifact}-${v}.nar already cached"
            continue
        fi
        url="https://repo1.maven.org/maven2/org/apache/nifi/${artifact}/${v}/${artifact}-${v}.nar"
        echo "↓ Fetching ${url}"
        curl -fsSL "$url" -o "${target}.tmp"
        mv "${target}.tmp" "$target"
        echo "✓ ${target}"
    done
done
