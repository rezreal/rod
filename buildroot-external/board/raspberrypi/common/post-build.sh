#!/bin/bash
# Called by Buildroot after the root filesystem is assembled.
# $1 = TARGET_DIR, $2 = BOARD (from BR2_ROOTFS_POST_SCRIPT_ARGS)
set -euo pipefail

TARGET_DIR="$1"
BOARD="${2:-rpi4}"

# Enable systemd services at boot
WANTS="${TARGET_DIR}/etc/systemd/system/multi-user.target.wants"
mkdir -p "${WANTS}"

# bt-attach: attaches the BCM chip to BlueZ before bluetoothd starts
ln -sf /etc/systemd/system/bt-attach.service \
    "${WANTS}/bt-attach.service"

# rod: the application itself
ln -sf /etc/systemd/system/rod.service \
    "${WANTS}/rod.service"

# avahi: advertise on the LAN as rod.local (best-effort — the unit path
# varies by Buildroot version, so don't fail the build if it's not found).
for p in /usr/lib/systemd/system/avahi-daemon.service /lib/systemd/system/avahi-daemon.service; do
    if [ -e "${TARGET_DIR}${p}" ]; then
        ln -sf "${p}" "${WANTS}/avahi-daemon.service"
        break
    fi
done

# Hostname → rod.local
echo "rod" > "${TARGET_DIR}/etc/hostname"

# Ensure config directory exists with correct permissions
install -d -m 755 "${TARGET_DIR}/etc/rod"
