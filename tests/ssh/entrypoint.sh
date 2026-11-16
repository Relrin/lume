#!/bin/sh
# Starts the test server. LUME_TEST_PASSWORD is the account's password; a file at
# /keys/authorized_keys names the keys the account accepts.
set -eu

if [ -z "${LUME_TEST_PASSWORD:-}" ]; then
    echo "LUME_TEST_PASSWORD is not set" >&2
    exit 64
fi
echo "lume:${LUME_TEST_PASSWORD}" | chpasswd >/dev/null

if [ -f /keys/authorized_keys ]; then
    cp /keys/authorized_keys /home/lume/.ssh/authorized_keys
    chown lume:lume /home/lume/.ssh/authorized_keys
    chmod 600 /home/lume/.ssh/authorized_keys
fi

# New host keys at every start, so a server that was started again looks changed.
rm -f /etc/ssh/ssh_host_*_key /etc/ssh/ssh_host_*_key.pub
ssh-keygen -A >/dev/null

exec /usr/sbin/sshd -D -e
