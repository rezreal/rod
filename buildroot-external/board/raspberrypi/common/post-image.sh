#!/bin/bash
# Called by Buildroot after the filesystem images are built.
# $1 = BINARIES_DIR, $2 = BOARD (from BR2_ROOTFS_POST_SCRIPT_ARGS)
# Runs genimage to produce a ready-to-flash sdcard.img.
set -euo pipefail

BINARIES_DIR="$1"
BOARD="${2:-rpi4}"
SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
BOARD_DIR="${SCRIPT_DIR}/../${BOARD}"

rm -rf "${BUILD_DIR}/genimage.tmp"

genimage \
    --config  "${BOARD_DIR}/genimage.cfg" \
    --rootpath "${TARGET_DIR}" \
    --tmppath  "${BUILD_DIR}/genimage.tmp" \
    --inputpath "${BINARIES_DIR}" \
    --outputpath "${BINARIES_DIR}"
