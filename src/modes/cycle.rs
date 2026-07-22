//! Cycle — a pattern playlist driven by one button (SSCP extension).
//!
//! The actuator continuously plays a motion *pattern* over a fixed zone (the
//! same origin/distance for every pattern — they differ only in speed and
//! shape). A **short press** advances to the next of ten patterns; a **long
//! press** (≥ `long_press_ms`, default 2 s) toggles pause. Patterns run from
//! simple (sine, triangle) to complex (a "wander" that oscillates around the
//! near end, travels to the far end and oscillates there, then returns).
//!
//! Each pattern is a function `pos(u) → 0..1` of a normalized phase `u`, mapped
//! into the zone. The task samples it every `tick_ms` and issues a point-to-
//! point move sized so the rod arrives by the next tick (same streaming
//! approach as HSP) — there is no deadman; pause is explicit.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::info;

use super::CycleControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

const SERVO_SETTLE: Duration = Duration::from_millis(50);

/// Display names of the patterns, simple → complex.
pub const PATTERN_NAMES: [&str; 11] = [
    "Sine",
    "Triangle",
    "Sawtooth",
    "Thrust & hold",
    "Tease",
    "Double-stroke",
    "Build",
    "Crescendo",
    "Weave",
    "Wander",
    "Plunge",
];

/// Base period (seconds) of one full cycle of each pattern — this is the
/// per-pattern "speed".
const PATTERN_PERIOD_S: [f32; 11] = [4.0, 4.0, 4.0, 5.0, 6.0, 5.0, 6.0, 8.0, 7.0, 16.0, 6.0];

pub const PATTERN_COUNT: u32 = 11;

/// Per-pattern user-adjustable playback parameters — a generic wrapper
/// applied uniformly around every pattern's `pos(u)` shape, rather than
/// bespoke knobs per pattern. Defaults reproduce the original fixed
/// behaviour exactly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CyclePatternParams {
    /// Playback rate multiplier (0.25–3.0). Effective period =
    /// `PATTERN_PERIOD_S[idx] / speed`.
    pub speed: f32,
    /// Amplitude multiplier (0.0–1.5) applied around the stroke's centre —
    /// "gentle ↔ hard". 1.0 reproduces the pattern's original swing.
    pub intensity: f32,
    /// How many times the base shape repeats before a pause ("strokes per
    /// loop"), 1–8.
    pub reps: u32,
    /// Rest duration (seconds, 0–10) holding position after `reps`
    /// repetitions complete, before the next loop starts.
    pub pause_s: f32,
}

impl Default for CyclePatternParams {
    fn default() -> Self {
        CyclePatternParams {
            speed: 1.0,
            intensity: 1.0,
            reps: 1,
            pause_s: 0.0,
        }
    }
}

/// Phase of the reps/pause burst state machine driving one pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BurstPhase {
    Stroking,
    Pausing { remaining_s: f32 },
}

fn smoothstep01(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    v * v * (3.0 - 2.0 * v)
}

/// Pattern position in `0..1` (0 = near end / start, 1 = far end) for phase
/// `u` in `[0,1)`.
fn pattern_pos(idx: usize, u: f32) -> f32 {
    let p = match idx {
        // Sine — smooth full-range.
        0 => 0.5 - 0.5 * (TAU * u).cos(),
        // Triangle — constant-speed strokes.
        1 => 1.0 - (2.0 * u - 1.0).abs(),
        // Sawtooth — slow extend, fast retract.
        2 => {
            if u < 0.75 {
                u / 0.75
            } else {
                1.0 - (u - 0.75) / 0.25
            }
        }
        // Thrust & hold — push to the far end, dwell, return, dwell.
        3 => {
            if u < 0.4 {
                smoothstep01(u / 0.4)
            } else if u < 0.5 {
                1.0
            } else if u < 0.9 {
                1.0 - smoothstep01((u - 0.5) / 0.4)
            } else {
                0.0
            }
        }
        // Tease — small, quick strokes hugging the near end.
        4 => 0.22 * (0.5 - 0.5 * (TAU * 3.0 * u).cos()),
        // Double-stroke — two short jabs then one long stroke.
        5 => {
            if u < 0.2 {
                0.4 * (0.5 - 0.5 * (TAU * (u / 0.2)).cos())
            } else if u < 0.4 {
                0.4 * (0.5 - 0.5 * (TAU * ((u - 0.2) / 0.2)).cos())
            } else {
                0.5 - 0.5 * (TAU * ((u - 0.4) / 0.6)).cos()
            }
        }
        // Build — frequency rises across the cycle (phase ∝ u²).
        6 => 0.5 - 0.5 * (TAU * (u * u) * 3.0).cos(),
        // Crescendo — pulses of growing amplitude.
        7 => u * (0.5 - 0.5 * (TAU * 5.0 * u).cos()),
        // Weave — sum of two detuned sines for a wandering motion.
        8 => 0.5 + 0.32 * (TAU * u).sin() + 0.16 * (TAU * 2.6 * u + 1.0).sin(),
        // Wander — oscillate near the start, travel to the far end, oscillate
        // there, travel back. One long meta-cycle.
        9 => {
            if u < 0.25 {
                let v = u / 0.25;
                0.1 - 0.1 * (TAU * 2.0 * v).cos() // [0, 0.2] around the near end
            } else if u < 0.5 {
                let v = (u - 0.25) / 0.25;
                0.1 + (0.9 - 0.1) * smoothstep01(v) // travel out
            } else if u < 0.75 {
                let v = (u - 0.5) / 0.25;
                0.9 - 0.1 * (TAU * 2.0 * v).cos() // [0.8, 1.0] around the far end
            } else {
                let v = (u - 0.75) / 0.25;
                0.9 - (0.9 - 0.1) * smoothstep01(v) // travel back
            }
        }
        // Plunge — fast strong thrust to the far end, then a slow retract.
        10 => {
            if u < 0.15 {
                u / 0.15
            } else {
                1.0 - (u - 0.15) / 0.85
            }
        }
        _ => 0.0,
    };
    p.clamp(0.0, 1.0)
}

/// Advance one tick of the reps/pause burst state machine wrapped around
/// `pattern_pos`. Pure and deterministic so it can be driven directly by
/// tests without any async/time machinery. Returns the updated `(u, rep,
/// burst)` and, if a stroke should be issued this tick, the intensity-
/// adjusted target position in `0..1` — `None` means hold the last position
/// (resting between bursts).
fn advance_burst(
    idx: usize,
    u: f32,
    rep: u32,
    burst: BurstPhase,
    params: CyclePatternParams,
    dt: f32,
) -> (f32, u32, BurstPhase, Option<f32>) {
    match burst {
        BurstPhase::Pausing { remaining_s } => {
            let remaining = remaining_s - dt;
            if remaining <= 0.0 {
                (0.0, 0, BurstPhase::Stroking, None)
            } else {
                (u, rep, BurstPhase::Pausing { remaining_s: remaining }, None)
            }
        }
        BurstPhase::Stroking => {
            let period = PATTERN_PERIOD_S[idx].max(0.1) / params.speed.max(0.05);
            let mut u = u + dt / period;
            let mut rep = rep;
            let mut burst = BurstPhase::Stroking;
            if u >= 1.0 {
                u -= 1.0;
                rep += 1;
                if rep >= params.reps.max(1) {
                    if params.pause_s > 0.0 {
                        burst = BurstPhase::Pausing { remaining_s: params.pause_s };
                    } else {
                        rep = 0;
                    }
                }
            }
            let raw = pattern_pos(idx, u);
            let p = (0.5 + (raw - 0.5) * params.intensity).clamp(0.0, 1.0);
            (u, rep, burst, Some(p))
        }
    }
}

pub struct CycleTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    stroke_mm: f32,
    zone_min: f32,
    zone_max: f32,
    max_velocity_mm_s: f32,
    accel_g: f32,
    profile: MotionProfile,
    tick: Duration,
    long_press: Duration,
}

impl CycleTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        let c = &cfg.actuator.cycle;
        CycleTask {
            state,
            cmd_tx,
            stroke_mm: cfg.stroke_mm(),
            zone_min: c.zone_min,
            zone_max: c.zone_max,
            max_velocity_mm_s: c
                .max_velocity_mm_s
                .min(cfg.actuator.limits.max_velocity_mm_s),
            accel_g: c.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
            tick: Duration::from_millis(c.tick_ms.max(20)),
            long_press: Duration::from_millis(c.long_press_ms.max(200)),
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<CycleControl>) {
        info!("cycle task running");
        loop {
            match ctrl_rx.recv().await {
                None => break,
                Some(CycleControl::Start) => self.play(&mut ctrl_rx).await,
                // Button/Stop while idle: nothing to do.
                Some(_) => {}
            }
        }
        info!("cycle task stopped");
    }

    async fn play(&self, rx: &mut mpsc::Receiver<CycleControl>) {
        // Energise and announce.
        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
        tokio::time::sleep(SERVO_SETTLE).await;
        let mut pattern: u32 = 0;
        let mut paused = false;
        let mut u: f32 = 0.0;
        let mut rep: u32 = 0;
        let mut burst = BurstPhase::Stroking;
        let mut last_target = self.state.read().await.position_mm;
        {
            let mut st = self.state.write().await;
            st.cycle.active = true;
            st.cycle.pattern = 0;
            st.cycle.paused = false;
            st.set_mode(AppMode::Cycle);
        }
        self.publish(pattern, paused).await;
        info!(pattern = PATTERN_NAMES[0], "cycle started");

        let dt = self.tick.as_secs_f32();
        let mut tick = interval(self.tick);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Pending button-press timing: deadline at which a held press becomes a
        // long press, and whether the long-press action already fired.
        let mut press_deadline: Option<Instant> = None;
        let mut long_fired = false;

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None | Some(CycleControl::Stop) => {
                        self.halt().await;
                        return;
                    }
                    Some(CycleControl::Start) => {
                        // Restart from the first pattern, running.
                        pattern = 0;
                        paused = false;
                        u = 0.0;
                        rep = 0;
                        burst = BurstPhase::Stroking;
                        last_target = self.state.read().await.position_mm;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        self.publish(pattern, paused).await;
                    }
                    Some(CycleControl::Button { down: true }) => {
                        press_deadline = Some(Instant::now() + self.long_press);
                        long_fired = false;
                    }
                    Some(CycleControl::Button { down: false }) => {
                        // A release that didn't reach long-press is a short press.
                        if !long_fired {
                            pattern = (pattern + 1) % PATTERN_COUNT;
                            u = 0.0;
                            rep = 0;
                            burst = BurstPhase::Stroking;
                            last_target = self.state.read().await.position_mm;
                            if paused {
                                paused = false;
                                self.energize().await;
                                last_target = self.state.read().await.position_mm;
                            }
                            self.publish(pattern, paused).await;
                            info!(pattern = PATTERN_NAMES[pattern as usize], "cycle: next pattern");
                        }
                        press_deadline = None;
                        long_fired = false;
                    }
                },
                _ = sleep_until_opt(press_deadline), if press_deadline.is_some() => {
                    // Held past the threshold → long press → toggle pause.
                    paused = !paused;
                    long_fired = true;
                    press_deadline = None;
                    if paused {
                        // Rest: stop, then drop the servo and let the brake hold
                        // the rod so the motor bears no load while paused.
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::Park).await;
                    } else {
                        self.energize().await;
                        last_target = self.state.read().await.position_mm;
                    }
                    self.publish(pattern, paused).await;
                    info!(paused, "cycle: long press");
                }
                _ = tick.tick(), if !paused => {
                    let params = self.state.read().await.cycle.params[pattern as usize];
                    let (new_u, new_rep, new_burst, out) =
                        advance_burst(pattern as usize, u, rep, burst, params, dt);
                    u = new_u;
                    rep = new_rep;
                    burst = new_burst;
                    if let Some(p) = out {
                        let rel = self.zone_min + p * (self.zone_max - self.zone_min);
                        let target = rel * self.stroke_mm;
                        let delta = target - last_target;
                        if delta.abs() > 0.05 {
                            let vel = (delta.abs() / dt).clamp(f32::MIN_POSITIVE, self.max_velocity_mm_s);
                            let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                pos_mm: target,
                                vel_mm_s: vel,
                                accel_g: self.accel_g,
                                profile: self.profile,
                                soften: false,
                            }).await;
                            last_target = target;
                        }
                    }
                }
            }
        }
    }

    /// Re-energise the servo and let it settle before the next move is issued
    /// (mirrors the startup delay in `play`).
    async fn energize(&self) {
        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
        tokio::time::sleep(SERVO_SETTLE).await;
    }

    async fn publish(&self, pattern: u32, paused: bool) {
        let mut st = self.state.write().await;
        st.cycle.pattern = pattern;
        st.cycle.paused = paused;
    }

    async fn halt(&self) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        // Rest with the brake holding the rod (automatic playback, not hand-held).
        let _ = self.cmd_tx.send(ActuatorCommand::Park).await;
        let mut st = self.state.write().await;
        st.cycle.active = false;
        st.cycle.paused = false;
        st.set_mode(AppMode::Idle);
        info!("cycle halted");
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending::<()>().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            [actuator.cycle]
            tick_ms = 80
            long_press_ms = 2000
        "#,
        )
        .unwrap()
    }

    #[test]
    fn patterns_stay_in_range_and_wander_visits_both_ends() {
        for idx in 0..PATTERN_COUNT as usize {
            for k in 0..100 {
                let p = pattern_pos(idx, k as f32 / 100.0);
                assert!((0.0..=1.0).contains(&p), "pattern {idx} out of range: {p}");
            }
        }
        // Wander (9) should reach near both ends across its cycle.
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for k in 0..200 {
            let p = pattern_pos(9, k as f32 / 200.0);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        assert!(lo < 0.1 && hi > 0.9, "wander range {lo}..{hi}");
    }

    #[test]
    fn default_params_reproduce_original_period_and_amplitude() {
        // speed=1, intensity=1, reps=1, pause_s=0 must be a no-op wrapper:
        // one full lap of pattern 0 (period 4.0s) with dt=0.5s (a power-of-two
        // fraction, so accumulation is exact in f32) takes exactly 8 ticks,
        // never pauses, and reports the raw (unscaled) shape.
        let params = CyclePatternParams::default();
        let (mut u, mut rep, mut burst) = (0.0_f32, 0_u32, BurstPhase::Stroking);
        let mut wraps = 0;
        for _ in 0..8 {
            let (nu, nrep, nburst, out) = advance_burst(0, u, rep, burst, params, 0.5);
            assert!(out.is_some(), "default params should never pause");
            assert_eq!(out.unwrap(), pattern_pos(0, nu), "intensity=1 must not rescale the raw shape");
            if nu < u {
                wraps += 1;
            }
            u = nu;
            rep = nrep;
            burst = nburst;
        }
        assert!(matches!(burst, BurstPhase::Stroking));
        assert_eq!(wraps, 1, "one full 4.0s period across 8 ticks of 0.5s should wrap exactly once");
        assert_eq!(u, 0.0, "should land exactly back at u=0");
        assert_eq!(rep, 0, "reps=1 with no pause resets rep on every wrap, not accumulate");
    }

    #[test]
    fn reps_and_pause_hold_position_between_bursts() {
        // period_eff = 4.0 / 4.0 = 1.0s = 8 ticks of 0.125s (power-of-two
        // fraction, exact in f32); reps=1 so a pause starts right after the
        // first full lap, and pause_s=1.0 is also exactly 8 ticks.
        let params = CyclePatternParams {
            speed: 4.0,
            intensity: 1.0,
            reps: 1,
            pause_s: 1.0,
        };
        let (mut u, mut rep, mut burst) = (0.0_f32, 0_u32, BurstPhase::Stroking);
        let mut stroked = 0;
        let mut paused = 0;
        // 24 ticks = stroke (8) + pause (8) + stroke again (8).
        for _ in 0..24 {
            let (nu, nrep, nburst, out) = advance_burst(0, u, rep, burst, params, 0.125);
            match out {
                Some(_) => stroked += 1,
                None => paused += 1,
            }
            u = nu;
            rep = nrep;
            burst = nburst;
        }
        assert_eq!(stroked, 16, "expected two 8-tick strokes across 24 ticks");
        assert_eq!(paused, 8, "expected one 8-tick pause across 24 ticks");
    }

    #[test]
    fn intensity_scales_amplitude_around_center() {
        // Triangle (idx 1) peaks at u=0.5 → raw 1.0; halved intensity should
        // land the reported position halfway between raw and the 0.5 center.
        let params = CyclePatternParams {
            speed: 1.0,
            intensity: 0.5,
            reps: 1,
            pause_s: 0.0,
        };
        // Start almost exactly at the peak (u=0.499) and take a tiny dt step
        // so the resulting u stays ~0.5.
        let (_, _, _, out) = advance_burst(1, 0.499, 0, BurstPhase::Stroking, params, 0.001);
        let p = out.expect("stroking tick should produce a position");
        assert!((p - 0.75).abs() < 0.01, "expected ~0.75 (halfway to 1.0), got {p}");
    }

    #[tokio::test(start_paused = true)]
    async fn short_press_advances_long_press_pauses() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, mut cmd_rx) = mpsc::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let h = tokio::spawn(CycleTask::new(state.clone(), cmd_tx, &cfg()).run(ctrl_rx));

        ctrl_tx.send(CycleControl::Start).await.unwrap();
        // Settle past the ServoOn delay before the task marks itself active.
        for _ in 0..3 {
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
            while cmd_rx.try_recv().is_ok() {}
        }
        assert!(state.read().await.cycle.active);
        assert_eq!(state.read().await.cycle.pattern, 0);

        // Short press: down then up well under 2s → next pattern.
        ctrl_tx
            .send(CycleControl::Button { down: true })
            .await
            .unwrap();
        tokio::time::advance(Duration::from_millis(200)).await;
        tokio::task::yield_now().await;
        ctrl_tx
            .send(CycleControl::Button { down: false })
            .await
            .unwrap();
        tokio::time::advance(Duration::from_millis(50)).await;
        tokio::task::yield_now().await;
        assert_eq!(state.read().await.cycle.pattern, 1);

        // Long press: hold past 2s → pause.
        ctrl_tx
            .send(CycleControl::Button { down: true })
            .await
            .unwrap();
        for _ in 0..25 {
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
            while cmd_rx.try_recv().is_ok() {}
        }
        assert!(state.read().await.cycle.paused, "long press should pause");

        ctrl_tx.send(CycleControl::Stop).await.unwrap();
        drop(ctrl_tx);
        let _ = h.await;
    }
}
