# rod

Makes an IAI linear actuator (driven over **Modbus RTU**) appear as
an Ohdoki **"Handy" FW4** device, speaking the genuine `RpcMessage` RPC protocol.
See [`SPEC.md`](SPEC.md) for the full design.

Both transports carry the identical `RpcMessage` envelope; only the framing
differs. Everything above the transport — request dispatch, mode logic, actuator
translation — is shared.

```
 Handy clients ──BLE GATT──┐
                           ├──► RPC dispatch + mode logic ──► Modbus RTU ──► IAI actuator
 (cloud, deferred) ──WS────┘        (HAMP / HDSP / HSP)         (RS-485)
```

## Install on a Raspberry Pi

Two ways to get rod onto a Pi (3/4/5). Full walkthrough with Raspberry Pi
Imager steps and troubleshooting is in **[INSTALL.md](INSTALL.md)**.

**1. Flash a ready-made image — easiest, no terminal.**
Download the image for your board from the
[Releases](https://github.com/rezreal/rod/releases) page
(`rod-rpi{3,4,5}_64.img.gz`), open it in
[Raspberry Pi Imager](https://www.raspberrypi.com/software/) via
*Choose OS → Use custom*, set Wi-Fi + hostname in Imager's settings (⚙), and
write the card. Boot the Pi, plug in the actuator, and connect from the web app
over Bluetooth — it appears as `Rod-…`. The service starts on every boot.

**2. One-line installer — for an existing Raspberry Pi OS (64-bit).**

```sh
curl -sSL https://raw.githubusercontent.com/rezreal/rod/main/scripts/install.sh | sudo bash
```

Downloads and checksum-verifies the latest release binary, installs it as a
systemd service (`rod.service`), writes a default
`/etc/rod/config.toml` (an existing one is kept), and sets up
`rod.local`. **Re-run to update.** Remove with
[`scripts/uninstall.sh`](scripts/uninstall.sh) (add `--purge` to also delete the
config).

```sh
sudo systemctl status rod     # running?
journalctl -u rod -f          # live logs
```

There is no auto-updater — updating is re-running the installer (or re-flashing
the image).

## Layout

| Path | Role |
|---|---|
| `proto/` | Vendored Handy RPC `.proto` files (`hdy_rpc`), compiled by `build.rs` |
| `src/rpc/` | Generated types + `dispatch.rs` (Request → command / Response / Notification) |
| `src/modbus/` | `protocol.rs` (register/coil/bitfield packing) + `driver.rs` (sole serial owner) |
| `src/modes/` | `hamp.rs` (timer-driven oscillation), `hdsp.rs` (inline), `hsp.rs` (script playback) |
| `src/transport/` | `ble.rs` (bluer GATT peripheral, Linux), `cloud.rs` (deferred), shared `serve_frames` |
| `src/translator.rs` | Units + stroke-zone mapping |
| `src/telemetry.rs` | OpenTelemetry (env-var gated) metrics/logs/traces |
| `tests/` | Protobuf round-trip + Modbus packing integration tests |

## Build & test (dev, on this host)

```sh
cargo build
cargo test          # 34 tests: protocol packing, dispatch, mode timing, transport loop
cargo run -- config.toml
```

On non-Linux dev hosts the BLE transport is a no-op stub (it needs BlueZ/D-Bus);
everything else builds and tests normally.

## Build for the Raspberry Pi (arm64)

The only deployment target is the Pi (3/4/5), all of which run 64-bit
Raspberry Pi OS, so arm64 is the only target — no 32-bit (armv7) build. The
build is defined by the multi-stage [`Dockerfile`](Dockerfile), built for
`--platform linux/arm64` so the output is the exact `aarch64` ELF the Pi runs
(linked against `libdbus-1.so.3`).

The `builder` stage itself always runs on the build host's *own* platform and
cross-compiles to arm64, rather than running under QEMU as a "native" build
for the target — see the note in the [`Dockerfile`](Dockerfile) header. On an
Apple-Silicon host, host and target already match so this is just a native
build, same as always. On any other host (amd64 laptops/desktops, Windows or
Intel/AMD macOS via Docker Desktop, CI), rustc/LLVM run at full native host
speed and only the final link step targets arm64 — a clean build of this
crate's dependency tree takes ~4 min instead of ~7+ min under emulation, and
incremental rebuilds (touching one source file) take well under a minute.

**Just the Pi binary** (the usual case — deploy the ELF to the Pi directly):

```sh
scripts/build-pi.sh
# -> target/linux-arm64/release/rod
```

The script wraps `docker build --target export --output`, which runs the
Dockerfile's `builder` stage (installs `libdbus-1-dev` + `protobuf-compiler`,
plus a cross gcc/libc/libdbus for the target arch when cross-compiling,
`cargo build --release`, strips) and writes out only the binary — no image. It
finds Docker on `PATH` or in `~/.rd/bin` (Rancher Desktop) and needs BuildKit
(default in current Docker). BuildKit cache mounts keep `target/` and the cargo
registry warm across runs, so repeat builds are incremental despite the
from-scratch `COPY`.

**A runnable container image** (bundles the binary + `config.toml` on
`debian:bookworm-slim`):

```sh
docker build --platform linux/arm64 -t rod .
# run on the Pi, e.g.:  docker run --device /dev/ttyUSB0 rod
```

> We deliberately do **not** use the `cross` tool: its 0.2.5 images are
> amd64-only, so on an Apple-Silicon host they'd run under x86 emulation and
> the in-container rustup breaks. The Dockerfile's `builder` stage does its
> own cross-compilation instead (see above), which works natively on any host
> arch.

**Reproducibility.** The Dockerfile pins its base images (`rust`, `debian-slim`)
and the BuildKit frontend to both a version tag and a multi-arch `@sha256:`
digest; Rust deps are pinned by the committed `Cargo.lock` + `cargo build
--locked`. The only unpinned input is apt (build-time `libdbus-1-dev` /
`pkg-config` / `protobuf-compiler`) — see the note in the [`Dockerfile`](Dockerfile).
To bump an image: `docker buildx imagetools inspect <image>` and update the tag
and digest in the `ARG` lines.

Alternatively, build natively **on the Pi itself**, no Docker:
```sh
sudo apt install -y build-essential pkg-config libdbus-1-dev protobuf-compiler
cargo build --release
```

## BLE setup (Pi)

BLE setup fails with a generic `org.bluez.Error.Failed` ("Bluetooth operation
failed: Failed") for **two unrelated reasons** — check both. Run the bridge with
`RUST_LOG=rod=debug` to see which BlueZ call failed (the log now names
the operation: powering the adapter vs. GATT registration vs. advertising).

### 1. Bluetooth radio must not be rfkill-blocked

If the radio is soft-blocked, the adapter won't power on and *every* BLE call
fails. This bit us on a fresh Pi (the adapter showed `Powered: no`, `hci0 DOWN`,
`Operation not possible due to RF-kill`). Note `rfkill` lives in `/usr/sbin`, so
it may not be on a non-login `PATH` — use the full path if "command not found":

```sh
/usr/sbin/rfkill list bluetooth          # look for "Soft blocked: yes"
sudo /usr/sbin/rfkill unblock bluetooth  # clear it
```

The persisted state in `/var/lib/systemd/rfkill/` is restored at boot, so a clean
unblock normally survives a reboot. If it keeps coming back, add a boot-time
oneshot that unblocks before the bridge starts (ordered `Before=bluetooth.service`):

```sh
sudo tee /etc/systemd/system/bt-unblock.service >/dev/null <<'UNIT'
[Unit]
Description=Unblock Bluetooth rfkill
DefaultDependencies=no
After=systemd-rfkill.service
Before=bluetooth.service
[Service]
Type=oneshot
ExecStart=/usr/sbin/rfkill unblock bluetooth
RemainAfterExit=yes
[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl enable --now bt-unblock.service
```

### 2. BlueZ experimental features must be enabled

`bluer`'s GATT server + advertising require BlueZ experimental features; without
them BlueZ rejects `serve_gatt_application` / `advertise`. Enable once on the Pi
(the file may not exist on minimal images — creating it is fine):

```sh
sudo mkdir -p /etc/bluetooth
sudo sed -i 's/^#\?Experimental.*/Experimental = true/' /etc/bluetooth/main.conf 2>/dev/null
grep -q '^Experimental' /etc/bluetooth/main.conf 2>/dev/null || \
  printf '[General]\nExperimental = true\n' | sudo tee -a /etc/bluetooth/main.conf >/dev/null

sudo systemctl restart bluetooth
# confirm it's live: GattManager1 / LEAdvertisingManager1 should be present
busctl introspect org.bluez /org/bluez/hci0 | grep -E "GattManager1|LEAdvertisingManager1"
```

When both are satisfied, the bridge logs `BLE adapter powered` →
`BLE advertising started name=OHD_hw<MODEL>_<UID>`.

## Run as a systemd service

For a permanent install (auto-start on boot, restart on failure, logs to the
journal), use [`deploy/rod.service`](deploy/rod.service). It
runs the bridge **unprivileged as `pi`** and clears any rfkill soft-block first
via a root `ExecStartPre` (so the separate `bt-unblock` unit above is not needed).

Assuming the binary at `/home/pi/rod` and config at `/home/pi/config.toml`:

```sh
sudo cp deploy/rod.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now rod.service
journalctl -u rod -f          # watch it come up
```

Adjust `User=`, `WorkingDirectory=`, and the `ExecStart` paths if you deploy
elsewhere. Run **one** instance only — starting/stopping repeatedly can leak BLE
advertising instances (the Pi 3 supports just one); `sudo systemctl restart
bluetooth` clears them.

## Status / caveats

* **Modbus + RPC + modes + translator + telemetry** are complete and unit-tested
  on any platform (the serial layer builds on macOS/Linux).
* **BLE transport** uses `bluer` (BlueZ/D-Bus) and therefore only compiles and
  runs on **Linux** (e.g. the target Raspberry Pi). On other platforms a no-op
  stub is built so the rest of the bridge still builds and tests. Hardware-verified
  on a Pi 3 (Debian 13 / BlueZ 5.82): advertises once the radio is rfkill-unblocked
  and BlueZ experimental features are on — see **BLE setup (Pi)** above.
* **Cloud transport is deferred** — the FW4 device-side WebSocket URL and
  enrolment/auth handshake are firmware-baked and undocumented (SPEC §14 #1).
  `cloud.rs` is a documented placeholder.
* Several values await **hardware verification** (homing edge direction, velocity
  ceilings, HAMP smoothness) — see SPEC §14.

## Telemetry

Opt-in via standard OTel env vars. With no `OTEL_EXPORTER_OTLP_*_ENDPOINT` set
(and `OTEL_SDK_DISABLED` ≠ `true`), the SDK stays off and the bridge logs to
stderr only. Set an endpoint to enable OTLP logs + metrics + traces:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 cargo run
```
