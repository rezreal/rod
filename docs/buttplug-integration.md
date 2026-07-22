# Design note: integrating buttplug.io for additional BLE devices

Status: **design only — not implemented.** Captures the plan for driving many
BLE toys/sensors via [buttplug](https://buttplug.io) alongside the IAI rod.

## Goal

Let rod control a broad range of Bluetooth actuators (vibrators, strokers,
rotators, …) and read extra biosensors, driven by the *same* programs and web UI
that drive the rod — without hand-writing a BLE protocol per device.

The **rod stays on Modbus RTU**. buttplug is purely *additive*: it manages
*other* BLE devices. rod becomes a multi-device controller, not a
rewrite.

## What buttplug gives us

- **`buttplug` crate** (Rust, BSD-3-Clause — embeddable). Ships a large device
  database (Lovense, We-Vibe, Kiiroo, Magic Motion, …) with protocols already
  implemented, plus some sensors (battery, pressure, button).
- A **capability model** every device maps onto:
  - `ScalarCmd` — vibrate / oscillate / constrict / inflate (0..1 intensity)
  - `LinearCmd` — position over a duration (**stroking** — maps almost 1:1 onto
    the motion our programs already generate)
  - `RotateCmd` — rotation speed + direction
  - `SensorRead` / `SensorSubscribe` — battery, pressure, etc.
- BLE backend is **btleplug** (on Linux → BlueZ/D-Bus).

## The key hazard: two BLE stacks on one adapter

We already use **`bluer`** for the peripheral GATT server + advertising
([`transport/ble.rs`](../src/transport/ble.rs)) and for the heart-rate central
([`sensors/`](../src/sensors/)). buttplug brings **btleplug**, a *second* BLE
library, onto the *same* BlueZ adapter.

- BlueZ permits multiple D-Bus clients per adapter, and modern BlueZ
  reference-counts discovery per client, so concurrent scanning largely
  coexists. The real cost is **radio airtime**: advertising + scanning + N
  connections time-multiplex one antenna.
- **Mitigations**: give buttplug sole ownership of central *scanning*; keep
  `bluer` peripheral-focused. If contention bites, add a **USB BLE dongle as
  `hci1`** and split roles across adapters — no architecture change.
- **Heart rate**: buttplug is toy-focused and likely won't enumerate a standard
  Heart-Rate-Service strap, so our HR central ([`sensors/`](../src/sensors/))
  stays separate (or moves to the second adapter).

## Integration options

### Option A — embed `buttplug` in-process
Run a `ButtplugServer` + in-process client + btleplug comm manager inside
rod. **Single binary, no extra process.** Costs: large dependency tree,
heavier Pi cross-compile and bigger Buildroot image, two BLE stacks in one
process, tracking buttplug's API across versions.

### Option B — Intiface sidecar (recommended)
Run buttplug's **Intiface engine** as a separate local process; rod
connects to it as a buttplug **client over WebSocket** (the standard way apps
use buttplug). **More robust** (separate process + BLE stack, independently
restartable; sidesteps in-process two-runtime friction) and keeps rod's
binary lean. Cost: a second service to install/run — on the appliance image it'd
be bundled as another systemd unit.

Either way, the rod-internal architecture is identical (below); only the
"how we reach buttplug" differs. **Recommendation: B** for the appliance goals.

## Architecture inside rod

A new **device-fleet layer**, parallel to the existing rod path:

1. **`buttplug` module (client)** — connects (in-process or to Intiface ws),
   enumerates devices, exposes each as an actuator with its capabilities, and
   surfaces its sensors into `AppState` (exactly like the HR sensor does today).
2. **Motion fan-out** — today programs emit `ActuatorCommand` → shaper → Modbus
   driver. Add a tap so program *intent* also reaches selected buttplug
   actuators, translated per capability:
   - stroke target/velocity → `LinearCmd` for other strokers,
   - intensity (e.g. ramp/pulse level, 0..1) → `ScalarCmd` for vibrators,
   - direction → `RotateCmd`.
   Each device has an enable + scale/assignment so the user picks what mirrors
   what.
3. **Sensors** — buttplug `SensorSubscribe` readings land in `AppState` and can
   feed reactive programs (like Pulse consumes heart rate).

### Surface
- **SSCP telemetry**: a `devices` list (id, name, capabilities, connected) and
  their sensor values.
- **Web UI**: a "Devices" panel — discovered devices, enable/assign each to a
  motion channel (mirror stroke / map intensity), per-device scale.
- **Config**: `[buttplug] enable`, connection mode (in-process vs ws URL),
  optional device allow-list.

## Costs & risks

- **Build size**: btleplug + device config materially grow the binary and the
  Buildroot image; the cross-compile gets slower. (Option B keeps rod's
  own binary lean and pushes this into Intiface.)
- **API churn**: buttplug's Rust API moves between majors — pin a version.
- **RF contention**: covered above; dongle escalation available.
- **Safety**: the same stop/limit/deadman properties must extend to fanned-out
  devices — `StopAll` must stop *every* actuator, and a dropped buttplug
  connection must fail safe.

## Phased plan (when we build it)

1. **Spike**: connect (Option B, local Intiface), enumerate devices, log
   capabilities — and confirm BLE coexistence with our peripheral + HR central
   on the actual Pi.
2. **Device layer**: client module → `AppState` device list + SSCP telemetry +
   web "Devices" panel (read-only).
3. **Fan-out**: mirror rod motion onto one buttplug actuator (`LinearCmd` /
   `ScalarCmd`), with enable + scale; extend `StopAll`.
4. **Sensors**: buttplug sensor → `AppState` → reactive program input.
5. **Appliance**: bundle Intiface as a systemd unit in the Buildroot image
   (Option B), or add the embedded path (Option A) behind a feature flag.

## References
- buttplug: https://buttplug.io · crate: https://crates.io/crates/buttplug
- btleplug (BLE backend): https://crates.io/crates/btleplug
- Intiface engine: https://intiface.com
