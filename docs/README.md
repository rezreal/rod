# rod — engineering notes

Key findings and hard-won lessons from the initial bring-up. Listed so the next
person (or future-you) doesn't spend hours on each one.

---

## Contents

- [IAI Modbus protocol findings](#iai-modbus-protocol-findings)
- [BLE stack bring-up](#ble-stack-bring-up)
- [Deployment](#deployment)
- [Known bugs fixed in this codebase](#known-bugs-fixed-in-this-codebase)
- [Debug tooling](#debug-tooling)

---

## IAI Modbus protocol findings

### CTLF motion-profile bits cause alarm 0xA3

The most impactful finding. knock-rod (the reference implementation) maps the
motion profile into CTLF `reg[8]` of the 9-register move block (`0x9900`):

- Trapezoid → `0x00`
- S-curve → `0x40` (bit 6)
- Filter → `0x80` (bit 7)

On the tested IAI PCON-C controller **any of CTLF bits 4–7 causes immediate
alarm `0xA3`** (command-data error), even on a zero-distance move at minimum
speed. The servo de-energises, the alarm LED lights, and no further motion is
possible until the alarm is reset. S-curve is effectively **unavailable** on
this hardware.

Only `CTLF = 0x00` (trapezoid) is accepted. This is now the hardcoded default
in `protocol.rs`; the profile config key is preserved for documentation and
future hardware that may support other values.

See [`knock-rod-protocol-notes.md`](knock-rod-protocol-notes.md) for the full
bit sweep and context.

### Always home on startup

The controller's HEND bit ("home complete") can stay set across a fault even
when the slider is physically jammed at the end-stop. Trusting HEND and skipping
the home-return means the servo has a stale position reference; every absolute
move then faults with `0xA3`. **Always issue a home-return during startup**
regardless of the current HEND value.

### Physical stroke ≠ configured stroke

Verify stroke empirically by commanding `max_position_mm` and watching where
the rod hard-clamps. The tested unit (labelled/guessed as 12-inch) clamps at
~200 mm — it is an 8-inch unit. Over-configuring the stroke means HAMP
oscillation hits the physical end-stop and faults.

### Alarm reset is an edge, not a level

ALRS (coil `0x0407`) needs a `FF00 → 0000` edge with ≥20 ms gap between edges.
Writing `FF00` and leaving it set does not clear the alarm.

After reset, the servo must be explicitly re-enabled (coil `0x0403` → `FF00`)
before motion commands are accepted again.

---

## BLE stack bring-up

### TX and RX UUIDs are swapped from the host's perspective

The Handy FW4 GATT service has two characteristics. From the **device**
(peripheral / bridge) point of view:

| UUID | Direction | GATT property |
|------|-----------|---------------|
| `77835410-…` | **TX** (device → client) | Notify |
| `77835032-…` | **RX** (client → device) | Write |

A naive reading of the names could lead to wiring them the wrong way round —
the client writes to the "RX" characteristic (from the device's perspective)
and subscribes to the "TX" characteristic. If you swap these, the client writes
to a notify-only characteristic and subscriptions land on a write-only one; BLE
connects fine but no commands are processed and no notifications arrive.

This was verified against the buttplug `thehandy-v3.yml` device config, which
documents the same mapping.

### BlueZ battery plugin causes pairing failures

With the default BlueZ battery plugin active, BlueZ reads the host's battery
level and tries to forward it to connected centrals. This triggers an
authentication/pairing request. If pairing is rejected (as it is on browsers
using Web Bluetooth), BlueZ immediately disconnects. The fix is to start
`bluetoothd` with `--noplugin=battery`:

```ini
# /etc/systemd/system/bluetooth.service.d/noplugin-battery.conf
[Service]
ExecStart=
ExecStart=/usr/libexec/bluetooth/bluetoothd --noplugin=battery
```

(`DisablePlugins` in `main.conf` is silently ignored — only the daemon flag
works.)

### Advertising instances leak on repeated restarts

The Pi 3 BLE controller supports exactly 1 advertising instance. If the bridge
process is killed without cleanly stopping the advertisement, the instance
remains allocated in BlueZ. After ~4–5 crashes, advertising fails with
`ActiveInstances` ≥ `SupportedInstances`. Fix: `sudo systemctl restart bluetooth`.

The bridge now cleans up its advertisement on exit via `bluer`'s RAII drop.

### BLE adapter name must match the expected prefix

The Handy client app filters for `namePrefix: 'OHD_'` and optionally a specific
format `OHD_hw<MODEL>_<UID>`. Both the **advertising name** (set on the
`LEAdvertisement`) and the **adapter alias** (the GAP device name, set via
`adapter.set_alias()`) must be set. Setting only one of the two leaves the other
at the hostname, which breaks name filtering.

---

## Deployment

### `pkill -f rod` kills the SSH shell

`pkill -f` matches against the full command line. An SSH session whose command
line contains the string `rod` (e.g. `ssh pi ... 'pkill -f
rod; mv rod.new rod'`) **kills the shell itself**
before the `mv` can run. The old binary stays in place; the new one is stranded
as `rod.new`.

Always use `pkill -x rod` (exact comm-name match). The `deploy-pi.sh`
script encodes this and also verifies the running `/proc/<pid>/exe` sha256
matches the local build before reporting success.

### Verify the running binary's hash, not just the deployed file

A failed deploy (silently killed shell, network blip, scp to a different path)
leaves the old binary running. Always confirm:

```sh
sha256sum /proc/$(pgrep -x rod)/exe
```

matches the local build output. `deploy-pi.sh` does this automatically.

### Docker release build fails with exit 101 (not OOM)

`scripts/build-pi.sh` runs Docker BuildKit and pipes output through `tail`,
masking the build exit code. A Rust compile error in the Linux-only BLE module
(which macOS `cargo build` never compiles) shows up as exit 101 but the wrapper
reports success. To diagnose, run with `--progress=plain` and grep for `error`.

---

## Known bugs fixed in this codebase

### HAMP reversal timer: `from_secs_f32(ms)` → 26-minute delays

`hamp.rs` computed the reversal timer as:

```rust
let travel_ms = (span_mm / speed_mm_s) * 1000.0;
let travel = Duration::from_secs_f32(travel_ms.max(1.0)) + …;
//                    ^^^ takes SECONDS; travel_ms is MILLISECONDS
```

At 100 mm/s over a 160 mm span, `travel_ms = 1600`. `from_secs_f32(1600)` =
**1600 seconds = 26 minutes**. The rod reached the first end, the reversal
timer was set 26 minutes in the future, and oscillation appeared to stop.

The test didn't catch this because `tokio::test(start_paused = true)` with
`cmd_rx.recv().await` causes tokio to auto-advance the mock clock to the next
pending timer, regardless of how far away it is. The test passed instantaneously
whether the timer was 1.6 seconds or 1600 seconds.

Fix: `Duration::from_millis(travel_ms.max(1.0) as u64)`.

### First BLE write after subscribe is dropped

After `startNotifications()`, the first write-without-response to the RX
characteristic is sometimes dropped or corrupts the first few bytes (likely a
race in the BlueZ characteristic-control setup). A 250 ms settle delay between
subscribing to notifications and the first write resolves this reliably.

### Auto-alarm-reset blocks the driver loop

The alarm auto-reset in `poll_once()` uses `sleep(500ms).await` inline. Because
the driver's `run()` loop `await`s `poll_once()`, this blocks the entire driver
(cmd_rx, bridge_rx, poll) for 500 ms. Any move commands queued during that
window are suppressed by the alarm-check guard (alarm_code is still non-zero in
AppState until the next poll cycle updates it). In practice this only occurs
when a fault happens; normal oscillation is unaffected.

---

## Debug tooling

### Raw-Modbus debug console

The bridge can expose a line-based TCP console on loopback that proxies text
commands to the Modbus driver. Enable in `config.toml`:

```toml
[debug]
enable = true
listen = "127.0.0.1:7878"
```

Connect over SSH:

```sh
ssh pi "nc 127.0.0.1 7878"
```

Commands:

| Command | Effect |
|---------|--------|
| `status` | Decoded 10-word status block (PNOW, ALMC, DSS1, …) |
| `rreg <addr> <count>` | Read holding registers; hex with `0x` prefix |
| `wreg <addr> <w0> [w1 …]` | Write holding registers (FC 0x10) |
| `wcoil <addr> <0\|1>` | Write single coil (FC 0x05) |
| `testmove <pos_001> <vel_001> <accel_001> <ctlf>` | Atomic: write move block, settle 150 ms, read ALMC back |
| `reset-alarm` | ALRS edge reset |
| `calibrate` | Home + push-to-contact calibration |
| `help` | Command list |

`testmove` is particularly useful for protocol experiments: it writes a move,
waits long enough for the alarm to latch, and reads ALMC in a single driver
serialisation — the status-poll / auto-reset cannot clear the alarm between the
write and the read.

### Useful Modbus addresses

| Address | Name | Notes |
|---------|------|-------|
| `0x9000` | PNOW_hi | 32-bit current position, 0.01 mm units |
| `0x9001` | PNOW_lo | |
| `0x9002` | ALMC | Present alarm code; 0 = normal |
| `0x9005` | DSS1 | Status bits: HEND (home), SV (servo), PWR |
| `0x9007` | DSSE | PUSH bit set when push-to-contact contact detected |
| `0x9900–0x9908` | Move block | 9 registers; FC 0x10 to command a move |
| `0x0403` | COIL_SERVO | `FF00` = servo on, `0000` = servo off |
| `0x0407` | COIL_ALARM_RESET | ALRS edge: `FF00` → 20 ms → `0000` |
| `0x040B` | COIL_HOME | Home-return edge: `0000` → `FF00` |
| `0x0427` | COIL_PIO_MODBUS | `FF00` = enable Modbus numerical commands (PMSS) |
