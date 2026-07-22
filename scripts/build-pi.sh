#!/usr/bin/env bash
# Build rod for a Raspberry Pi via the multi-stage Dockerfile.
#
# The only target is the Pi: arm64 (64-bit Raspberry Pi OS) or, optionally,
# armv7 (32-bit). We build *natively* for the matching platform rather than
# using `cross` — `cross` 0.2.5 ships amd64-only images, so on an Apple-Silicon
# (arm64) host it runs under x86 emulation and breaks. A native arm64 build
# needs no emulation and produces exactly the Pi binary; armv7 runs under qemu
# emulation on arm64/amd64 hosts (slower, still correct).
#
# This extracts just the stripped ELF (Dockerfile `export` stage). To build the
# runnable container image instead:
#   docker build --platform linux/arm64 -t rod .
#
# Usage:
#   scripts/build-pi.sh            # arm64 (default)
#   scripts/build-pi.sh arm64
#   scripts/build-pi.sh armv7
#
# Output: target/linux-<arch>/release/rod  (a Linux ELF for the Pi)
#
# Requires a working Docker with BuildKit (e.g. Rancher Desktop). If `docker` is
# not on PATH this looks in ~/.rd/bin.

set -euo pipefail

arch="${1:-arm64}"
case "$arch" in
    arm64)  platform="linux/arm64"   ;;
    armv7)  platform="linux/arm/v7"  ;;
    *) echo "unknown arch '$arch' (expected arm64 | armv7)" >&2; exit 2 ;;
esac

# Locate docker (Rancher Desktop puts it in ~/.rd/bin).
if ! command -v docker >/dev/null 2>&1; then
    export PATH="$HOME/.rd/bin:$PATH"
fi
command -v docker >/dev/null 2>&1 || { echo "docker not found on PATH or ~/.rd/bin" >&2; exit 1; }

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
out_dir="${repo_root}/target/linux-${arch}/release"

echo ">> building rod for ${arch} (${platform}) via Dockerfile (export stage)"
DOCKER_BUILDKIT=1 docker build \
    --platform "$platform" \
    --target export \
    --output "type=local,dest=${out_dir}" \
    "$repo_root"

echo ">> done: target/linux-${arch}/release/rod"
file "${out_dir}/rod" 2>/dev/null || true
