#!/bin/sh
# Checks that the application's control endpoint, and the session host's, belong to one
# user only, on Linux and macOS.
#
#   tests/endpoint/cross-user.sh <path to lume-gui> <path to lume>
#
# Three things are checked, each from the side of whoever it protects:
#
#   1. Another user cannot reach the endpoint of a running application.
#   2. Another user cannot reach the endpoint of a running session host.
#   3. The console command refuses an endpoint that another user put where the
#      application's would be, and one put where the session host's would be.
#
# It needs a display for the application, `sudo` without a password, and `python3`. The
# other user is `nobody`. It is meant for a CI runner: it plants a socket as root.

set -eu

app=$1
lume=$2
name="xuser-$$"
other=nobody

fail() {
    echo "cross-user: FAILED: $1" >&2
    exit 1
}

# Where the endpoint lives, as the application places it: the runtime directory on
# Linux when there is one, a directory of the user's own under the temporary one
# otherwise.
if [ "$(uname)" = Linux ] && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
    dir="$XDG_RUNTIME_DIR/lume"
else
    dir="${TMPDIR:-/tmp}"
    dir="${dir%/}/lume-$(id -u)"
fi
socket="$dir/$name.sock"

connect='
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
'

# 1. A running application's endpoint.
"$app" --endpoint "$name" &
tries=0
until "$lume" ctl --endpoint "$name" status >/dev/null 2>&1; do
    tries=$((tries + 1))
    [ "$tries" -lt 300 ] || fail "the application did not start"
    sleep 0.2
done
[ -S "$socket" ] || fail "the endpoint is not at $socket"

# The owner reaches it. This is what shows the check below can tell the difference.
python3 -c "$connect" "$socket" || fail "the owner could not connect to $socket"

if sudo -u "$other" python3 -c "$connect" "$socket" 2>/dev/null; then
    fail "another user connected to $socket"
fi
if sudo -u "$other" ls "$dir" >/dev/null 2>&1; then
    fail "another user can list $dir"
fi
echo "cross-user: another user cannot reach the endpoint"

"$lume" ctl --endpoint "$name" quit >/dev/null
wait

# 2. A running session host's endpoint. It is named after the application's.
host_socket="$dir/$name-host.sock"
"$lume" host --endpoint "$name" start >/dev/null || fail "the session host did not start"
[ -S "$host_socket" ] || fail "the session host's endpoint is not at $host_socket"
python3 -c "$connect" "$host_socket" || fail "the owner could not connect to $host_socket"
if sudo -u "$other" python3 -c "$connect" "$host_socket" 2>/dev/null; then
    fail "another user connected to $host_socket"
fi
echo "cross-user: another user cannot reach the session host"
"$lume" host --endpoint "$name" stop --end-sessions >/dev/null || fail "the session host did not stop"

# 3. An endpoint of another user's making, where the application's would be.
planted="xplant-$$"
planted_socket="$dir/$planted.sock"
sudo python3 -c '
import os, pwd, socket, sys, time
path, owner = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX)
s.bind(path)
user = pwd.getpwnam(owner)
os.chown(path, user.pw_uid, user.pw_gid)
os.chmod(path, 0o666)
s.listen(1)
time.sleep(30)
' "$planted_socket" "$other" &
planter=$!
tries=0
until sudo test -S "$planted_socket"; do
    tries=$((tries + 1))
    [ "$tries" -lt 100 ] || fail "the planted socket did not appear"
    sleep 0.1
done

status=0
answer=$("$lume" ctl --endpoint "$planted" status) || status=$?
sudo kill "$planter" 2>/dev/null || true
sudo rm -f "$planted_socket"
[ "$status" -ne 0 ] || fail "the console command talked to another user's endpoint: $answer"
case "$answer" in
*'"ok":false'*) ;;
*) fail "the console command did not refuse: $answer" ;;
esac
echo "cross-user: the console command refuses an endpoint another user made"

# The same where the session host's endpoint would be: neither `lume host` nor `lume ctl`,
# which turns to the session host when no application runs, talks to it.
planted_host="$dir/$planted-host.sock"
sudo python3 -c '
import os, pwd, socket, sys, time
path, owner = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX)
s.bind(path)
user = pwd.getpwnam(owner)
os.chown(path, user.pw_uid, user.pw_gid)
os.chmod(path, 0o666)
s.listen(1)
time.sleep(30)
' "$planted_host" "$other" &
planter=$!
tries=0
until sudo test -S "$planted_host"; do
    tries=$((tries + 1))
    [ "$tries" -lt 100 ] || fail "the planted host socket did not appear"
    sleep 0.1
done

host_status=0
host_answer=$("$lume" host --endpoint "$planted" status) || host_status=$?
ctl_status=0
ctl_answer=$("$lume" ctl --endpoint "$planted" list) || ctl_status=$?
sudo kill "$planter" 2>/dev/null || true
sudo rm -f "$planted_host"
[ "$host_status" -ne 0 ] || fail "lume host talked to another user's endpoint: $host_answer"
[ "$ctl_status" -ne 0 ] || fail "lume ctl talked to another user's session host: $ctl_answer"
case "$ctl_answer" in
*'"ok":false'*) ;;
*) fail "lume ctl did not refuse another user's session host: $ctl_answer" ;;
esac
echo "cross-user: the console command refuses a session host another user made"
