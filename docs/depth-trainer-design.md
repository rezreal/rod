# Depth Trainer — design (draft, awaiting confirmation)

SSCP program mode `depth_trainer` (`src/modes/depth.rs`). Helps a user get
comfortable with a gradually increasing insertion depth over many sessions
(weeks). Two exercises: **hold** at depth, and **oscillate** at depth.

Nothing here has been tested on hardware.

## How it fits the existing code

Same shape as plumb/echo/impale: one task (`DepthTask`) with an
`mpsc<DepthControl>` channel, a `DepthRuntime` in `AppState`, an `AppMode::DepthTrainer`
variant, `Command::DepthStart/DepthButton/DepthStop/...` in `src/sscp`, telemetry
`depth_trainer: Option<DepthTelemetry>`, dispatcher stop-on-mode-switch +
`stop_everything`, and a web `DepthTrainerControls` card + manual section.

Deadman input: on-screen `Button{down}` heartbeat (client resends every ~50 ms,
as games/impale) **and** the hardware hand switch, routed through
`Dispatcher::hand_switch` the same way impale does (`Press|Hold` → down,
`Release` → up). Either source holding counts as "held".

### Two things in the existing code that matter for safety

1. **`ModbusDriver::depth_scaled` rescales every mode move** by
   `ceiling / stroke` (`comfortable_depth_mm` for unlisted modes). Depth Trainer
   needs exact "mm beyond the origin", so it must get its own branch there:
   no rescale, only the existing `HARD_STOP_MARGIN_MM` clamp. The task enforces
   its own ceiling = `min(max_depth_mm, stroke − margin)` before issuing any
   move. (A test pins this.)
2. **No mode currently reacts to a controller alarm / e-stop / lost link.** Other
   modes just keep timing. Depth Trainer will watch `alarm_major`,
   `emergency_stop` and `actuator_connected` each tick. Limit that I cannot
   remove in software: if the controller itself has faulted/cut power, or the
   Modbus link is down, no retract command can reach the rod. The e-stop /
   hand-switch hardware is the backstop there. The UI says so.

## Safety model

**Fail-safe deadman.** The rod only advances / oscillates / dwells while the
deadman is held. Any of: release, heartbeat gap > `deadman_timeout_ms`
(150 ms), `Stop`, transport drop (`stop_everything`), mode switch, alarm,
e-stop, link loss, task error ⇒ **emergency retract**: `Stop` then `MoveTo(origin)`
at retract speed, servo left **on** (no brake — see impale.rs). The retract
profile and speed are compile-time floors/ceilings; config cannot disable it.
`Stop` handling in this task is "retract", unlike other modes that release the
servo.

**Breath-hold budget.** The user records `breath_hold_s` once (required before
the first session; re-confirmable any time; persisted).

- Per-excursion cap: `cap = min(breath_hold_s × fraction, ABS_MAX_S)`;
  `ABS_MAX_S = 15 s` is a `const`, config can only lower it.
- The *submerged timer* starts when the **sensed** `position_mm` leaves
  `origin + 1 mm` and stops when it returns. It includes outbound transit,
  dwell and return transit — not just the dwell.
- Planning: dwell budget = `cap − t_out − t_retract_est − margin`. If that is
  ≤ 0 for the requested depth/speed the step is refused ("too deep for the
  current breath-hold; increase breath-hold or lower depth"), not shortened
  silently. (Slow, gentle motion eats the budget: 40 mm at 4 mm/s is 10 s out
  alone.)
- Hard watchdog: independent of the plan, if the submerged timer reaches
  `cap − t_retract_est − margin` the task emergency-retracts.
- Mandatory rest at/outside the origin after every excursion:
  `rest ≥ max(3 × t_submerged, 20 s)`. The next advance is blocked until the
  rest has elapsed *and* the deadman is held again.
- Oscillation: each stroke is its own excursion for the per-excursion cap and
  returns to the origin with a pause (default 2 s) before the next stroke.
  Additionally a set's cumulative time beyond the "breathing-safe offset" is
  capped — **open question** (below).

**Gentle motion.** Advance and oscillation use low speed/accel with S-curve;
the last 5 mm to the deepest point are a separate slower leg (≈ 40 % speed).
Only the emergency retract is faster (it still decelerates smoothly into the
origin).

**Calibration required.** Start is refused with a clear error unless
`work_origin_mm` is `Some`. All depths are mm beyond the origin, clamped to
`max_depth_mm`. Max-depth changes are already refused while a program runs.

## Session structure

1. Arm: user holds deadman; origin re-checked.
2. **Warm-up**: 3 strokes to 50 % of target oscillation depth.
3. **Oscillation sets**: `sets × strokes` at target oscillation depth
   (default 3 × 6), rest between sets.
4. **Hold sets**: 3 holds at hold depth, each ≤ dwell budget, rest between.
5. **Cool-down**: 2 shallow strokes, then rod parked at origin.
6. End screen: user rates the session (*comfortable / okay / too much*).

Any deadman release mid-set aborts the set (emergency retract + rest) and marks
the session "had early release".

## Progression (persisted)

Sidecar `depth-trainer.json` next to the binary (same pattern as `device-uid`,
`max-depth-mm`). Contents: `breath_hold_s`, per-exercise `{depth_mm, hold_s}`,
session history (date, targets reached, early releases, rating), last
progression time. Written atomically (tmp + rename); a corrupt file falls back
to the conservative starting values, never to larger ones.

- Start: oscillation depth 10 mm, hold depth 5 mm, hold time 5 s.
- Session targets = stored comfortable values (+ at most one step, see below).
- Rated **comfortable**, no early release, and cooldown elapsed (24 h) ⇒
  advance: first `hold_s += 1 s` until it reaches the cap, then
  `depth_mm += 2 mm` and `hold_s` resets to 5 s. Oscillation depth advances
  with the same rule (depth +2 mm).
- **okay** ⇒ keep. **too much** ⇒ step back one step.
- 2 consecutive sessions with early release ⇒ UI suggests a step back (user
  confirms; never automatic).
- Nothing ever advances twice inside the cooldown, and a depth can never exceed
  `max_depth_mm`.

## UI

Phase, submerged-time countdown, rest countdown, depth gauge, big hold-to-run
button (pointer/touch capture, release on blur/visibilitychange), emergency
"retract now" button. Spoken guidance (Web Speech API, `web/src/lib/depthSpeech.ts`
maps status transitions to cues, `web/src/audio/speech.ts` speaks them) is a purely
supplementary UI layer: on by default, per-viewer voice/rate/volume prefs, cancels
stale speech on every cue, critical cues (emergency retract, pre-move countdown)
interrupt, and it is a no-op when `speechSynthesis` is unavailable. Tones (`web/src/audio/tones.ts`): a soft cue ~2 s before
advancing and before the planned retract; a distinct cue on emergency retract.

## Tests (planned)

Hold-time clamp incl. absolute ceiling and config trying to raise it; budget
refusal when transit eats the cap; rest ≥ max(3×, 20 s) enforced; progression
+ cooldown + step-back + early-release; deadman lapse ⇒ `Stop`+`MoveTo(origin)`;
`Stop`/alarm/link-loss ⇒ retract; no-calibration refusal; `depth_scaled` pass-through
for the new mode; persistence round-trip + corrupt-file fallback; wire-tag
deserialization for the new commands.

## Decisions confirmed by the user

- Hold limits: per-excursion cap = `min(0.6 × breath_hold_s, ABS_MAX_S)`, `ABS_MAX_S = 20 s`
  (const; config can only lower it). Rest = `max(3 × time submerged, 20 s)`.
- Breathing: the user can **not** reliably breathe at the origin; only above a
  user-set **breathing-safe offset** (`breath_offset_mm`, mm beyond the origin;
  0 = origin = most conservative). "Submerged" = sensed
  `position > origin + breath_offset_mm`. Oscillation strokes turn around at the
  offset (not the origin), pause ≥ 2 s there (breath window), and the cumulative
  submerged time of a set is also capped by the per-excursion cap (so a set
  has fewer strokes at greater depth). Holds return to the origin.
- Progression: +2 mm, 24 h cooldown. Hold time +1 s per comfortable session up
  to the dwell budget, then depth +2 mm and hold time resets to 5 s.
- **Deadman = hardware hand switch only.** The web UI cannot hold or start
  motion (no button command exists); it can only arm, stop/retract, set
  breath-hold/offset, rate, step back, and view state. The task reads
  `AppState::hand_switch` directly and treats it as *not held* when
  `actuator_connected` is false or the status poll is stale
  (`AppState::status_seq` unchanged for > `deadman_timeout_ms`).
- Starting values/speeds: advance 4 mm/s (last 5 mm at 1.6 mm/s), normal return
  10 mm/s (added: a slow return would eat the breath budget), emergency retract
  20 mm/s; oscillation depth 10 mm, hold depth 5 mm for 5 s.

## Wire format (SSCP JSON; telemetry camelCase, command tags snake_case)

Commands (web → bridge):
- `{type:'depth_start'}` — arm a session (targets come from stored progress). Ignored unless `depthProgress.canStart`.
- `{type:'depth_stop'}` — emergency retract to the origin, end and leave the mode.
- `{type:'depth_settings', breathHoldS:number, breathOffsetMm:number}` — both required; ignored while a session is active.
- `{type:'depth_rate', rating:'comfortable'|'okay'|'too_much'}` — only in the `rating` phase.
- `{type:'depth_step_back'}` — user-confirmed step back; idle only.

Telemetry: top-level `depthProgress` (ALWAYS present) and `depthTrainer`
(present only while `mode === 'depth_trainer'`):

```
depthProgress: {
  breathHoldS: number|null, breathOffsetMm: number,
  oscillationDepthMm: number, holdDepthMm: number, holdS: number,   // NEXT session's targets
  holdCapS: number|null,          // per-excursion cap, null until breath-hold is set
  sessions: number,
  nextAdvanceInS: number,         // 0 = a +step is allowed now
  suggestStepBack: boolean,
  canStart: boolean, blockReason: string|null,
  history: { atMs:number, oscillationDepthMm:number, holdDepthMm:number, holdS:number,
             earlyReleases:number, rating:string|null }[]            // last 10, newest last
}
depthTrainer: {
  phase: 'arming'|'warmup'|'oscillation'|'hold'|'rest'|'cooldown'|'retracting'|'rating'|'ended',
  setIndex:number, setCount:number, strokeIndex:number, strokeCount:number,   // 1-based, 0 when n/a
  targetDepthMm:number, plannedHoldS:number|null,
  depthBeyondOriginMm:number, submergedS:number, submergedCapS:number|null,
  restRemainingS:number, restTotalS:number,
  deadmanHeld:boolean, awaitingDeadman:boolean,
  emergency:boolean, emergencyReason:string|null, earlyRelease:boolean,
  cueSoonS:number|null            // seconds to next advance / planned retract when <= 3 s, else null
}
```
Mode label in `status.mode` is `depth_trainer`.

## Implementation notes (as built)

- Extra safeguards beyond the brief: a stale status poll (`status_seq` unchanged
  > `deadman_timeout_ms`), a started calibration, or another program taking the
  mode all count as faults / stop → retract. A release during a normal
  end-of-block return finishes the return at retract speed. The second deadman
  release in a session ends the session (first one: retract, rest, continue).
- Driver: `AppState::depth.driving` makes `depth_scaled` pass positions through
  unscaled from session start until the final retract has finished (needed
  because `stop_everything` flips the mode to Idle *before* the task retracts).
- On exit the servo is left **on** at the origin (no working brake).
- Config: `[actuator.depth_trainer]`; every safety value is clamped to
  compile-time bounds in `modes::depth::Limits::from_config`.

## Not verified / known limits

- **Not tested on hardware.** Deadman-release → retract latency is unmeasured:
  hand-switch bit refreshes with the ~80 ms status poll, plus the 25 ms task
  tick and the Modbus write of `Stop` then `MoveTo`; and whether a `MoveTo`
  issued mid-motion (after `Stop`) behaves smoothly on the IAI controller.
- A controller fault, e-stop power cut or lost Modbus link cannot be retracted
  from software.
- Retract completion is judged from sensed position; the two-leg approach
  (fast, then the last 5 mm slow) pauses briefly at the junction.

## Timed warm-up (added after review)

Each session starts with a warm-up so the work-piece can self-lubricate.
Lubrication can't be sensed, so it is **timed, then the user confirms**:

- If the breathing-safe offset is ≥ 2 mm: gentle continuous strokes between the
  origin and `min(offset, max(½ × oscillation depth, 2 mm))` at the advance speed,
  for `warmup_s` (default 120 s). These stay above the offset, so they never
  count as submerged and need no breath budget or rest. Releasing the switch
  retracts as usual and the warm-up resumes (it is never skipped).
- If the offset is < 2 mm there's no breathing room: the old short shallow
  stroke warm-up runs (3 strokes, normal budget/rest rules) and there is no gate.
- When the time is up the rod parks at the origin (phase `warmup`,
  `awaitingWarmupConfirm`) and nothing moves until the user taps **Continue** or
  **+1 min more** (`warmup_extend_s`, up to `warmup_max_s` = 10 min total).
  Each extension and the main blocks still need the switch held plus the normal
  3 s countdown.

Wire additions: commands `{type:'depth_warmup_continue'}` and
`{type:'depth_warmup_extend'}` (only honoured at the gate); `depthTrainer` gains
`warmupRemainingS: number|null` (time left; `0` at the gate; `null` outside the
warm-up), `awaitingWarmupConfirm: boolean`, `warmupCanExtend: boolean`.
