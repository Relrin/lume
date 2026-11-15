#!/bin/sh
# Checks that the application's control endpoint belongs to one user only, on Linux and
# macOS.
#
#   tests/endpoint/cross-user.sh <path to lume-gui> <path to lume>
#
# Two things are checked, each from the side of whoever it protects:
#
#   1. Another user cannot reach the endpoint of a running application.
#   2. The console command refuses an endpoint that another user put where the
#      application's would be.
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

# 2. An endpoint of another user's making, where the application's would be.
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
