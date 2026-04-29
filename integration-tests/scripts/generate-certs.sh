#!/usr/bin/env bash
# Generates a CA + server cert pair and PKCS12 keystore/truststore for the
# nifi-lens integration-test NiFi container.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CERTS_DIR="$SCRIPT_DIR/certs"
mkdir -p "$CERTS_DIR"

openssl req -x509 -newkey rsa:2048 -keyout "$CERTS_DIR/ca.key" \
    -out "$CERTS_DIR/ca.crt" -days 365 -nodes \
    -subj "/CN=nifi-lens Test CA" \
    -addext "basicConstraints=critical,CA:true,pathlen:0" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    2>/dev/null

openssl req -newkey rsa:2048 -keyout "$CERTS_DIR/server.key" \
    -out "$CERTS_DIR/server.csr" -nodes \
    -subj "/CN=localhost" \
    2>/dev/null

openssl x509 -req -in "$CERTS_DIR/server.csr" \
    -CA "$CERTS_DIR/ca.crt" -CAkey "$CERTS_DIR/ca.key" \
    -CAcreateserial -out "$CERTS_DIR/server.crt" -days 365 \
    -extfile <(printf "basicConstraints=critical,CA:false\nsubjectAltName=DNS:localhost,DNS:nifi-2-6-0,DNS:nifi-2-9-0-node1,DNS:nifi-2-9-0-node2,IP:127.0.0.1\nextendedKeyUsage=serverAuth,clientAuth") \
    2>/dev/null

openssl pkcs12 -export \
    -in "$CERTS_DIR/server.crt" \
    -inkey "$CERTS_DIR/server.key" \
    -certfile "$CERTS_DIR/ca.crt" \
    -out "$CERTS_DIR/keystore.p12" \
    -passout pass:testpassword \
    -name nifi-key \
    2>/dev/null

keytool -importcert \
    -file "$CERTS_DIR/ca.crt" \
    -keystore "$CERTS_DIR/truststore.p12" \
    -storetype PKCS12 \
    -storepass testpassword \
    -alias ca \
    -noprompt \
    2>/dev/null

echo "--- Certificates generated in $CERTS_DIR"
