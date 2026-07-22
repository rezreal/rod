#!/usr/bin/env bash
# Download and unpack Buildroot 2024.02 LTS into ./buildroot/.
# Run once after cloning the repo. The buildroot/ directory is git-ignored.
set -euo pipefail

BUILDROOT_VERSION="2024.02"
BUILDROOT_URL="https://buildroot.org/downloads/buildroot-${BUILDROOT_VERSION}.tar.gz"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${REPO_ROOT}/buildroot"

if [[ -d "${TARGET}" ]]; then
    echo "buildroot/ already exists — nothing to do."
    echo "To upgrade: rm -rf buildroot/ && $0"
    exit 0
fi

echo ">> Downloading Buildroot ${BUILDROOT_VERSION}..."
mkdir -p "${TARGET}"
curl -L "${BUILDROOT_URL}" \
    | tar xz --strip-components=1 -C "${TARGET}"

echo ""
echo ">> Buildroot ${BUILDROOT_VERSION} ready at buildroot/"
echo ""
echo "Quick start (RPi 4):"
echo "  cd buildroot"
echo "  make O=output/rpi4 BR2_EXTERNAL=\$(pwd)/../buildroot-external rod_rpi4_64_defconfig"
echo "  make O=output/rpi4"
echo "  # output: output/rpi4/images/sdcard.img"
