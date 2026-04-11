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

if [ -n "${SINGLE_USER_CREDENTIALS_USERNAME}" ] && [ -n "${SINGLE_USER_CREDENTIALS_PASSWORD}" ]; then
    "${NIFI_HOME}/bin/nifi.sh" set-single-user-credentials \
        "${SINGLE_USER_CREDENTIALS_USERNAME}" \
        "${SINGLE_USER_CREDENTIALS_PASSWORD}"

    prop_replace 'nifi.security.user.authorizer' 'managed-authorizer'

    unset SINGLE_USER_CREDENTIALS_USERNAME
    unset SINGLE_USER_CREDENTIALS_PASSWORD
fi

unset AUTH
exec /opt/nifi/scripts/start.sh
