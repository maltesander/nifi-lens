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

echo "--- Booting NiFi containers..."
docker compose -f "$COMPOSE_FILE" up -d

# Wait for each service's healthcheck. Services boot in parallel but we
# poll sequentially — the total wait time is ~max(90s per service), not
# the sum, because they start together.
for version in "${VERSIONS[@]}"; do
    service="nifi-${version//./-}"
    echo "--- Waiting for $service healthcheck..."
    SECONDS_WAITED=0
    until docker compose -f "$COMPOSE_FILE" exec -T "$service" \
            grep -q 'Started Server on https://' /opt/nifi/nifi-current/logs/nifi-app.log 2>/dev/null; do
        if [[ $SECONDS_WAITED -ge 240 ]]; then
            echo "ERROR: $service did not become ready in time" >&2
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
