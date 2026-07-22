# knock-rod / IAI Modbus protocol — field notes

Notes from re-implementing the knock-rod control protocol against a real IAI
ROBO Cylinder actuator. Intended as upstream feedback for the
[knock-rod](https://github.com/KnockRodProject/knock-rod) project.

---

## 1. CTLF MOD bits (motion profile) cause alarm 0xA3 on tested hardware

### What knock-rod does

The numerical-value movement command (FC 0x10 @ `0x9900`, 9 registers) sets the
motion-profile via the CTLF control-flag register (`reg[8]`):

| Profile | CTLF value |
|---------|-----------|
| Trapezoid | `0x00` |
| S-motion | `0x40` (bit 6, "MOD0") |
| Primary-delay filter | `0x80` (bit 7, "MOD1") |

### What actually happens

On the tested IAI controller (ROBO Cylinder PCON-C, 8-inch stroke),
**any of CTLF bits 4–7 (`0x00F0`) causes the controller to immediately assert
alarm `0x00A3`** (command-data error / position-deviation overflow) and
de-energise the servo, aborting the move.

We swept every CTLF bit individually using raw Modbus writes and then read
back the alarm code (ALMC @ `0x9002`):

| CTLF | Bit | Result | Notes |
|------|-----|--------|-------|
| `0x00` | — | ✓ OK | Trapezoid, known-good baseline |
| `0x01` | 0 | ✓ OK | |
| `0x02` | 1 | ✓ OK | |
| `0x04` | 2 | ✓ OK | |
| `0x08` | 3 | ✓ OK | |
| **`0x10`** | **4** | **✗ FAULT (0xA3)** | |
| **`0x20`** | **5** | **✗ FAULT (0xA3)** | |
| **`0x40`** | **6** | **✗ FAULT (0xA3)** | knock-rod S-curve bit |
| **`0x80`** | **7** | **✗ FAULT (0xA3)** | knock-rod Filter bit |
| `0x100`+ | 8+ | ✓ OK | Accepted, no motion change observed |

Crucially, alarm `0xA3` fires **even on a zero-distance move** (target = current
position, velocity 1 mm/s, accel 0.05 G). This confirms it is a **command
rejection** at the parameter-validation stage, not a motion tracking/servo error.

### Impact

If the bridge or software sends `default_motion_profile = "s_curve"` (CTLF
`0x40`), **every** move command faults. The servo de-energises, the red alarm
LED lights, and no further motion is possible until the alarm is reset. Because
startup always resets the alarm and re-enables the servo, the issue is masked at
boot and only manifests once the first move command is issued.

### Fix / workaround

Use `CTLF = 0x00` (trapezoid) for all moves on this controller variant. The
command is accepted and motion completes correctly. S-curve behaviour (smooth
acceleration ramps) was not achievable with any accepted CTLF value on the
tested hardware.

### Resolved: the hardware has no S-motion in this interface

Confirmed against IAI's primary source — *Serial Communication Protocol
[Modbus Version] Operation Manual* (covers ERC2, PCON-C/CG/CF/…, ACON, SCON).
The CTLF "Control Flag Specification Register" in the numerical-value movement
command (`reg[8]` of the `0x9900` block) defines **only bits 0–3**:

| CTLF bit | Meaning |
|----------|---------|
| 0 | Fixed (0) |
| 1 | Push-motion (0 = normal, 1 = push) |
| 2 | Push-motion direction |
| 3 | Incremental (relative) move |

Bits 4–7 are **undefined/reserved** → "command-data error", which is the `0x00A3`
we observe. There is **no MOD0/MOD1, no S-motion, and no filter bit**. knock-rod's
`0x40`/`0x80` mapping is incorrect for this controller family: in IAI's vocabulary
"MOD" is the *teaching-mode* bit of the device-control register, unrelated to
motion profile.

There is also **no S-motion parameter**: the Modbus manual's full user-parameter
and position-table register maps contain no S-motion / S-shape / first-order-delay
/ smoothing setting anywhere. The direct-numerical-value movement path is
**trapezoid-only by design**.

IAI *does* implement S-motion, but only as a per-row "acceleration/deceleration
mode" field of the **position table**, configured with IAI's PC software and used
in **position-number** operation — a finite set of pre-stored moves, incompatible
with continuous arbitrary-target streaming. So it is unavailable for this
application regardless.

### Recommendation for knock-rod

For PCON/ACON/SCON over the numerical-value movement command, send `CTLF = 0x00`
(trapezoid) only. Do not expose S-curve/filter as a CTLF option for this command
— the bits don't exist and any value in `0x00F0` faults the controller. For
smoother motion, soften acceleration or jerk-limit in software.

### What this bridge does

`ctlf_for_profile()` emits `0x00` for every profile, so no move can fault on a
MOD bit. For smoother oscillation it implements **software jerk-limiting**
(`src/shaper.rs`, opt-in via `[actuator.softening]`): each HAMP/ramp stroke is
expanded into a short sequence of sub-moves whose commanded velocity ramps up
along an ease-in curve, approximating an S-curve launch out of trapezoid
primitives. Bandwidth-bounded (~13 ms/frame at 19200 baud, ~80 ms poll), so it
shapes the launch edge with a handful of steps rather than a continuous profile.

---

## 2. 9-register vs 10-register move block

We tested sending 10 registers instead of 9 to the `0x9900` block (to probe
whether a separate deceleration register exists). With 10 registers written, the
controller silently **ignores the write entirely** — no motion, no alarm. The
move block appears to be exactly 9 registers on this firmware.

---

## 3. Alarm code `0x00A3` vs `0x0080`

Two alarm codes appear in practice:

| Code | When |
|------|------|
| `0xA3` | Command-data error (invalid CTLF / position out of range) — controller rejected the command |
| `0x80` | Servo system alarm / emergency stop — typically cascades from `0xA3` when a second command is issued while the servo is already faulted |

Both require an ALRS edge-reset (coil `0x0407`: `FF00` → settle 20ms → `0000`)
followed by a servo-on (coil `0x0403`) before motion can resume.

---

## 4. Home-return on startup

The controller's DSS1.HEND bit can remain set (`homing_complete = true`) across
a fault, even if the slider is jammed against a mechanical stop. Trusting HEND
and skipping the home-return on restart leads to absolute-position moves that
immediately fault with `0xA3` (the servo stalls because the rod cannot move in
the commanded direction). Always performing a home-return on startup, regardless
of HEND, resolves this reliably.

---

## 5. Physical stroke vs configured stroke

The IAI model designation (e.g. "8 inch") should be confirmed empirically by
commanding the maximum position and observing where the rod hard-clamps. On the
tested unit, a "12 inch" (`variant = "12inch"`, 300 mm) configuration produced
hard clamping at **~200.10 mm**, identifying it as an 8-inch unit. Configuring
a larger stroke than the physical one causes HAMP oscillation to hit the hard
end-stop, triggering the `0xA3` deviation-overflow alarm.

---

*Tested on: IAI PCON-C, 8-inch stroke, Modbus RTU 19200 8N1, single axis.*
