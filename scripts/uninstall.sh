#!/usr/bin/env bash
#
# Remove a rod install done by scripts/install.sh.
#   curl -sSL https://raw.githubusercontent.com/rezreal/rod/main/scripts/uninstall.sh | sudo bash
# Pass --purge to also delete the config in /etc/rod.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "Please run with sudo." >&2; exit 1; }

echo "Stopping and disabling the service…"
systemctl disable --now rod 2>/dev/null || true
rm -f /etc/systemd/system/rod.service
systemctl daemon-reload

rm -f /usr/local/bin/rod
echo "Removed binary and service."

if [ "${1:-}" = "--purge" ]; then
  rm -rf /etc/rod
  echo "Purged /etc/rod."
else
  echo "Kept config in /etc/rod (pass --purge to remove it)."
fi
echo "Done."
