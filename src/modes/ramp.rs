//! Ramp — auto-ramp interactive program (SSCP extension, not part of Handy FW4).
//!
//! Press start and the rod oscillates on its own, *building* over time: both the
//! stroke speed and the stroke length climb from a gentle start to a configured
//! peak along an ease-in curve, then plateau. Unlike HAMP (constant speed) and
//! drill (deadman-held), the user only interacts occasionally:
//!
//!   * a **nudge** shifts the current intensity up or down a notch, and
//!   * every nudge (and the initial start) resets an **idle timeout** — if no
//!     input arrives within that window the program stops, drops the servo and
//!     returns to Idle.
//!
//! So hands-off the program runs for at most `idle_timeout`; nudging
//! occasionally both steers the intensity and keeps it alive. The reversal is
//! timer-driven (estimated travel time), exactly like [`HampTask`].
//!
//! [`HampTask`]: super::hamp::HampTask

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tracing::info;

use super::RampControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// After ServoOn the IAI controller needs ~50 ms before it reliably accepts a
/// move command (same settle used in peck-probe / drill).
const SERVO_SETTLE: Duration = Duration::from_millis(50);

/// Small margin added to each estimated travel time before the reversal, so a
/// move is never cut short by clock jitter (matches HAMP).
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);

pub struct RampTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    stroke_mm: f32,
    min_velocity_mm_s: f32,
    max_velocity_mm_s: f32,
    /// Time to climb from intensity 0 to 1 (before any nudges).
    ramp_duration: Duration,
    /// Auto-stop after this long with no Start/Nudge.
    idle_timeout: Duration,
    /// Relative bounds (0..1) of the *full-intensity* stroke zone.
    full_zone_min: f32,
    full_zone_max: f32,
    /// Fraction of the full span used at intensity 0 (so early strokes are short
    /// but not zero-length).
    min_span_frac: f32,
    /// Ease-in exponent for the time→intensity curve (1 = linear, >1 = slow
    /// start). Climb feels gentler at the top end for larger values.
    curve_exp: f32,
    accel_g: f32,
    profile: MotionProfile,
    /// Flag strokes for software jerk-limiting by the motion shaper.
    soften: bool,
}

impl RampTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        let r = &cfg.actuator.ramp;
        RampTask {
            state,
            cmd_tx,
            stroke_mm: cfg.stroke_mm(),
            min_velocity_mm_s: r.min_velocity_mm_s,
            max_velocity_mm_s: r.max_velocity_mm_s.min(cfg.actuator.limits.max_velocity_mm_s),
            ramp_duration: Duration::from_secs_f32(r.ramp_duration_s.max(0.1)),
            idle_timeout: Duration::from_secs_f32(r.idle_timeout_s.max(1.0)),
            full_zone_min: r.zone_min,
            full_zone_max: r.zone_max,
            min_span_frac: r.min_span_frac.clamp(0.0, 1.0),
            curve_exp: r.curve_exp.max(0.1),
            accel_g: r.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
            soften: cfg.actuator.softening.enable,
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<RampControl>) {
        info!("ramp task running");
        let mut running = false;
        // Program start (for the time-based climb) and the accumulated nudge
        // offset applied on top of the time curve.
        let mut start: Option<Instant> = None;
        let mut offset: f32 = 0.0;
        let mut out = false; // which zone end we travel toward next
        let mut next: Option<Instant> = None; // reversal deadline
        let mut idle: Option<Instant> = None; // auto-stop deadline
        let mut duration = self.ramp_duration;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(RampControl::Start { duration_s }) => {
                        if let Some(d) = duration_s {
                            duration = Duration::from_secs_f32(d.max(0.1));
                        }
                        // Ensure the servo is energised before the first stroke
                        // (we may be arriving from drill, which leaves it off).
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        tokio::time::sleep(SERVO_SETTLE).await;

                        running = true;
                        start = Some(Instant::now());
                        offset = 0.0;
                        out = false;
                        next = Some(Instant::now()); // fire the first stroke now
                        idle = Some(Instant::now() + self.idle_timeout);
                        {
                            let mut st = self.state.write().await;
                            st.ramp.active = true;
                            st.set_mode(AppMode::Ramp);
                        }
                        info!(secs = duration.as_secs_f32(), "ramp: started");
                    }

                    Some(RampControl::Nudge { delta }) => {
                        // A nudge steers intensity and keeps the program alive.
                        offset = (offset + delta).clamp(-1.0, 1.0);
                        if running {
                            idle = Some(Instant::now() + self.idle_timeout);
                            // Re-issue the current stroke so the new intensity
                            // (speed/zone) takes effect without waiting for the
                            // next reversal.
                            if let Some(s) = start {
                                let i = self.intensity(s.elapsed(), duration, offset);
                                self.stroke(i, out).await;
                                // Keep `out` pointing at the *next* end; stroke()
                                // already aimed at the current one.
                            }
                            info!(offset, "ramp: nudged");
                        }
                    }

                    Some(RampControl::Stop) => {
                        self.halt(&mut running, &mut next, &mut idle).await;
                        info!("ramp: stopped");
                    }
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    let Some(s) = start else { next = None; continue };
                    let i = self.intensity(s.elapsed(), duration, offset);
                    let travel = self.stroke(i, out).await;
                    out = !out;
                    next = Some(Instant::now() + travel);
                }

                _ = sleep_until_opt(idle), if idle.is_some() => {
                    // No input within the idle window — auto-stop.
                    self.halt(&mut running, &mut next, &mut idle).await;
                    info!("ramp: idle timeout, stopped");
                }
            }
        }

        info!("ramp task stopped");
    }

    /// Time- and nudge-derived intensity in `0..=1`.
    fn intensity(&self, elapsed: Duration, duration: Duration, offset: f32) -> f32 {
        let progress = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
        let eased = progress.powf(self.curve_exp);
        (eased + offset).clamp(0.0, 1.0)
    }

    /// Issue one stroke toward the current end at the given intensity, publish
    /// the derived runtime to `AppState`, and return the estimated travel time
    /// until the reversal should be scheduled.
    async fn stroke(&self, intensity: f32, out: bool) -> Duration {
        let velocity = self.min_velocity_mm_s
            + intensity * (self.max_velocity_mm_s - self.min_velocity_mm_s);
        let velocity = velocity.max(f32::MIN_POSITIVE);

        // Center-anchored zone that widens with intensity.
        let center = (self.full_zone_min + self.full_zone_max) / 2.0;
        let full_span = (self.full_zone_max - self.full_zone_min).abs();
        let span_frac = self.min_span_frac + intensity * (1.0 - self.min_span_frac);
        let half = (full_span * span_frac) / 2.0;
        let lo = center - half;
        let hi = center + half;

        let target_rel = if out { lo } else { hi };
        let target_mm = target_rel * self.stroke_mm;

        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: target_mm,
                vel_mm_s: velocity,
                accel_g: self.accel_g,
                profile: self.profile,
                soften: self.soften,
            })
            .await;

        {
            let mut st = self.state.write().await;
            st.ramp.intensity = intensity;
            st.ramp.velocity_mm_s = velocity;
            st.ramp.zone_min = lo;
            st.ramp.zone_max = hi;
        }

        // Estimated travel time across the current span at this speed.
        let span_mm = ((hi - lo) * self.stroke_mm).abs();
        let travel_ms = (span_mm / velocity) * 1000.0;
        Duration::from_millis(travel_ms.max(1.0) as u64) + REVERSAL_MARGIN
    }

    /// Decel-stop, drop the servo, clear runtime and return to Idle.
    async fn halt(
        &self,
        running: &mut bool,
        next: &mut Option<Instant>,
        idle: &mut Option<Instant>,
    ) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        // Come to rest with the brake holding the rod — ramp is automatic, the
        // rod isn't meant to be moved by hand here.
        let _ = self.cmd_tx.send(ActuatorCommand::Park).await;
        *running = false;
        *next = None;
        *idle = None;
        let mut st = self.state.write().await;
        st.ramp.active = false;
        st.set_mode(AppMode::Idle);
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d).await;
    } else {
        std::future::pending::<()>().await;
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
            default_accel_g = 0.3
            [actuator.ramp]
            min_velocity_mm_s = 40.0
            max_velocity_mm_s = 300.0
            ramp_duration_s = 10.0
            idle_timeout_s = 60.0
        "#,
        )
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn climbs_and_idle_stops() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, mut cmd_rx) = mpsc::channel(32);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let task = RampTask::new(state.clone(), cmd_tx, &cfg());
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(RampControl::Start { duration_s: None }).await.unwrap();

        // Start energises the servo, then fires the first stroke at intensity ~0.
        let first_servo = cmd_rx.recv().await.unwrap();
        assert_eq!(first_servo, ActuatorCommand::ServoOn(true));
        let first = cmd_rx.recv().await.unwrap();
        let v0 = match first {
            ActuatorCommand::MoveTo { vel_mm_s, .. } => vel_mm_s,
            o => panic!("{o:?}"),
        };
        // At t≈0 the velocity should be near the configured minimum.
        assert!(v0 < 60.0, "expected slow start, got {v0}");
        assert!(state.read().await.ramp.active);

        // Let the climb run and drain strokes; a later stroke must be faster.
        // Step the clock so the task is polled between the reversals it
        // reschedules (one big advance wouldn't let it run in between).
        let mut v_late = v0;
        for _ in 0..12 {
            tokio::time::advance(Duration::from_millis(900)).await;
            tokio::task::yield_now().await;
            while let Ok(c) = cmd_rx.try_recv() {
                if let ActuatorCommand::MoveTo { vel_mm_s, .. } = c {
                    v_late = vel_mm_s;
                }
            }
        }
        assert!(v_late > v0, "velocity should climb: {v0} -> {v_late}");

        // No input for the idle window → auto-stop parks the rod (servo off,
        // brake holding).
        let mut saw_park = false;
        for _ in 0..60 {
            tokio::time::advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;
            while let Ok(c) = cmd_rx.try_recv() {
                if c == ActuatorCommand::Park {
                    saw_park = true;
                }
            }
        }
        assert!(saw_park, "idle timeout should park the rod");
        assert!(!state.read().await.ramp.active);
        assert_eq!(state.read().await.mode, AppMode::Idle);

        drop(ctrl_tx);
        let _ = h.await;
    }
}
