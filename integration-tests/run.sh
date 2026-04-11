#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$ROOT/docker-compose.yml"
CERTS_DIR="$ROOT/scripts/certs"
NIFI_URL="${NIFILENS_IT_URL:-https://localhost:8443}"
NIFI_USERNAME="${NIFILENS_IT_USERNAME:-admin}"
NIFI_PASSWORD="${NIFILENS_IT_PASSWORD:-adminpassword123}"

cleanup() { docker compose -f "$COMPOSE_FILE" down; }
trap cleanup EXIT

if [[ ! -f "$CERTS_DIR/keystore.p12" ]]; then
    "$ROOT/scripts/generate-certs.sh"
fi

echo "--- Booting NiFi..."
docker compose -f "$COMPOSE_FILE" up -d

echo "--- Waiting for healthcheck..."
SECONDS_WAITED=0
until docker compose -f "$COMPOSE_FILE" exec -T nifi \
        grep -q 'Started Server on https://' /opt/nifi/nifi-current/logs/nifi-app.log 2>/dev/null; do
    if [[ $SECONDS_WAITED -ge 180 ]]; then
        echo "ERROR: NiFi did not become ready in time"
        exit 1
    fi
    sleep 5
    SECONDS_WAITED=$((SECONDS_WAITED + 5))
    printf "    ... %ds\r" "$SECONDS_WAITED"
done
echo "    NiFi ready after ${SECONDS_WAITED}s."

echo "--- Running integration tests..."
cd "$(dirname "$ROOT")"
NIFILENS_IT_URL="$NIFI_URL" \
NIFILENS_IT_USERNAME="$NIFI_USERNAME" \
NIFILENS_IT_PASSWORD="$NIFI_PASSWORD" \
NIFILENS_IT_CA_CERT_PATH="$CERTS_DIR/ca.crt" \
    cargo test --test 'integration_*' -- --ignored --nocapture
