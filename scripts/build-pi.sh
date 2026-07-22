#!/usr/bin/env bash
# Build rod for a Raspberry Pi via the multi-stage Dockerfile.
#
# The only target is the Pi, arm64 (64-bit Raspberry Pi OS) — every model we
# support (3/4/5) runs it, so there's no 32-bit (armv7) target. We don't use
# the `cross` tool — `cross` 0.2.5 ships amd64-only images, so on an
# Apple-Silicon (arm64) host it runs under x86 emulation and breaks. Instead
# the Dockerfile's `builder` stage always runs on the build host's own
# platform and cross-compiles to arm64: on an Apple-Silicon host, host ==
# target so it's a plain native build; on any other host (amd64,
# Windows/Intel-Mac via Docker Desktop, CI) it's a real cross-compile —
# rustc/LLVM run at native host speed, no QEMU.
#
# This extracts just the stripped ELF (Dockerfile `export` stage). To build the
# runnable container image instead:
#   docker build --platform linux/arm64 -t rod .
#
# Usage:
#   scripts/build-pi.sh
#
# Output: target/linux-arm64/release/rod  (a Linux ELF for the Pi)
#
# Requires a working Docker with BuildKit (e.g. Rancher Desktop). If `docker` is
# not on PATH this looks in ~/.rd/bin.

set -euo pipefail

# Locate docker (Rancher Desktop puts it in ~/.rd/bin).
if ! command -v docker >/dev/null 2>&1; then
    export PATH="$HOME/.rd/bin:$PATH"
fi
command -v docker >/dev/null 2>&1 || { echo "docker not found on PATH or ~/.rd/bin" >&2; exit 1; }

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
out_dir="${repo_root}/target/linux-arm64/release"

echo ">> building rod for arm64 (linux/arm64) via Dockerfile (export stage)"
DOCKER_BUILDKIT=1 docker build \
    --platform linux/arm64 \
    --target export \
    --output "type=local,dest=${out_dir}" \
    "$repo_root"

echo ">> done: target/linux-arm64/release/rod"
file "${out_dir}/rod" 2>/dev/null || true
