# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89
#
# Multi-stage build for rod, targeting the Raspberry Pi.
#
# The deployment target is the Pi (arm64, or armv7 for 32-bit), so build with a
# matching --platform — the toolchain is native to that platform and the output
# is the exact ELF that runs on the Pi (no `cross`, no host/target mismatch):
#
#   # Runnable container image (arm64):
#   docker build --platform linux/arm64 -t rod .
#
#   # Just the stripped Pi binary, extracted to ./out/rod:
#   DOCKER_BUILDKIT=1 docker build --platform linux/arm64 \
#       --target export --output type=local,dest=out .
#
# `scripts/build-pi.sh` wraps the second form and drops the binary at the
# familiar target/linux-<arch>/release/rod path.
#
# armv7 (32-bit Pi) builds under QEMU emulation: --platform linux/arm/v7.

# Pinned to version + multi-arch index digest (works for both arm64 and armv7).
# rust:1.96.0-bookworm ships rustc 1.96.0 (>= the Cargo.toml rust-version 1.80).
# To bump: `docker buildx imagetools inspect <image>` and update tag + digest.
ARG RUST_IMAGE=rust:1.96.0-bookworm@sha256:13c186980fa33cc12759b429662a1322939dbe697484b7c33b47dd2698d28460
ARG DEBIAN_IMAGE=debian:bookworm-slim@sha256:0104b334637a5f19aa9c983a91b54c89887c0984081f2068983107a6f6c21eeb

# ── builder ──────────────────────────────────────────────────────────────────
FROM ${RUST_IMAGE} AS builder

# BLE transport links libdbus (BlueZ/D-Bus); protoc compiles the vendored RPC
# .proto files via build.rs.
#
# These apt versions are deliberately NOT pinned: the base image digest already
# fixes the OS, and exact `pkg=version` pins break on Debian point releases
# (superseded versions are dropped from deb.debian.org). For fully hermetic apt,
# point sources.list at snapshot.debian.org and pin versions there.
RUN apt-get update -qq \
 && apt-get install -y --no-install-recommends \
      libdbus-1-dev pkg-config protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /work
COPY . .

# Cache mounts keep the cargo registry and target/ across builds on the build
# host, so repeated builds are incremental despite the from-scratch COPY. The
# compiled binary lives inside the target/ cache mount, which is NOT part of the
# image layer — copy it to a stable path so later stages can reach it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/work/target \
    cargo build --release --locked \
 && cp target/release/rod /rod \
 && strip /rod

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
