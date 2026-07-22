# Technical Specification: Handy-Compatible Bridge for an IAI Linear Actuator

**Project:** `rod`
**Target platform:** Raspberry Pi (Linux, arm64/armv7)
**Language:** Rust (stable)
**Status:** Draft v3.0 (English)

---

## 1. Overview

`rod` makes an IAI linear actuator (driven over Modbus RTU) appear as a genuine Ohdoki "Handy" (firmware 4) device. It exposes the device on **two co-equal transports**, both speaking the **same Protobuf RPC protocol**:

1. **Cloud / Web API transport (primary use case):** the bridge connects outbound to the Handyverse socket server (`handyfeeling.com`) over a WebSocket and registers as a device. Any tool built on the **Handy REST API v3** (script players, interactive-video sites, etc.) can then control the actuator through the cloud using the device's connection key. HSSP scripting is the main scenario.
2. **BLE transport:** the bridge runs a BLE GATT peripheral with the Handy FW4 service, so BLE clients connect to it directly.

The decisive architectural fact (confirmed from the public protobuf): **both transports carry the identical `RpcMessage` envelope.** Only the framing differs (GATT TX/RX characteristics vs. WebSocket frames). Everything above the transport — request dispatch, mode logic, actuator translation — is shared.

```
                         ┌──────────────────────────── Raspberry Pi ────────────────────────────┐
                         │                                                                       │
 Handy REST API v3       │   ┌────────────────────┐                                              │
 clients (ScriptPlayer,  │   │ Cloud transport     │                                              │
 FunPlayer, web, …)      │   │ WebSocket client ───┼──┐                                           │
        │ HTTP/SSE        │   │ → handyfeeling.com  │  │                                           │
        ▼                │   └────────────────────┘  │   ┌──────────────┐   ┌────────────────┐    │
 handyfeeling.com  ──WS──┼──►                          ├──►│ RPC dispatch │──►│ Actuator       │    │
 (socket server)         │   ┌────────────────────┐  │   │ + mode logic │   │ translator     │    │
                         │   │ BLE transport       │  │   │ (shared)     │   │                │    │
 BLE clients ───GATT─────┼──►│ GATT peripheral ────┼──┘   └──────────────┘   └───────┬────────┘    │
                         │   │ TX/RX characteristics│                                 │             │
                         │   └────────────────────┘                          Modbus RTU │ RS-485    │
                         │                                                              ▼            │
                         └──────────────────────────────────────────────────  IAI actuator ────────┘
```

> **Naming note (important):** Over the RPC protocol the streaming-script protocol is **HSP** (`RequestHspSetup/Add/Play/…`). There is **no HSSP message** in the device protocol. **HSSP** is a REST-API-v3 abstraction: a REST client calls `/hssp/setup` with a URL/CSV/actions; the Handyverse cloud fetches and converts it into an **HSP point stream** which it pushes to the device. So on the bridge, "HSSP from the web" arrives as **HSP** commands — the bridge never fetches script URLs itself. (See §7.3.)

---

## 2. Technology Stack

| Component | Crate | Rationale |
|---|---|---|
| Async runtime | `tokio` | Concurrent BLE + WebSocket + Modbus |
| BLE peripheral | `bluer` (BlueZ D-Bus) | Mature Linux GATT-server impl. |
| WebSocket client | `tokio-tungstenite` + `rustls` | Cloud device transport (TLS) |
| Modbus RTU | `tokio-modbus` (rtu) | Async Modbus over serial |
| Serial | `tokio-serial` | Async serial port |
| Protobuf | `prost` + `prost-build` | Encode/decode the Handy RPC messages |
| Shared state | `Arc<RwLock<T>>`, `tokio::sync::{mpsc,broadcast}` | Fan-in transports → dispatcher → bus |
| Logging / tracing | `tracing` + `tracing-subscriber` | Structured logging, span instrumentation |
| Observability | `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry` | OTLP logs + metrics + traces, env-var driven (§11) |
| Config | `serde` + TOML | All parameters configurable |
| Errors | `thiserror` + `anyhow` | Typed errors |

```toml
# Cargo.toml (excerpt)
[dependencies]
tokio                 = { version = "1", features = ["full"] }
bluer                 = { version = "0.17", features = ["full"] }
tokio-tungstenite     = { version = "0.23", features = ["rustls-tls-native-roots"] }
tokio-modbus          = { version = "0.11", features = ["rtu"] }
tokio-serial          = "5"
prost                 = "0.12"
serde                 = { version = "1", features = ["derive"] }
toml                  = "0.8"
tracing               = "0.1"
tracing-subscriber    = { version = "0.3", features = ["env-filter"] }
tracing-opentelemetry = "0.27"
opentelemetry         = { version = "0.27", features = ["metrics", "logs", "trace"] }
opentelemetry_sdk     = { version = "0.27", features = ["rt-tokio", "metrics", "logs"] }
opentelemetry-otlp    = { version = "0.27", features = ["grpc-tonic", "http-proto", "metrics", "logs"] }
opentelemetry-semantic-conventions = "0.27"
thiserror             = "1"
anyhow                = "1"

[build-dependencies]
prost-build        = "0.12"
```

---

## 3. The Handy RPC Protocol (fully resolved from `handy-public-rpc`)

The vendored `.proto` files (`handy_rpc.proto`, `messages.proto`, `notifications.proto`, `constants.proto`, package `hdy_rpc`) define everything below. Compile with `prost-build`.

### 3.1 Envelope

```protobuf
message RpcMessage {
    MessageType type = 1;          // REQUEST | REQUESTS | RESPONSE | NOTIFICATION
    oneof message {
        Request      request      = 2;
        Requests     requests     = 3;   // repeated Request — bundle
        Response     response     = 4;
        Notification notification = 5;
    }
}
message Request  { oneof params { … } uint32 id = 2; }   // one request, client-assigned id
message Requests { repeated Request requests = 1; }       // bundle; responses come back individually
message Response { uint32 id = 1; oneof result { … } Error error = 2; }
message Notification { oneof notification { … } uint32 id = 2; }
```

Rules (from the spec): every request gets an instant response (may be blank); long actions return OK immediately, then a notification on completion; responses echo the request `id` and carry a result/`Error`; requests may be bundled; some fields require `has_xxx` presence to be acted on.

### 3.2 Mode enum (`constants.proto`)

```
MODE_HAMP=0  MODE_HSSP=1  MODE_HDSP=2  MODE_MAINTENANCE=3  MODE_HSP=4
MODE_OTA=5   MODE_BUTTON=6  MODE_IDLE=7  MODE_HVP=8  MODE_HRPP=9  MODE_DISABLED=10
```
The device usually switches mode automatically when a protocol-specific command arrives; explicit `RequestModeSet` is rarely needed. The bridge implements **HAMP, HDSP, HSP** (HSP is the on-device form of HSSP, see §1 note). HVP/HRPP are out of scope for a single linear actuator without vibration hardware.

### 3.3 Command set the bridge must implement

**HAMP — alternating motion (oscillation)**
| Request (id) | Fields | Response |
|---|---|---|
| `RequestHampStart` (720) | — | `ResponseHampStart{ HampState }` |
| `RequestHampStop` (721) | — | `ResponseHampStop{ HampState }` |
| `RequestHampVelocitySet` (723) | `float velocity` (0–1, default 0) | `ResponseHampVelocitySet{ HampState }` |
| `RequestHampStateGet` (724) | — | `ResponseHampStateGet{ HampState }` |
| `RequestHampZoneSet` (725) | `float min, max` (0–1) | `ResponseHampZoneSet{ HampState }` |

`HampState { HampPlayState play_state(STOPPED=0,RUNNING=1); float velocity; bool direction; float min; float max }`

**HDSP — direct streaming (immediate moves).** Responses are OK/Error only.
| Request (id) | Fields |
|---|---|
| `RequestHdspXaVaSet` (740) | `float xa` (mm), `float va` (mm/s), `bool stop_on_target` |
| `RequestHdspXpVaSet` (741) | `float xp` (0–1), `float va` (mm/s), `bool stop_on_target` |
| `RequestHdspXpVpSet` (742) | `float xp` (0–1), `float vp` (0–1), `bool stop_on_target` |
| `RequestHdspXaTSet` (743) | `float xa` (mm), `uint32 t` (ms), `bool stop_on_target` |
| `RequestHdspXpTSet` (744) | `float xp` (0–1), `uint32 t` (ms), `bool stop_on_target` |
| `RequestHdspXaVpSet` (745) | `float xa` (mm), `float vp` (0–1), `bool stop_on_target` |
| `RequestHdspStop` (746) | — |

`x` = position (absolute mm or percent 0–1, 0 = bottom). `v` = velocity (absolute mm/s or percent 0–1). `t` = duration ms. Absolute values are capped to device limits. Notification `NotificationHdspChanged{ HdspPlayState (STOPPED/MOVING/REACHED) }`.

**HSP — streaming script protocol (the device form of HSSP)**
| Request (id) | Fields | Notes |
|---|---|---|
| `RequestHspSetup` (860) | `uint32 stream_id` | begin a stream session |
| `RequestHspAdd` (861) | `repeated Point points`, `bool flush`, `uint32 tail_point_stream_index`, `uint32 tail_point_threshold` | append points to buffer |
| `RequestHspPlay` (863) | `int32 start_time` (ms, may be <0), `uint64 server_time`, `float playback_rate`, `bool loop`, `bool pause_on_starving` | start playback |
| `RequestHspStop/Pause/Resume` (864/865/866) | — / `bool pick_up` | |
| `RequestHspCurrentTimeSet` (868) | `int32 current_time`, `uint64 server_time`, `float filter` | resync drift |
| `RequestHspThresholdSet` (869) | `uint32 tail_point_threshold` | buffer-refill notification |
| `RequestHspPlaybackRateSet` (871) / `RequestHspLoopSet` (872) | `float playback_rate` / `bool loop` | |

`Point { uint32 t; uint32 x }` — **`t` is ms, `x` is 0–255 (8-bit position), NOT 0–100 or 0–1.** `HspState` reports `play_state`, `points`, `max_points`, `current_point`, `current_time`, `loop`, `playback_rate`, buffer head/tail times, `stream_id`, `tail_point_stream_index(+threshold)`. Threshold notifications (`NotificationHspThresholdReached`) and `NotificationHspStarving` drive client-side buffer refills — relevant mainly on the cloud transport where the server streams points in chunks.

**Slider / stroke zone & calibration**
| Request (id) | Fields | Response |
|---|---|---|
| `RequestSliderStrokeGet` (840) | — | `{ float min, max, min_absolute(mm), max_absolute(mm) }` |
| `RequestSliderStrokeSet` (841) | `float min, max` (0–1) | same shape as Get |
| `RequestSliderStateGet` (842) | — | `{ position(%), position_absolute(mm), motor_temp(°C), speed_absolute(mm/s), bool dir, … }` |
| `RequestSliderCalibrate` (843) | `bool go_to_start` | `{ bool success }` → maps to actuator homing (§9) |

Stroke zone (`min`/`max`, 0–1) defines the active sub-range; all relative positions are remapped into it. `NotificationStrokeChanged` mirrors `min/max/min_absolute/max_absolute`.

**Device / housekeeping**
`RequestModeGet/Set` (700/701), `RequestStopCurrentMode` (715), `RequestConnectionModeGet/Set` (716/717), `RequestCapabilitiesGet` (713 → advertise slider count, `ble_mtu`=512, `ws_buffer_size`), `RequestSessionIdsGet` (714), `RequestClockOffsetGet/Set` (709/712, for `server_time` sync), `RequestConnectionKeyGet` (606 → the key REST clients use to address this device), `RequestBatteryGet` (710, report mains/healthy). Relevant notifications: `NotificationModeChanged` (700), `NotificationStrokeChanged` (701), `NotificationHampChanged` (720), `NotificationHdspChanged` (740), `NotificationHspStateChanged` (861), `NotificationError` (706).

---

## 4. Transports

### 4.1 BLE transport (FW4)

Advertising name format `OHD_hw<MODEL>_<UID>` (e.g. `OHD_hw3_a1b2c3d4e5f6`; `hw3` = Handy 2). The prefix is **uppercase `OHD_`** — genuine FW4 devices advertise uppercase and the official app validates the name (both the advertised name and the GAP Device Name `0x2a00`) **case-sensitively**; a lowercase `ohd_` is discovered but rejected on connect. Verified against buttplug's hardware-tested `thehandy-v3` device config. The bridge also sets the adapter Alias to this name so `0x2a00` matches (otherwise it returns the host's hostname). The UID is generated once and persisted.

| Element | UUID |
|---|---|
| Service | `77834d26-40f7-11ee-be56-0242ac120002` |
| TX (device→client, **Notify**) | `77835410-40f7-11ee-be56-0242ac120002` |
| RX (client→device, **Write**) | `77835032-40f7-11ee-be56-0242ac120002` |

> **Characteristic direction (corrected via buttplug):** the client **writes commands on `…5032`** and **subscribes for notifications on `…5410`**. buttplug's `thehandy-v3` config names these `tx`/`rx` from the *host's* perspective, which is the opposite of the device-side TX/RX labels above — earlier drafts had the two UUIDs swapped, which let the device be discovered but made the app fail to set up its notify path on connect.

Client writes `RpcMessage`(REQUEST/REQUESTS) to RX; device emits `RpcMessage`(RESPONSE/NOTIFICATION) via TX notify. Negotiate the largest MTU the central offers (capability `ble_mtu` defaults to 512; 512 is the practical BLE cap). Messages larger than one MTU must be reassembled on RX / chunked on TX — bundles and HSP point batches are sized accordingly.

### 4.2 Cloud / Web API transport (primary)

The device connects **outbound** over a TLS WebSocket to the Handyverse socket server and exchanges the same `RpcMessage` frames. Ohdoki's own description of the flow: a controlling app sends a request to the Handyfeeling servers, the server transforms it into "Handy language" and routes it to the right device **by connection key** — i.e. the server-to-device link carries exactly the RPC protocol in this spec, and `RequestConnectionKeyGet` (606) is the routing identity.

Known host pattern (`*.handyfeeling.com`):
- REST API the *clients* call: `https://www.handyfeeling.com/api/handy-rest/v3/…` (v2 lived at `/api/handy/v2/`).
- Environment hosts mirror the proto `ServerEnvironment`: `www.handyfeeling.com` (PRODUCTION), `staging.handyfeeling.com` (STAGING), a dev host (DEVELOPMENT).
- Script hosting: `scripts01.handyfeeling.com/api/script/hosting/v0/` — where the cloud fetches HSSP scripts before streaming them to the device as HSP points (confirms §7.3).
- `ServerEnvironment.CUSTOM` (FW4.2) + `CustomServerCertificateType` (`OPEN` = no/insecure cert, `PUBLIC` = Ohdoki public cert) let a real device be pointed at a self-hosted socket server — the supported escape hatch, though it is the inverse of what this bridge needs.

`ConnectionMode` (`constants.proto`) selects which transports are live: `WIFI` (cloud only), `BLE`, `WIFI_AND_BLE` (extra latency), `OFFLINE`, `LEGACY_BLE`.

### 4.3 Bridge-control service (vendor extension)

A **second, vendor-specific** GATT service for out-of-band maintenance commands that are *not* part of the Handy FW4 protocol. It carries a tiny line-based ASCII protocol (one command per write; one textual reply per notify) instead of protobuf, so it never pollutes the FW4 message set.

| Element | UUID |
|---|---|
| Service | `6f1d0b00-9a2e-4b8c-9c11-0a1b2c3d4e5f` |
| CMD (client→device, **Write**) | `6f1d0b01-9a2e-4b8c-9c11-0a1b2c3d4e5f` |
| RESP (device→client, **Notify**) | `6f1d0b02-9a2e-4b8c-9c11-0a1b2c3d4e5f` |

Commands (subscribe to RESP notify *before* writing; the channel is strictly serial, so a long command blocks the next):

| Write | Action | Reply |
|---|---|---|
| `reset-alarm` | Clear a latched controller alarm (ALRS edge) — use after the motor faults into ERR, e.g. blocked past the thrust threshold | `ok reset-alarm` / `err <msg>` |
| `calibrate` | Home, then slow **push-to-contact** at minimal thrust to find the work-piece origin (§9.2) | `ok contact <mm>` / `err <msg>` |

> **Still genuinely open:** the *exact* FW4 device-side WebSocket URL (a specific `wss://…handyfeeling.com` socket host), the device authentication/enrolment handshake, and the WS framing of `RpcMessage`. These are baked into firmware and are **not** in the public proto, the SDK, or any community project (the `wnksy/HandyServer` redirect trick only covers **FW2**, which used plain HTTPS to `www.handyfeeling.com:443`; FW3+ changed endpoint and validates the server certificate). Getting them requires a TLS-intercept capture of a real FW4 device or Ohdoki's device-side docs. See §14 #1.

---

## 5. Configuration (`config.toml`)

```toml
[actuator]
serial_device  = "/dev/ttyUSB0"
baud_rate      = 19200             # knock-rod uses 19200 8N1; per-unit configurable
modbus_slave   = 1
variant        = "12inch"          # 4/6/8/10/12 inch → stroke 100/150/200/250/300 mm
home_direction = "negative"

[actuator.limits]
# ShockRodSize enum value == stroke in mm: FourInch=100 … TwelveInch=300.
# PNOW/PCMD in 0.01 mm units, so pos[0.01mm] = rel(0..1) × stroke_mm × 100.
min_position_mm   = 0.0
max_position_mm   = 300.0          # = stroke of selected variant (e.g. 12inch → 300)
max_velocity_mm_s = 400.0          # oscillate caps at 40000 ×0.01 mm/s; moveTo clamp ≤ 500 mm/s
default_accel_g   = 0.3            # knock-rod default (30 × 0.01 G); slider range 0.05–0.5 G
default_motion_profile = "s_curve" # "trapezoid" | "s_curve" | "filter"

[transports]
enable_ble   = true
enable_cloud = true                # primary path

[ble]
hw_model = 3                       # 3 = Handy 2
uid      = "a1b2c3d4e5f6"          # generated + persisted on first boot
adapter  = "hci0"

[cloud]
server_env  = "production"         # production | staging | custom
custom_url  = ""                   # used when server_env = "custom"
# device credentials / connection key — see §13 (registration handshake TBD)
```

---

## 6. Internal State

```rust
#[derive(Debug, Clone, PartialEq)]
enum AppMode {
    Uninitialized, Homing, Idle,
    Hamp { velocity: f32, running: bool, zone: (f32, f32) },
    Hdsp,
    Hsp  { playing: bool, looped: bool, rate: f32 },   // device-side script playback
}

struct AppState {
    mode: AppMode,
    position_mm: f32, target_mm: f32,
    slide_min: f32, slide_max: f32,        // 0..1 active stroke zone
    homing_complete: bool, servo_on: bool, alarm_code: u16, is_moving: bool,
    uid: String, connection_key: Option<String>,
    hsp_buffer: Vec<Point>, hsp_stream_id: u32,
}

enum ActuatorCommand {
    MoveTo { pos_mm: f32, vel_mm_s: f32, accel_g: f32, profile: MotionProfile },
    Home, Stop, ServoOn(bool),
}
```

Both transports decode incoming `RpcMessage` into the **same** `Request` handler, which produces `ActuatorCommand`s and `Response`/`Notification` replies routed back to the originating transport.

---

## 7. Translation Layer (RPC ⇄ Modbus)

### 7.1 Units

| Source | Meaning | → IAI Modbus (knock-rod) |
|---|---|---|
| HDSP `xa` | absolute mm | PCMD: `mm × 100` (i32, 0.01 mm) |
| HDSP `xp`, stroke `min/max`, HAMP `min/max` | percent 0–1 | map into zone, then `× stroke_mm × 100` |
| HSP `Point.x` | **0–255** | `x / 255 × stroke_mm × 100` |
| HDSP `va`, slider speed | mm/s | VCMD: `mm/s × 100` (u32, 0.01 mm/s) |
| HDSP `vp`, HAMP `velocity` | percent 0–1 | `× max_velocity_mm_s`, then `× 100` |
| HDSP `t` | duration ms | velocity = `Δmm / (t/1000)` → VCMD |
| profile / "softness" | — | CTLF MOD bits: trapezoid / S-curve / filter |

```rust
fn map_zone(p: f32, lo: f32, hi: f32) -> f32 { lo + p.clamp(0.0,1.0) * (hi - lo) }
fn rel_to_iai_pos(p: f32, stroke: f32) -> i32 { (p * stroke * 100.0) as i32 }
fn mm_to_iai_pos(mm: f32) -> i32 { (mm * 100.0) as i32 }
fn mm_s_to_iai_vel(v: f32) -> u32 { (v * 100.0) as u32 }
fn hsp_x_to_rel(x: u8) -> f32 { x as f32 / 255.0 }
```

### 7.2 HAMP (software oscillation)
HAMP maps onto knock-rod's `oscillate(speed 0–1, lower 0–1, upper 0–1, accel)`, whose algorithm is now known and is **timer-driven, not PEND-gated** — which is what makes it smooth:

```
effective_speed = speed × 40000            # 0.01 mm/s  (→ 0..400 mm/s)
on each tick (alternating out/in):
    target = (out ? min : max) × stroke_mm × 100      # 0.01 mm
    moveTo(target, effective_speed, accel)            # fire one FC0x10 move, don't await arrival
    travel_ms = (max - min) × stroke_mm × 100000 / effective_speed   # estimated time
    schedule next tick in (travel_ms + 10) ms
```

So the bridge issues a single move to one end, then schedules the reversal for the *estimated* arrival time (distance ÷ speed) plus a small margin, rather than blocking on `PEND`. `RequestHampVelocitySet` updates `speed`/zone and re-triggers immediately if the speed changed. This removes the turnaround-dwell concern from earlier drafts; the only residual question is empirical smoothness at high stroke rates over a 19200-baud link (§14 #2).

### 7.3 HSP playback (this is "web HSSP" on the device)
A REST `/hssp/...` call is converted by the cloud into HSP: `Setup` → `Add(points)` → `Play(start_time, server_time, rate, loop)`. The bridge:
- keeps the point buffer (`hsp_buffer`), honoring `flush`, `tail_point_stream_index`, and threshold notifications so the cloud can stream long scripts in chunks;
- on `Play`, computes local playback time using `server_time` and the stored clock offset, then walks points emitting `MoveTo(next.x → mm, vel = Δmm/Δt × rate)`;
- emits `NotificationHspStarving` if the buffer empties and `NotificationHspThresholdReached` at the configured tail index.
  Direct BLE clients may also drive HSP themselves with the same messages.

### 7.4 HDSP
Each HDSP request → one `MoveTo` immediately, mirroring knock-rod's `moveToWithin(rel, duration_ms, accel)`: `target[0.01mm] = rel × stroke_mm × 100`, `speed[0.01mm/s] = |target − current| / max(1, duration_ms) × 1000`, then a single FC 0x10 write. Velocity variants supply speed directly; `stop_on_target=false` (default) widens the position band for smooth chaining, `true` tightens it. Current position comes from the polled PNOW (knock-rod estimates it by interpolation between moves — the bridge can do the same to avoid a read before each command).

---

## 8. Concurrency (Tokio tasks)

| Task | Responsibility |
|---|---|
| `cloud_ws_task` | Maintain WebSocket to Handyverse; decode/encode `RpcMessage`; reconnect with backoff |
| `ble_peripheral_task` | GATT server; RX decode / TX notify; MTU + chunking |
| `dispatch_task` | Single owner of request→command logic and response routing |
| `modbus_driver_task` | **Sole owner** of the serial port; serialises all Modbus traffic |
| `status_poll_task` | Poll PNOW/STAT/DSS1; raise stroke/slider state and position notifications |
| `hamp_task` | Generate HAMP strokes when running |
| `hsp_task` | Time-stamped HSP point playback |

A priority rule in `modbus_driver_task`: movement commands preempt the status poll; poll cadence drops during active HDSP/HSP streaming to protect bus throughput.

---

## 9. Startup & Calibration

The sequence mirrors knock-rod's `KnockRod.init()` (treated as a reference):

```
1. Load config; load or generate+persist BLE UID.
2. Open serial port at 19200 8N1 (see §9.1).
3. Reset alarm: coil 0x0407 edge FF00→(20 ms)→0000.
4. PIO→Modbus switch: coil 0x0427 = FF00 (enables Modbus commands).
5. Servo ON: coil 0x0403 = FF00; verify STAT.SV / DSS1.SV.
6. Query status block once (0x9000 ×10) to seed state.
7. If DSS1.HEND is not set → Home: coil 0x040B edge FF00→(2 ms)→0000,
   settle 200 ms, then poll DSS1.HEND for up to ~12 s (homing is slow).
8. Establish coordinate mapping: home → Handy position 0.0; full stroke → 1.0.
   targetPos[0.01 mm] = rel(0..1) × stroke_mm × 100, clamped to [0, stroke_mm×100].
   `RequestSliderCalibrate` re-runs homing on demand.
9. Start ~80 ms status-poll loop; bring up enabled transports (BLE advertise
   OHD_hw3_<uid> and/or cloud WebSocket); Mode → Idle.
```

IAI Modbus details (from `rezreal/knock-rod`): status block `0x9000` (PNOW i32 @0.01 mm, STAT/DSS1/DSS2/DSSE); movement = FC `0x10` write 9 regs @ `0x9900` (PCMD i32 0.01 mm, position band, VCMD u32 0.01 mm/s, ACMD u16 0.01 G, push-current limit, CTLF). CTLF MOD0/MOD1 select trapezoid / S-curve / primary-delay-filter ("softness"). Coils: SON 0x0403, HOME 0x040B, STOP 0x042C, PMSS 0x0427, ALRS 0x0407. knock-rod's `moveTo` uses position band = 10 (0.1 mm), push-current = 0, empty CTLF, and clamps speed to 1..50000 (0.01 mm/s, i.e. ≤ 500 mm/s).

### 9.1 Modbus implementation notes (porting from knock-rod)

`knock-rod` talks raw bytes over WebSerial in the browser, so it hand-builds every frame and computes the CRC itself via the `crc` package (`crc16modbus`). **In Rust this is not ported.** `tokio-modbus`'s RTU codec owns the full ADU — it prepends the slave address and appends/validates the 2-byte Modbus CRC16 (poly `0xA001`, low byte first) on every frame automatically. You work at the PDU level through the `Reader`/`Writer` traits and never touch a CRC:

| knock-rod (raw frame) | Rust (`tokio-modbus`) |
|---|---|
| `forceSingleCoil(addr, 0xFF00/0x0000)` + manual CRC | `ctx.write_single_coil(addr, bool).await` |
| `queryHoldingRegisters(addr, n)` + manual CRC + slice off CRC | `ctx.read_holding_registers(addr, n).await -> Vec<u16>` |
| `numericalValueMovementCommand(...)` packs 18 bytes + CRC | `ctx.write_multiple_registers(0x9900, &words).await` |

What **does** carry over from knock-rod:
- **Register/coil addresses and the status bitfields** (STAT, DSS1, DSS2, DSSE, CTLF) — reuse verbatim.
- **Multi-register packing.** The move command writes 9 registers; pack the typed fields into a `[u16; 9]`, **high word first** (matches knock-rod's big-endian `DataView` writes): `PCMD` i32→2 words, position band→2, `VCMD` u32→2, `ACMD` u16→1, push-current u16→1, `CTLF` u16→1. Split helper: `let u = v as u32; [ (u >> 16) as u16, u as u16 ]`.
- **Edge-triggered coils.** Homing (`0x040B`) and alarm reset (`0x0407`) require two writes — `0xFF00` then `0x0000` — i.e. two `write_single_coil` calls, not one. This is application logic, unaffected by CRC handling.
- **Status decoding** via `read_holding_registers(0x9000, 10)` → map the returned `Vec<u16>` straight onto the bitfield enums (no byte-offset slicing as knock-rod needs, since the library already stripped address/CRC).

### 9.2 Soft-touch (push-to-contact) calibration

Beyond homing, the bridge can locate the **start of a work-piece** by sensing contact — there is no load cell; contact is sensed *electrically* via the controller's push-motion mode. Triggered by `calibrate` on the bridge-control service (§4.3):

1. **Home** to re-establish the absolute zero (DSS1.HEND).
2. Issue a **push-motion** move (FC 0x10 with CTLF.PUSH set) toward the search limit (`calibration.max_travel_mm`, or full stroke) at `calibration.touch_velocity_mm_s` with thrust capped at `calibration.push_current_pct`. Low speed + low thrust = a soft touch.
3. Poll status: **contact** = `DSSE.PUSH` asserted (pressing) or `DSS1.PEND` short of the limit (stopped early). Record `PNOW` as the contact position and store it in `work_origin_mm`. Bail with an error if the limit is reached with no contact, an alarm is raised mid-press (auto-reset then report), or it times out. Always decel-stop on exit (servo stays on).

Push-motion is also the *correct* way to avoid the blocked-motor ERR from §10: a current-limited press stalls gracefully instead of faulting.

> **Serial line parameters are now known** (from knock-rod's `KnockRod.SERIAL_OPTIONS`): **19200 baud, 8 data bits, 1 stop bit, no parity (8N1)**, with a Modbus RTU silent interval of ~2 ms between frames. Note 19200 is slow — a 25-byte status response is ~13 ms on the wire — which bounds the practical poll/stream rate (knock-rod polls roughly every 80 ms). The value is a per-unit controller setting, so keep `baud_rate` configurable, but default it to 19200 to match known-working hardware.

---

## 10. Error Handling & Safety

| Condition | Reaction |
|---|---|
| Transport drop (BLE disconnect / WS close) | Stop active mode → IDLE, deceleration stop (coil 0x042C), keep servo on; cloud task reconnects |
| Invalid request | `Response` with `Error{code,message}`, echo id |
| Mode switch while moving | Stop, wait PEND, switch |
| Modbus CRC error / timeout | Retry ≤3, then alarm/restart driver |
| Alarm (ALMC≠0) | Log + emit `NotificationError`. Clear on demand via the `reset-alarm` bridge command (§4.3) — the latched ERR (e.g. motor blocked past threshold) is not auto-reset during normal operation; only startup runs an unconditional ALRS edge |
| Position/velocity out of range | Clamp to limits; HDSP absolute values are capped by spec anyway |

Safety: soft position limits and a hard velocity cap are enforced in the translator regardless of requested values; movement is rejected until `homing_complete`; a transport loss triggers an immediate decel-stop. A movable actuator reachable from the internet (cloud transport) needs an explicit policy — connection-key confidentiality, rate limiting, and ideally a hardware e-stop independent of software.

---

## 11. Observability (OpenTelemetry)

Telemetry is **opt-in and entirely driven by the standard OTel environment variables** — no TOML config. On startup the bridge inspects the environment:

- If `OTEL_EXPORTER_OTLP_ENDPOINT` (or a signal-specific `OTEL_EXPORTER_OTLP_{LOGS,METRICS,TRACES}_ENDPOINT`) is set **and** `OTEL_SDK_DISABLED` is not `true`, the OTLP pipeline is initialised (logs + metrics + traces).
- Otherwise the SDK stays disabled and the app logs only to stderr via `tracing-subscriber`. Zero overhead, no exporter threads.

All standard variables are honoured by the SDK, including `OTEL_EXPORTER_OTLP_PROTOCOL` (`grpc` | `http/protobuf`), `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME` (default `rod`), `OTEL_RESOURCE_ATTRIBUTES`, and `OTEL_METRIC_EXPORT_INTERVAL`.

### 11.1 Initialisation

```rust
// telemetry::init() — called once in main(), returns a guard that flushes on drop
fn init() -> anyhow::Result<TelemetryGuard> {
    let otlp_enabled = std::env::var("OTEL_SDK_DISABLED").as_deref() != Ok("true")
        && (std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok()
            || std::env::var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").is_ok());

    let registry = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer());        // always: local stderr

    if otlp_enabled {
        let meter   = init_otlp_meter_provider()?;       // metrics
        let logger  = init_otlp_logger_provider()?;      // logs bridge
        let tracer  = init_otlp_tracer()?;               // spans
        registry
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&logger))
            .init();
        opentelemetry::global::set_meter_provider(meter);
    } else {
        registry.init();
    }
    Ok(TelemetryGuard { /* providers to shut down + flush */ })
}
```

A small `metrics` module creates the instruments once from `global::meter("rod")` and exposes typed helpers the rest of the code calls (e.g. `metrics::hsp_points_processed(n)`).

### 11.2 Metrics

Naming follows OTel conventions (`rod.<area>.<thing>`, base units: `mm`, `mm/s`, `ms`, `1`). Attributes in parentheses are dimensions.

**Motion / actuator**
| Instrument | Type | Unit | Notes |
|---|---|---|---|
| `rod.actuator.position` | Gauge (async) | `mm` | sampled each status poll from PNOW |
| `rod.actuator.speed` | Gauge (async) | `mm/s` | from slider `speed_absolute` / commanded VCMD |
| `rod.actuator.target_position` | Gauge | `mm` | last commanded target |
| `rod.actuator.moving` | UpDownCounter | `1` | 1 while moving, 0 settled |
| `rod.actuator.stroke_zone` | Gauge | `1` | exports `min`/`max` as separate series via attribute `bound={min,max}` |
| `rod.moves.total` | Counter | `1` | every `MoveTo` issued (attr `mode`) |
| `rod.move.distance` | Histogram | `mm` | per-move travel distance |
| `rod.homing.total` | Counter | `1` | homing runs (attr `result={ok,timeout}`) |
| `rod.homing.duration` | Histogram | `ms` | homing time |

**HSP (script playback) — the headline numbers**
| Instrument | Type | Unit | Notes |
|---|---|---|---|
| `rod.hsp.points_processed` | Counter | `1` | points actually played (attr `stream_id`) |
| `rod.hsp.points_added` | Counter | `1` | points received via `RequestHspAdd` |
| `rod.hsp.buffer_fill` | Gauge | `1` | `points / max_points` |
| `rod.hsp.buffer_points` | Gauge | `1` | current buffer depth |
| `rod.hsp.starving.total` | Counter | `1` | starvation events |
| `rod.hsp.loops.total` | Counter | `1` | loop wrap-arounds |
| `rod.hsp.playback_rate` | Gauge | `1` | current rate |
| `rod.hsp.sync_drift` | Histogram | `ms` | requested vs. local playback time at `CurrentTimeSet` |

**HAMP / HDSP**
| Instrument | Type | Unit | Notes |
|---|---|---|---|
| `rod.hamp.strokes.total` | Counter | `1` | completed strokes (attr `direction`) |
| `rod.hamp.velocity` | Gauge | `1` | current HAMP velocity 0–1 |
| `rod.hamp.cycle_time` | Histogram | `ms` | per-stroke time (the real-time health signal, §14 #2) |
| `rod.hdsp.commands.total` | Counter | `1` | HDSP requests (attr `kind=xava…xpt`) |

**Protocol / transport**
| Instrument | Type | Unit | Notes |
|---|---|---|---|
| `rod.rpc.requests.total` | Counter | `1` | attrs `transport={ble,cloud}`, `request` name, `result={ok,error}` |
| `rod.rpc.dispatch_latency` | Histogram | `ms` | request received → command issued |
| `rod.mode` | Gauge | `1` | current mode as attribute `mode=hamp/hdsp/hsp/idle` (value 1) |
| `rod.transport.connected` | UpDownCounter | `1` | per `transport` attribute |
| `rod.cloud.reconnects.total` | Counter | `1` | WebSocket reconnections |

**Modbus / lower bus (operational health)**
| Instrument | Type | Unit | Notes |
|---|---|---|---|
| `rod.modbus.txn.total` | Counter | `1` | attrs `fn`, `result={ok,crc_error,timeout}` |
| `rod.modbus.rtt` | Histogram | `ms` | request→response round-trip |
| `rod.modbus.errors.total` | Counter | `1` | CRC/timeout/exception (attr `kind`) |
| `rod.alarms.total` | Counter | `1` | controller alarms (attr `almc`) |

### 11.3 Logs & traces
`tracing` events become OTel **log records** through the appender bridge, and `#[tracing::instrument]` spans become OTel **traces** when the exporter is on. Recommended spans: one per inbound RPC request (`request`, `id`, `transport` attributes) wrapping decode→dispatch→Modbus, so dispatch latency and failures are traceable end-to-end. Keep movement-hot-path spans cheap (no per-point spans during HSP — use the counters instead; only span the `Play`/`Setup` boundaries).

### 11.4 Cardinality guard
`stream_id` and `almc` are bounded; never attach raw positions, timestamps, or the connection key as attributes (unbounded / sensitive). Per-point work is recorded as counters/histograms, never as spans or per-point attributes.

---

## 12. Directory Layout

```
rod/
├── Cargo.toml, build.rs, config.toml
├── proto/                      # vendored from handy-public-rpc
│   ├── handy_rpc.proto  messages.proto  notifications.proto  constants.proto
├── src/
│   ├── main.rs                 # task spawning
│   ├── config.rs  state.rs
│   ├── rpc/
│   │   ├── mod.rs              # prost-generated types re-export
│   │   └── dispatch.rs         # Request → ActuatorCommand; Response/Notification build
│   ├── transport/
│   │   ├── ble.rs              # advertise OHD_hwX_UID; GATT TX/RX; MTU chunking
│   │   └── cloud.rs            # WebSocket client to handyfeeling.com
│   ├── modbus/
│   │   ├── driver.rs           # tokio-modbus RTU, sole serial owner
│   │   └── protocol.rs         # frame builders (ported from knockRodProtocol.ts)
│   ├── modes/ { hamp.rs, hdsp.rs, hsp.rs }
│   ├── telemetry.rs            # OTel init (env-var gated) + metric instruments
│   └── translator.rs           # units + stroke-zone mapping
└── tests/ { rpc_roundtrip.rs, modbus_protocol.rs }
```

---

## 13. Resolved by the protobuf (previously open)

- **Full message schema** — have all `.proto` files; vendored, `prost`-compiled.
- **Exact message/field names + ids** — §3.3 is now authoritative.
- **Transport/framing** — `RpcMessage` envelope over BLE TX/RX and over WS; MTU up to 512 (`ble_mtu`); WS buffer 2048–6 kB.
- **"HSSP over BLE / URL fetch"** — moot: the device speaks **HSP point streaming**, never fetches URLs; the cloud converts HSSP→HSP.
- **Position/velocity units** — HDSP `xa` mm / `va` mm/s, `xp`/`vp` 0–1; HSP `Point.x` 0–255; slider absolute in mm and mm/s. Actuator velocity confirmed mm/s.
- **Homing hook** — `RequestSliderCalibrate` maps to IAI homing.
- **Connection key** — `RequestConnectionKeyGet` is how REST clients address the device.

---

## 14. Remaining Open Questions

| # | Item | Why it matters |
|---|---|---|
| 1 | **Cloud device-registration handshake & exact socket URL.** Architecture and host pattern are now known (server routes by connection key over `*.handyfeeling.com`; envs map to www/staging/dev; scripts via `scripts01`), but the specific FW4 `wss://` device host, the device auth/enrolment, and the WS framing of `RpcMessage` are firmware-baked and undocumented. The FW2 DNS-redirect trick (`wnksy/HandyServer`) doesn't apply — FW3+ validates the cert. | Blocks the **primary** web path. Resolve via a TLS-intercept capture of a real FW4 device or Ohdoki's device docs. **Fallback:** implement the REST API v3 surface locally and point clients (which allow a custom base URL) at the Pi — avoids the cloud but isn't the "appear in Handyverse" model. |
| 2 | **HAMP smoothness at speed** — the algorithm is now known (timer-driven reversals, §7.2), so the design is settled. Only the empirical question remains: how high a stroke rate stays smooth given accel ramps and a 19200-baud link (~13 ms per status frame). | Tune the reversal margin and max stroke rate on hardware. No longer a design unknown. |
| 3 | **Coordinate calibration** — knock-rod settles most of this: home = position 0, target `[0.01mm] = rel × stroke_mm × 100` clamped to `[0, stroke×100]`, stroke_mm from the variant. What remains is confirming the physical direction sign (which mechanical end is "bottom"/0) and any home offset on the specific rig. | Wrong direction runs inverted. Verify on hardware; otherwise resolved. |
| 4 | **Per-variant velocity/accel limits** — knock-rod caps speed at 40000–50000 (0.01 mm/s ≈ 400–500 mm/s) and accel at 0.05–0.5 G across all sizes; confirm the safe ceiling for the actual rig and stroke. Serial params are resolved (19200 8N1, §9.1). | Sets the hard caps in §7/§10. Verify on hardware. |
| 5 | **`bluer` peripheral spike** — validate GATT-server notify throughput, MTU negotiation, and advertising a 128-bit service UUID + custom name on the target BlueZ version. | De-risks the BLE transport before building on it. |
| 6 | **Multi-transport arbitration** — behaviour if BLE and cloud both try to drive at once (`WIFI_AND_BLE`); reconnection/resume semantics mid-HSP. | Define a single-active-controller policy. |

---

## 15. References

- Handy RPC protobuf (vendored): `handy-public-rpc` — `handy_rpc.proto`, `messages.proto`, `notifications.proto`, `constants.proto` (package `hdy_rpc`)
- Handy BLE FW4 protocol doc (UUIDs, `OHD_hwX_UID` naming): official "Bluetooth control" documentation
- Handy REST API v3 (HSSP/HDSP/HAMP/SSE semantics, client side): `https://www.handyfeeling.com/api/handy-rest/v3/docs/`. Related hosts: `staging.handyfeeling.com` (staging API), `scripts01.handyfeeling.com/api/script/hosting/v0/` (script hosting), `universalui.handyfeeling.com` (embeddable UI), `new.handyfeeling.com` / `handyverse.com` (FW4 web hubs). FW4 device-side socket host: not publicly documented.
- Device↔cloud architecture confirmation (server routes commands to the device by connection key): Ohdoki "Online Security and the Handy" help article.
- FW2 server-emulation reference (DNS redirect of `www.handyfeeling.com`, FW2-only): `https://github.com/wnksy/HandyServer`.
- IAI Modbus command reference: `https://github.com/rezreal/knock-rod` (`knockRodProtocol.ts`, `App.tsx`, `knockRod.ts` — all reviewed; treated as reference, not guaranteed-correct). `ShockRodSize` = 4/6/8/10/12 inch → 100/150/200/250/300 mm stroke. Serial 19200 8N1.
- `bluer` `https://docs.rs/bluer` · `tokio-modbus` `https://docs.rs/tokio-modbus` · `tokio-tungstenite` `https://docs.rs/tokio-tungstenite` · `prost` `https://docs.rs/prost`