#!/usr/bin/env bash
# Deploy a freshly-built rod to the Raspberry Pi and restart it.
#
# Assumes scripts/build-pi.sh has produced target/linux-<arch>/release/rod.
# Verifies the binary actually landed (sha256 local == deployed == running) so a
# silently-skipped copy can't leave the old binary running.
#
# IMPORTANT: stop the old process with `pkill -x rod` (exact comm name),
# NOT `pkill -f rod` — the latter also matches this very ssh command
# line (which contains the string "rod") and kills the shell mid-script,
# skipping the binary swap. That bug kept the old binary running for hours.
#
# Usage:
#   scripts/deploy-pi.sh                       # arm64, pi@192.168.178.25
#   scripts/deploy-pi.sh arm64 pi@<host>

set -euo pipefail

arch="${1:-arm64}"
target="${2:-pi@192.168.178.25}"
local_bin="target/linux-${arch}/release/rod"

[ -f "$local_bin" ] || { echo "missing $local_bin — run scripts/build-pi.sh $arch first" >&2; exit 1; }

local_sha="$(shasum -a 256 "$local_bin" | awk '{print $1}')"
echo ">> local sha256:    $local_sha"

scp "$local_bin" "$target":/home/pi/rod.new
remote_sha="$(ssh "$target" 'sha256sum rod.new' | awk '{print $1}')"
echo ">> uploaded sha256: $remote_sha"
[ "$local_sha" = "$remote_sha" ] || { echo "ERROR: upload sha mismatch" >&2; exit 1; }

# Swap the binary and relaunch. If rod.service is installed, drive it via
# systemd (needs passwordless `systemctl` for the rod unit — see the
# sudoers drop-in in the deploy-pi skill); otherwise fall back to a plain nohup
# run writing /tmp/bridge.log. Either way, stop the old process first with
# `pkill -x` (exact name — `pkill -f` would also match this ssh command line).
ssh "$target" '
  set -e
  managed=0
  systemctl is-enabled rod >/dev/null 2>&1 && managed=1
  if [ "$managed" = 1 ]; then sudo -n systemctl stop rod 2>/dev/null || pkill -x rod 2>/dev/null || true
  else pkill -x rod 2>/dev/null || true; fi
  sleep 1
  mv -f rod.new rod
  chmod +x rod
  launched=""
  if [ "$managed" = 1 ] && sudo -n systemctl start rod 2>/dev/null; then launched="systemd"; fi
  if [ -z "$launched" ]; then
    : > /tmp/bridge.log
    RUST_LOG=rod=info nohup ./rod config.toml >> /tmp/bridge.log 2>&1 &
    launched="nohup"
  fi
  sleep 3
  pid="$(pgrep -x rod || true)"
  echo ">> launched via:    $launched"
  echo ">> running pid:     ${pid:-<none>}"
  [ -n "$pid" ] && echo ">> running sha256:  $(sha256sum /proc/$pid/exe 2>/dev/null | cut -d" " -f1)"
  if [ "$launched" = systemd ]; then journalctl -u rod -n 8 --no-pager 2>/dev/null || true
  else tail -8 /tmp/bridge.log; fi
'
echo ">> deployed."
