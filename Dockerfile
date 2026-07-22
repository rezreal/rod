# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89
#
# Multi-stage build for rod, targeting the Raspberry Pi.
#
# The deployment target is the Pi, arm64 (64-bit Raspberry Pi OS) only:
#
#   # Runnable container image:
#   docker build --platform linux/arm64 -t rod .
#
#   # Just the stripped Pi binary, extracted to ./out/rod:
#   DOCKER_BUILDKIT=1 docker build --platform linux/arm64 \
#       --target export --output type=local,dest=out .
#
# `scripts/build-pi.sh` wraps the second form and drops the binary at the
# familiar target/linux-arm64/release/rod path.
#
# The `builder` stage always runs on the build host's OWN platform (never
# emulated) and cross-compiles to linux/arm64. On an Apple-Silicon host, host
# == target, so this degenerates to an ordinary native build (as before). On
# any other host (amd64 laptops, CI, this repo's primary dev boxes), host !=
# target, so rustc/LLVM still run at full native speed and only the final
# link targets arm64 — no QEMU-emulated compilation. See the `builder` stage
# below for how the two cases are told apart.

# Pinned to version + multi-arch index digest.
# rust:1.96.0-bookworm ships rustc 1.96.0 (>= the Cargo.toml rust-version 1.80).
# To bump: `docker buildx imagetools inspect <image>` and update tag + digest.
ARG RUST_IMAGE=rust:1.96.0-bookworm@sha256:13c186980fa33cc12759b429662a1322939dbe697484b7c33b47dd2698d28460
ARG DEBIAN_IMAGE=debian:bookworm-slim@sha256:0104b334637a5f19aa9c983a91b54c89887c0984081f2068983107a6f6c21eeb

# Populated automatically by buildx from --platform / the build host.
ARG BUILDPLATFORM

# ── builder ──────────────────────────────────────────────────────────────────
# Always runs on the BUILD host's native platform (--platform=$BUILDPLATFORM),
# never emulated. When the build host is already arm64 (e.g. Apple Silicon),
# rustc just compiles natively as before. When it isn't (e.g. an amd64 host
# building for the Pi), we cross-compile instead of running the whole
# rustc/LLVM toolchain under QEMU: rustc/LLVM run at full native host speed
# and only the final `cc` link step targets arm64, via Debian multiarch (no
# separate sysroot needed — the target's libdbus .so/.pc just installs
# alongside the host's).
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS builder
ARG TARGETARCH
ARG BUILDARCH

# BLE transport links libdbus (BlueZ/D-Bus); protoc compiles the vendored RPC
# .proto files via build.rs (build-dependencies always run on the build host's
# arch, cross or not, so protobuf-compiler is never cross-installed).
#
# These apt versions are deliberately NOT pinned: the base image digest already
# fixes the OS, and exact `pkg=version` pins break on Debian point releases
# (superseded versions are dropped from deb.debian.org). For fully hermetic apt,
# point sources.list at snapshot.debian.org and pin versions there.
#
# This RUN sits before COPY so it Docker-layer-caches on the Dockerfile alone
# (not on source changes) — apt/rustup setup doesn't re-run every dev loop.
RUN set -eux; \
    apt-get update -qq; \
    apt-get install -y --no-install-recommends pkg-config protobuf-compiler; \
    if [ "${TARGETARCH}" != "arm64" ]; then \
        echo "unsupported target ${TARGETARCH}: this Dockerfile only targets linux/arm64" >&2; \
        exit 1; \
    elif [ "${TARGETARCH}" = "${BUILDARCH}" ]; then \
        apt-get install -y --no-install-recommends libdbus-1-dev; \
    else \
        dpkg --add-architecture arm64; \
        apt-get update -qq; \
        apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross libdbus-1-dev:arm64; \
        rustup target add aarch64-unknown-linux-gnu; \
    fi; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /work
COPY . .

# Cache mounts keep the cargo registry and target/ across builds on the build
# host, so repeated builds are incremental despite the from-scratch COPY. The
# compiled binary lives inside the target/ cache mount, which is NOT part of the
# image layer — copy it to a stable path so later stages can reach it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/work/target \
    set -eux; \
    if [ "${TARGETARCH}" = "${BUILDARCH}" ]; then \
        cargo build --release --locked; \
        cp target/release/rod /rod; \
        strip /rod; \
    else \
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
        export PKG_CONFIG_ALLOW_CROSS=1; \
        export PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig; \
        cargo build --release --locked --target aarch64-unknown-linux-gnu; \
        cp target/aarch64-unknown-linux-gnu/release/rod /rod; \
        aarch64-linux-gnu-strip /rod; \
    fi

# ── export (binary only, for bare-Pi deployment) ─────────────────────────────
# `docker build --target export --output type=local,dest=out` writes just the
# ELF — no image, no layers.
FROM scratch AS export
COPY --from=builder /rod /rod

# ── runtime (default: a runnable container image) ────────────────────────────
FROM ${DEBIAN_IMAGE} AS runtime
RUN apt-get update -qq \
 && apt-get install -y --no-install-recommends libdbus-1-3 ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /rod /usr/local/bin/rod
COPY config.toml /etc/rod/config.toml
# The bridge needs the serial device and (for BLE) host D-Bus/BlueZ — run with
# e.g. --device /dev/ttyUSB0 and, for the GATT peripheral, --net=host plus the
# host system bus. See README "Run".
ENTRYPOINT ["rod"]
CMD ["/etc/rod/config.toml"]
