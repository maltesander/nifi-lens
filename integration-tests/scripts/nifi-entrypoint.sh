#!/bin/sh
set -e

. /opt/nifi/scripts/common.sh

prop_replace 'nifi.security.keystore'           "${KEYSTORE_PATH}"
prop_replace 'nifi.security.keystoreType'       "${KEYSTORE_TYPE}"
prop_replace 'nifi.security.keystorePasswd'     "${KEYSTORE_PASSWORD}"
prop_replace 'nifi.security.keyPasswd'          "${KEY_PASSWORD:-$KEYSTORE_PASSWORD}"
prop_replace 'nifi.security.truststore'         "${TRUSTSTORE_PATH}"
prop_replace 'nifi.security.truststoreType'     "${TRUSTSTORE_TYPE}"
prop_replace 'nifi.security.truststorePasswd'   "${TRUSTSTORE_PASSWORD}"

# Clustering properties are handled by NiFi's own start.sh via env vars:
#   NIFI_CLUSTER_IS_NODE, NIFI_CLUSTER_ADDRESS, NIFI_CLUSTER_NODE_PROTOCOL_PORT,
#   NIFI_ZK_CONNECT_STRING, NIFI_ELECTION_MAX_WAIT, NIFI_SENSITIVE_PROPS_KEY.
# start.sh also calls update_cluster_state_management.sh which patches
# state-management.xml with the ZK connect string. No prop_replace needed here.

if [ -n "${SINGLE_USER_CREDENTIALS_USERNAME}" ] && [ -n "${SINGLE_USER_CREDENTIALS_PASSWORD}" ]; then
    # `set-single-user-credentials` rewrites login-identity-providers.xml AND
    # authorizers.xml to use the `single-user-authorizer`, which grants the
    # sole user unconditional full access to every resource. That's exactly
    # the shape we want for an integration harness — one admin, no policy
    # management. We do NOT override `nifi.security.user.authorizer` here;
    # the default left by `set-single-user-credentials` is correct.
    "${NIFI_HOME}/bin/nifi.sh" set-single-user-credentials \
        "${SINGLE_USER_CREDENTIALS_USERNAME}" \
        "${SINGLE_USER_CREDENTIALS_PASSWORD}"

    unset SINGLE_USER_CREDENTIALS_USERNAME
    unset SINGLE_USER_CREDENTIALS_PASSWORD
fi

unset AUTH
exec /opt/nifi/scripts/start.sh
