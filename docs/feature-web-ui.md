# Feature: Rod Web UI

## Product framing

The Rod controller (Pi + bridge software) is the **primary product**. The web
UI is its native interface — device pairing, program selection, live telemetry, and
maintenance all happen here.

Handy ecosystem compatibility (Handy app, FunPlayer, ScriptPlayer, etc.) lives on a
completely separate BLE GATT service and is treated as a side feature — useful for
users already invested in the Handy ecosystem, but invisible to everyone else.

---

## Transport architecture

The web app connects exclusively via **Web Bluetooth** (`navigator.bluetooth`). The Pi
exposes nothing over the network — no HTTP server, no socket. The web app can be
hosted on any static HTTPS origin (GitHub Pages, CDN, or a local dev server) and
pairs directly to the Pi over BLE.

```
┌──────────────────────────────────────────────────────┐
│                    Raspberry Pi                      │
│                                                      │
│  ┌────────────────────────┐                          │
│  │   SSCP GATT service    │                          │
│  │   (new, custom UUIDs)  │                          │
│  │                        │                          │
│  │   Telemetry (notify)   │                          │
│  │   Command  (write)     │                          │
│  │   Ack      (notify)    │                          │
│  │   DevInfo  (read)      │                          │
│  └───────────┬────────────┘                          │
│              │                                       │
│  ┌───────────▼────────────┐                          │
│  │   SSCP dispatch task   │                          │
│  │   Command → ActuatorCommand → Modbus driver       │
│  │   StatusBlock → Telemetry (broadcast)             │
│  └────────────────────────┘                          │
│                                                      │
│  ┌────────────────────────┐                          │
│  │   Handy FW4 service    │  ← unchanged             │
│  │   (existing BLE GATT)  │    no interaction        │
│  └────────────────────────┘                          │
└──────────────────────────────────────────────────────┘
              ▲ BLE only
              │
    ┌─────────┴─────────────────┐
    │    Rod Web App      │
    │    (static, any HTTPS)    │
    │                           │
    │    BleTransport.ts        │
    │    (Web Bluetooth API)    │
    └───────────────────────────┘
```

### Web Bluetooth browser support

| Browser / OS | Support |
|---|---|
| Chrome / Edge — Android, Windows, macOS, Linux | ✓ |
| Samsung Internet — Android | ✓ |
| Safari — iOS / iPadOS | ✗ (Apple blocks Web Bluetooth) |
| Firefox | ✗ |

iOS is explicitly out of scope. The app should detect `!navigator.bluetooth` and show
a clear "unsupported browser" message rather than a broken UI.

Web Bluetooth requires a **secure context** (HTTPS or `localhost`). Hosting on any
public static platform (GitHub Pages, Netlify, Vercel) satisfies this automatically.
For local development `vite dev` over `localhost` is sufficient.

---

## PWA / TWA

The app communicates exclusively over BLE — it makes **zero network requests at
runtime**. This makes the offline story trivial: cache the entire build at install
time and the app works forever without connectivity.

### Service worker (Workbox via `vite-plugin-pwa`)

`vite-plugin-pwa` generates the service worker automatically from the Vite build
manifest. Strategy: **precache everything** (`generateSW` mode, `CacheFirst` for all
assets). Because there are no API calls, no runtime caching rules are needed.

```ts
// vite.config.ts (excerpt)
VitePWA({
  registerType: 'prompt',           // show "update available" UI, don't auto-reload
  workbox: {
    globPatterns: ['**/*.{js,css,html,wasm,ico,svg,png,webp}'],
  },
  manifest: { /* see below */ },
})
```

`registerType: 'prompt'` means the SW waits for user confirmation before activating
a new version — important for a device-control app where a mid-session reload would
drop the BLE connection.

### Web App Manifest

```jsonc
{
  "name": "Rod",
  "short_name": "Rod",
  "description": "Rod controller",
  "start_url": "/",
  "display": "standalone",
  "orientation": "any",
  "theme_color": "#0f172a",
  "background_color": "#0f172a",
  "icons": [
    { "src": "/icons/icon-192.png",            "sizes": "192x192", "type": "image/png" },
    { "src": "/icons/icon-512.png",            "sizes": "512x512", "type": "image/png" },
    { "src": "/icons/icon-512-maskable.png",   "sizes": "512x512", "type": "image/png",
      "purpose": "maskable" }
  ]
}
```

`display: standalone` hides the browser chrome on all platforms. `orientation: any`
lets Android rotate freely while keeping portrait the default on phones.

### Install flow

- **Android (Chrome):** browser emits `beforeinstallprompt`; the Connect screen
  shows an "Add to home screen" banner the first time. Once installed it runs as a
  standalone app with no browser UI.
- **Desktop (Chrome / Edge):** install icon appears in the address bar.
- **iOS Safari:** "Add to Home Screen" works and produces a standalone icon, but the
  app opens in Safari's WKWebView which has no Web Bluetooth — so iOS install is
  cosmetically possible but functionally useless. The app should detect this and show
  the unsupported-browser message even when running in standalone mode on iOS.

### Trusted Web Activity (Android)

A TWA wraps the hosted PWA in a native Android shell (Chrome Custom Tab) that passes
full-screen and hides the URL bar. It is validated by Digital Asset Links so the
system trusts it as equivalent to the web origin.

**Build steps (one-time):**
1. Generate the APK/AAB with [Bubblewrap CLI](https://github.com/GoogleChromeLabs/bubblewrap)
   or [PWABuilder](https://www.pwabuilder.com/).
2. Sign with a Play Store key; note the SHA-256 certificate fingerprint.
3. Host `/.well-known/assetlinks.json` on the same origin as the app:
   ```json
   [{ "relation": ["delegate_permission/common.handle_all_urls"],
      "target": { "namespace": "android_app",
                  "package_name": "com.rod.controller",
                  "sha256_cert_fingerprints": ["AA:BB:…"] } }]
   ```
4. Upload to Google Play (or side-load for private distribution).

Inside the TWA the Web Bluetooth API is available exactly as in Chrome — no
additional permissions required. The TWA is the recommended distribution path for
Android users who want a native-feeling app.

---

## Rod Control Protocol (SSCP) v1

A **new** protobuf schema — no dependency on the Handy RPC types. Defined in
`proto/sscp/v1/sscp.proto`.

### GATT layout (BLE transport)

**Service UUID:** `7e400001-b5a3-f393-e0a9-e50e24dc4179`

| Characteristic | UUID suffix | Properties | Purpose |
|---|---|---|---|
| Telemetry | `...0002...` | Notify | ~80 ms frames from device to app |
| Command | `...0003...` | Write Without Response | Commands from app to device |
| Ack | `...0004...` | Notify | Command acknowledgements |
| DeviceInfo | `...0005...` | Read | Static device metadata |

For the WebSocket transport: telemetry and ack frames are streamed as unsolicited
binary messages; commands are binary writes; a plain HTTP `GET /api/status` returns
the last telemetry frame as JSON for polling fallback.

### Proto schema (abbreviated)

```protobuf
syntax = "proto3";
package sscp.v1;

// ── Telemetry ────────────────────────────────────────────────
// Emitted every ~80 ms on the Telemetry characteristic.

message Telemetry {
  // Actuator position
  float position_mm  = 1;   // absolute, from home
  float position_pct = 2;   // 0.0–1.0 within configured zone

  // Motion
  bool moving    = 3;   // DSSE.MOVE
  bool extending = 4;   // inferred from successive position_mm delta

  // System state
  bool servo_on           = 10;  // STAT.SV
  bool controller_ready   = 11;  // DSS1.PWR
  bool homed              = 12;  // STAT.HEND / DSS1.HEND
  bool positioning_done   = 13;  // DSS1.PEND
  bool push_active        = 14;  // DSSE.PUSH (push-to-contact in progress)
  bool brake_released     = 15;  // DSS1.BKRL

  // Faults & safety
  uint32 alarm_code       = 20;  // ALMC; 0 = no alarm
  bool   alarm_minor      = 21;  // DSS1.ALML
  bool   alarm_major      = 22;  // DSS1.ALMH
  bool   emergency_stop   = 23;  // DSS1.EMGS | DSSE.EMGP
  bool   motor_voltage_low = 24; // DSSE.MPUV — only binary; no raw voltage yet
  bool   safety_speed     = 25;  // DSS1.SFTY

  // Active program
  ProgramMode mode = 30;

  oneof program_state {
    HampState hamp = 31;
    HdspState hdsp = 32;
    HspState  hsp  = 33;
  }
}

enum ProgramMode {
  IDLE   = 0;
  HAMP   = 1;
  HDSP   = 2;
  HSP    = 3;
  HOMING = 4;
}

message HampState {
  bool  running   = 1;
  float velocity  = 2;   // 0.0–1.0
  float zone_min  = 3;   // 0.0–1.0
  float zone_max  = 4;   // 0.0–1.0
  // 0 = hard/snappy reversals (max ACMD), 1 = very soft/gentle (min ACMD)
  // Handy-originated HAMP commands carry no softness preference → bridge uses 0.5
  float softness  = 5;   // 0.0–1.0
}

message HdspState {
  enum MoveState { IDLE = 0; MOVING = 1; REACHED = 2; }
  MoveState state = 1;
}

message HspState {
  enum PlayState { STOPPED = 0; PLAYING = 1; PAUSED = 2; STARVING = 3; }
  PlayState state         = 1;
  uint32    buffer_points = 2;
  float     playback_rate = 3;
  bool      looping       = 4;
}

// ── DeviceInfo ───────────────────────────────────────────────
// Returned by the DeviceInfo characteristic (read) and also as
// the first Telemetry frame after connection.

message DeviceInfo {
  string firmware_version = 1;
  float  stroke_mm        = 2;   // 100 / 150 / 200 / 250 / 300
  string device_name      = 3;   // user-configurable in config.toml
  uint32 sscp_version     = 4;   // protocol version, currently 1
}

// ── Commands ─────────────────────────────────────────────────
// Written to the Command characteristic.

message Command {
  uint32 seq = 1;   // monotonic; echoed in Ack
  oneof payload {
    StopAll       stop_all    = 10;

    HampStart     hamp_start  = 20;
    HampStop      hamp_stop   = 21;
    HampConfig    hamp_config = 22;   // partial update; unset fields unchanged

    HdspMove      hdsp_move   = 30;
    HdspStop      hdsp_stop   = 31;

    HspLoad       hsp_load    = 40;   // full script (chunked over BLE)
    HspPlay       hsp_play    = 41;
    HspPause      hsp_pause   = 42;
    HspStop       hsp_stop    = 43;
    HspSetRate    hsp_rate    = 44;

    Calibrate     calibrate   = 50;
    ResetAlarm    reset_alarm = 51;
  }
}

message HampConfig {
  optional float velocity = 1;   // 0.0–1.0
  optional float zone_min = 2;   // 0.0–1.0
  optional float zone_max = 3;   // 0.0–1.0
  optional float softness = 4;   // 0.0–1.0; absent = keep current value
}

message HdspMove {
  float position_pct = 1;   // 0.0–1.0
  float velocity_pct = 2;   // 0.0–1.0
}

message HspLoad {
  repeated HspPoint points = 1;
  bool append = 2;   // false = clear buffer first
}

message HspPoint {
  uint32 time_ms   = 1;
  uint32 position  = 2;   // 0–255
}

message HspPlay {
  bool  loop = 1;
  float rate = 2;
}

message HspSetRate {
  float rate = 1;
}

// ── Ack ──────────────────────────────────────────────────────
message CommandAck {
  uint32 seq   = 1;
  bool   ok    = 2;
  string error = 3;   // human-readable; present only when !ok
}
```

### BLE framing note

BLE write size is bounded by negotiated ATT MTU (typically 23–512 bytes). Large
`HspLoad` messages must be split into multiple `HspLoad{append: true}` writes by the
client before sending. The Rust side appends each chunk; playback begins only after
`HspPlay`. The SSCP dispatch layer is responsible for accepting chunked appends and
flushing them to the existing `HspTask`.

---

## Backend additions

### New work in the bridge process

1. **`sscp/` module** — encodes `Telemetry` from `StatusBlock`, decodes `Command`,
   translates to existing `ActuatorCommand` enum. Keeps the rest of the codebase
   unchanged.

2. **SSCP GATT service** — registers the four characteristics in `transport/ble.rs`
   alongside the existing Handy FW4 and bridge-control services. No HTTP server
   is added to the Pi.

---

## Frontend architecture

```
web/
├── index.html
├── vite.config.ts                    # includes VitePWA() plugin
├── tailwind.config.ts
├── public/
│   ├── manifest.webmanifest          # generated/validated by vite-plugin-pwa
│   ├── icons/
│   │   ├── icon-192.png
│   │   ├── icon-512.png
│   │   └── icon-512-maskable.png
│   └── .well-known/
│       └── assetlinks.json           # TWA Digital Asset Links (populate before Play release)
├── src/
│   ├── main.tsx                      # includes registerSW() from vite-plugin-pwa/client
│   ├── App.tsx                       # layout shell; shows UpdatePrompt when new SW waiting
│   ├── UpdatePrompt.tsx              # "New version available — reload?" banner
│   │
│   ├── transport/
│   │   ├── BleTransport.ts           # Web Bluetooth implementation
│   │   └── TransportProvider.tsx     # React context, GATT connect/reconnect
│   │
│   ├── proto/                        # generated from sscp.proto (protobufjs-cli)
│   │   └── sscp/v1/                  # Telemetry, Command, … TS classes
│   │
│   ├── store/
│   │   ├── deviceStore.ts            # zustand — latest Telemetry + DeviceInfo
│   │   └── commandQueue.ts           # pending acks keyed by seq
│   │
│   ├── components/
│   │   ├── layout/
│   │   │   ├── TopBar.tsx            # device name, transport badge, STOP, alarm dot
│   │   │   ├── NavRail.tsx           # tablet/desktop left nav
│   │   │   └── BottomNav.tsx         # phone bottom tab bar
│   │   │
│   │   ├── connect/
│   │   │   ├── ConnectScreen.tsx     # initial pairing; unsupported-browser gate
│   │   │   └── BleScanner.tsx        # navigator.bluetooth.requestDevice + filter
│   │   │
│   │   ├── dashboard/
│   │   │   ├── StrokeGauge.tsx       # animated vertical actuator bar (SVG)
│   │   │   ├── WaveformChart.tsx     # rolling 10 s position sparkline (Canvas)
│   │   │   ├── HealthRow.tsx         # servo, homed, voltage-low, e-stop chips
│   │   │   ├── AlarmBanner.tsx       # full-width dismissible fault strip
│   │   │   └── ProgramBadge.tsx      # IDLE / HAMP / HDSP / HSP pill
│   │   │
│   │   ├── programs/
│   │   │   ├── ProgramDrawer.tsx     # slides up from bottom on phone, side panel on tablet+
│   │   │   ├── HampControls.tsx      # speed slider, zone dual-thumb, start/stop
│   │   │   ├── HdspControls.tsx      # position + velocity sliders, quick presets
│   │   │   └── HspControls.tsx       # funscript upload, buffer bar, playback controls
│   │   │
│   │   └── maintenance/
│   │       ├── MaintenancePanel.tsx  # calibrate, reset alarm, diagnostics table
│   │       └── RawTelemetry.tsx      # all Telemetry fields as monospace table
│   │
│   ├── hooks/
│   │   ├── useDeviceState.ts         # selector over deviceStore
│   │   ├── useSendCommand.ts         # wraps transport.send + ack tracking
│   │   └── useFunscript.ts           # parse .funscript → HspPoint[], chunked load
│   │
│   └── lib/
│       ├── units.ts                  # mm ↔ pct, display formatting
│       └── alarmCodes.ts             # IAI ALMC lookup table (code → description)
│
└── tests/                            # Vitest
    ├── units.test.ts
    ├── funscript.test.ts
    └── transport.test.ts             # mock ITransport, store integration
```

State flows one way: `Telemetry` frame → `deviceStore` → React components re-render.
Commands flow the other way: component → `useSendCommand` → `ITransport.send` → ack
updates `commandQueue`.

---

## Screens

### Connect screen (initial state)

Shown when not paired. If `navigator.bluetooth` is absent (iOS, Firefox) a static
"unsupported browser" message replaces the button.

```
┌──────────────────────────────────────┐
│         Rod                    │
│                                      │
│   [ Connect via Bluetooth ]          │
│                                      │
│   ─────────────────────────          │
│   Requires Chrome or Edge            │
│   iOS is not supported               │
└──────────────────────────────────────┘
```

### Dashboard (home)

Always-visible at-a-glance state. Safe to leave on a bedside tablet in portrait.

```
┌─────────────────────────────────────────────────────┐
│  Rod  ⬤ BLE   HAMP               [■ STOP]  ⚙ │
├─────────────────────────────────────────────────────┤
│                                                     │
│  ╔══════╗    position  ▆▄▃▄▆▇▆▄▃▄▆▇▆▄▃ (10 s)     │
│  ║      ║    51 %  /  127 mm                        │
│  ║  ▓▓  ║                                           │
│  ║  ▓▓  ║    ⬤ servo   ⬤ homed   ○ e-stop          │
│  ║      ║    ○ volt-low  ○ alarm                    │
│  ╚══════╝                                           │
│                                                     │
│  ╔══════════╗  [  HAMP  ]  [  HDSP  ]  [  HSP  ]   │
│  ║ controls ║                                       │
│  ╚══════════╝  program drawer (slides in)           │
└─────────────────────────────────────────────────────┘
```

- **StrokeGauge** — SVG vertical bar; zone markers at zone_min/zone_max; fill tracks
  `position_pct` at every Telemetry tick
- **WaveformChart** — Canvas sparkline of last 10 s of `position_pct` values; cleared
  on disconnect
- **HealthRow** — icon chips for servo_on, homed, emergency_stop, motor_voltage_low,
  alarm_minor/alarm_major; green = healthy, red = fault, grey = unknown
- **STOP button** — always in TopBar; sends `StopAll`; disabled only while homing

### HAMP program

```
┌──────────────────────────────────────────────────┐
│  Oscillation                                     │
│                                                  │
│  Speed  ━━━━━━●━━━━━━━━  72%                    │
│                                                  │
│  Zone   ├━━━━━━━━━━━━━━━━┤  10 %  —  90 %       │
│         ▲                ▲  (drag handles)       │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │              ▶  START                      │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

- Speed slider sends `HampConfig{velocity}` on `pointerup`
- Zone dual-thumb sends `HampConfig{zone_min, zone_max}` on `pointerup`
- START/STOP derived from `hamp.running` in Telemetry

### HDSP program

```
┌──────────────────────────────────────────────────┐
│  Manual                                          │
│                                                  │
│  Position  ━━━━━━●━━━━━━━━  50%                 │
│  Velocity  ━━━━━━●━━━━━━━━  60%                 │
│                                                  │
│  [ MOVE ]                 [ STOP ]               │
│                                                  │
│  [ ▽ Bottom ]   [ ◇ Mid ]   [ △ Top ]            │
└──────────────────────────────────────────────────┘
```

- MOVE button sends `HdspMove`; re-arms when `hdsp.state == REACHED | IDLE`
- Quick preset buttons are `HdspMove` with fixed position_pct (0.0, 0.5, 1.0)

### HSP program

```
┌──────────────────────────────────────────────────┐
│  Script                                          │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │   drag .funscript here  /  tap to browse   │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ┤ edge-of-glory.funscript — 2 min 14 s  ✕ ├   │
│                                                  │
│  Buffer  ████████████░░░░░░░░  412 pts          │
│  Rate    ━━━━━●━━━━━━━━  1.0 ×                  │
│  Loop    ○  OFF                                  │
│                                                  │
│  [ ▶ PLAY ]   [ ⏸ PAUSE ]   [ ■ STOP ]          │
└──────────────────────────────────────────────────┘
```

- Funscript parsing and chunked `HspLoad` happen in `useFunscript`
- Buffer bar from `hsp.buffer_points`; turns red in `STARVING` state
- Rate slider sends `HspSetRate` on `pointerup`

### Maintenance panel (behind ⚙ in TopBar)

- **Calibrate** — triggers home-return; shows spinner while `mode == HOMING`;
  disables all other controls during homing
- **Reset Alarm** — only enabled when `alarm_code != 0`; shows alarm description from
  `alarmCodes.ts` lookup
- **Diagnostics table** — all Telemetry fields, monospace, updates live; useful for
  debugging
- **Device info** — firmware version, stroke variant, SSCP version (read-only)
- **Handy compatibility note** — one-line callout: "Handy app support active on
  separate BLE service"

---

## Responsive layout

| Viewport | Nav | Program controls |
|---|---|---|
| ≥ 1024 px | Left NavRail (icon + label) | Permanent right panel beside gauge |
| 768–1023 px | Left NavRail (icon only) | Permanent right panel |
| < 768 px | Bottom tab bar | Drawer slides up from bottom |

All tap targets ≥ 44 × 44 px. Sliders use `pointer-capture` so touch drag works
without accidentally scrolling the page. The StrokeGauge and WaveformChart are
`aria-hidden`; health state is also expressed as text for screen readers.

---

## Connection lifecycle

| State | UI treatment |
|---|---|
| Disconnected | ConnectScreen shown; nothing else interactive |
| Connecting | Spinner overlay on ConnectScreen; GATT service discovery in progress |
| Connected | Dashboard; TopBar shows BLE badge + device name |
| Reconnecting | TopBar badge pulses orange; controls greyed; last telemetry frozen |
| Alarm active | AlarmBanner full-width; alarm description + Reset Alarm shown |
| Homing | Blue progress banner; all program controls disabled |
| Motor voltage low | Warning chip in HealthRow; no automatic shutdown (user decides) |
| Emergency stop | Red banner; STOP and Reset Alarm shown; other controls disabled |

---

## Non-goals (this feature)

- Handy protocol changes — SSCP is additive; existing Handy GATT service is untouched
- Multi-device (one controller per bridge process)
- iOS / Firefox support (Web Bluetooth not available; iOS PWA install is cosmetically
  possible but the app gates itself behind the unsupported-browser message)
- Custom pattern / script editor (import only)
- Cloud relay / remote access
- Any network-facing server on the Pi

---

## Open questions

1. **Raw voltage telemetry.** `DSSE.MPUV` gives a binary low-voltage flag, not a
   measured value. The full IAI PCON-C register map may expose a bus voltage or motor
   current register beyond the currently polled 0x9000–0x9009 range. Worth
   investigating the manual; if found, add to the Telemetry message and to the
   diagnostics table.

2. **Velocity feedback.** Current velocity is inferred from position deltas. The IAI
   manual may expose a VNOW (velocity feedback) register. Same investigation as above.

3. **Programs roadmap.** The ProgramDrawer has three slots (HAMP, HDSP, HSP) today.
   Candidate future programs to plan nav affordances for:
   - Generative / random (auto-varies speed + zone, no user input)
   - BPM sync (tap tempo or audio analysis)
   - Breath / biometric follower
   - Partner remote (second BLE client relays commands, no cloud)
   How many programs should the nav support? Scrollable list or fixed tabs?

4. **HspLoad chunking over BLE.** Funscripts can have thousands of points. With
   typical MTU of 185 bytes, each BLE write fits ~5 points. A 5-minute script at
   1 point/100ms = 3000 points ≈ 600 writes. Define the chunking protocol precisely:
   max points per write, ack-per-chunk vs. ack-at-end, error recovery.

5. **TWA hosting origin.** The `assetlinks.json` must be served from the same HTTPS
   origin the TWA is built against. Decide the canonical host (e.g.
   `app.rod.io`) before generating the TWA, because changing the origin later
   requires a new APK signing round.

6. **Config editability.** Should stroke variant and actuator limits be editable from
   the UI, or remain `config.toml`-only on the Pi?
