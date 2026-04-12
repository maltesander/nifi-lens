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

# Clustering — only applied when the container sets NIFI_CLUSTER_IS_NODE=true.
# Standalone containers (e.g., nifi-2-6-0) omit this env var entirely.
if [ "${NIFI_CLUSTER_IS_NODE}" = "true" ]; then
    prop_replace 'nifi.cluster.is.node'                       'true'
    prop_replace 'nifi.cluster.node.address'                  "${NIFI_CLUSTER_NODE_ADDRESS}"
    prop_replace 'nifi.cluster.node.protocol.port'            '11443'
    prop_replace 'nifi.zookeeper.connect.string'              "${NIFI_ZOOKEEPER_CONNECT_STRING}"
    prop_replace 'nifi.cluster.flow.election.max.wait.time'   '30 secs'
    prop_replace 'nifi.state.management.embedded.zookeeper.start' 'false'
    # Cluster nodes require an explicit sensitive props key — the same
    # value on every node. Standalone mode auto-generates one, but
    # cluster mode refuses to start without it.
    prop_replace 'nifi.sensitive.props.key'                   "${NIFI_SENSITIVE_PROPS_KEY:-nifilens-fixture-key}"
fi

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
