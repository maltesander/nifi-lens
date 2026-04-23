#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$ROOT/docker-compose.yml"
CERTS_DIR="$ROOT/scripts/certs"
CONFIG="$ROOT/nifilens-config.toml"
VERSIONS_TOML="$ROOT/versions.toml"

# Parse versions.toml via grep. `versions = ["2.6.0", "2.8.0"]` →
# one version per line in the VERSIONS array.
mapfile -t VERSIONS < <(grep -oP '"\K[0-9]+\.[0-9]+\.[0-9]+(?=")' "$VERSIONS_TOML")
if [[ ${#VERSIONS[@]} -eq 0 ]]; then
    echo "ERROR: no versions found in $VERSIONS_TOML" >&2
    exit 1
fi

cleanup() {
    docker compose -f "$COMPOSE_FILE" down
}
trap cleanup EXIT

if [[ ! -f "$CERTS_DIR/keystore.p12" ]]; then
    "$ROOT/scripts/generate-certs.sh"
fi

# Ensure the committed config file has 0600 perms — git does not track
# non-executable unix mode bits, so a fresh clone starts at 0644 which the
# nifi-lens config loader rejects.
chmod 0600 "$CONFIG"

# Download Parquet NAR for fixture (apache/nifi images don't bundle it).
"$ROOT/scripts/download-nars.sh"

echo "--- Booting NiFi containers..."
docker compose -f "$COMPOSE_FILE" up -d

# Wait for each service's healthcheck. Services boot in parallel but we
# poll sequentially — the total wall-clock is ~max(boot_time per service),
# not the sum, because they start together.
#
# Timeout is sized for GitHub Actions `ubuntu-latest` (2 vCPU / 7 GB),
# where two NiFi 2.x instances competing for CPU routinely take 4–6
# minutes each to finish booting. 600s gives headroom on cold runners
# without exhausting the 30-minute job budget.
READY_TIMEOUT="${NIFILENS_READY_TIMEOUT:-600}"
for version in "${VERSIONS[@]}"; do
    base="nifi-${version//./-}"
    # Clustered versions use <base>-node1 / <base>-node2 service names;
    # standalone versions use just <base>. Probe for -node1 first.
    if docker compose -f "$COMPOSE_FILE" ps --services 2>/dev/null | grep -q "^${base}-node1$"; then
        service="${base}-node1"
    else
        service="$base"
    fi
    echo "--- Waiting for $service healthcheck (timeout ${READY_TIMEOUT}s)..."
    SECONDS_WAITED=0
    until docker compose -f "$COMPOSE_FILE" exec -T "$service" \
            grep -q 'Started Server on https://' /opt/nifi/nifi-current/logs/nifi-app.log 2>/dev/null; do
        if [[ $SECONDS_WAITED -ge $READY_TIMEOUT ]]; then
            echo "ERROR: $service did not become ready in time" >&2
            echo "--- Last 50 lines of $service nifi-app.log:" >&2
            docker compose -f "$COMPOSE_FILE" exec -T "$service" \
                tail -n 50 /opt/nifi/nifi-current/logs/nifi-app.log >&2 || true
            exit 1
        fi
        sleep 5
        SECONDS_WAITED=$((SECONDS_WAITED + 5))
        printf "    ... %ds\r" "$SECONDS_WAITED"
    done
    echo "    $service ready after ${SECONDS_WAITED}s."
done

export NIFILENS_IT_PASSWORD="${NIFILENS_IT_PASSWORD:-adminpassword123}"
export NIFILENS_IT_USERNAME="${NIFILENS_IT_USERNAME:-admin}"

echo "--- Seeding fixtures..."
cd "$(dirname "$ROOT")"
for version in "${VERSIONS[@]}"; do
    context="dev-nifi-${version//./-}"
    echo "    Seeding $context..."
    cargo run --quiet -p nifilens-fixture-seeder -- \
        --config "$CONFIG" --context "$context"
done

echo "--- Running integration tests..."
NIFILENS_IT_CA_CERT_PATH="$CERTS_DIR/ca.crt" \
NIFILENS_IT_USERNAME="admin" \
NIFILENS_IT_PASSWORD="$NIFILENS_IT_PASSWORD" \
    cargo test --test 'integration_*' -- --ignored --nocapture
