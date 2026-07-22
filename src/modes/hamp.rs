//! HAMP — software oscillation (SPEC §7.2).
//!
//! Maps onto knock-rod's `oscillate(speed, lower, upper, accel)`, whose
//! algorithm is **timer-driven, not PEND-gated**: the task fires a single move
//! to one end of the zone, then schedules the reversal for the *estimated*
//! arrival time (distance ÷ speed) plus a small margin. This avoids blocking on
//! the controller's position-complete bit and keeps the motion smooth.
//!
//! The desired oscillation parameters live in `AppState.hamp`; the dispatcher
//! writes them and sends a [`HampControl`] message to (re)evaluate.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tracing::info;

use super::HampControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppState};
use crate::telemetry::metrics;

/// Small margin added to each estimated travel time before the reversal, so a
/// move is never cut short by clock jitter (knock-rod uses 10 ms).
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);

pub struct HampTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    stroke_mm: f32,
    max_velocity_mm_s: f32,
    accel_g: f32,
    profile: MotionProfile,
    /// Flag strokes for software jerk-limiting by the motion shaper.
    soften: bool,
}

impl HampTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        HampTask {
            state,
            cmd_tx,
            stroke_mm: cfg.stroke_mm(),
            max_velocity_mm_s: cfg.actuator.limits.max_velocity_mm_s,
            accel_g: cfg.actuator.limits.default_accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
            soften: cfg.actuator.softening.enable,
        }
    }

    /// Run the oscillation loop until the control channel closes.
    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<HampControl>) {
        info!("hamp task running");
        // `out` toggles which end we travel toward; `next` is the next reversal.
        let mut out = false;
        let mut next: Option<Instant> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,
                    Some(HampControl::Stop) => next = None,
                    // Start / Update: (re)trigger immediately if we should be running.
                    Some(_) => {
                        if self.should_run().await {
                            next = Some(Instant::now());
                        } else {
                            next = None;
                        }
                    }
                },
                _ = sleep_until_opt(next), if next.is_some() => {
                    match self.stroke(&mut out).await {
                        Some(d) => next = Some(Instant::now() + d),
                        None => next = None, // stopped or velocity 0
                    }
                }
            }
        }
        info!("hamp task stopped");
    }

    async fn should_run(&self) -> bool {
        let st = self.state.read().await;
        st.hamp.running && st.hamp.velocity > 0.0
    }

    /// Issue one stroke toward the current end and return the estimated time
    /// until the reversal should be scheduled. Returns `None` if oscillation is
    /// no longer active.
    async fn stroke(&self, out: &mut bool) -> Option<Duration> {
        let (velocity, min, max, softness) = {
            let st = self.state.read().await;
            if !st.hamp.running || st.hamp.velocity <= 0.0 {
                return None;
            }
            (st.hamp.velocity, st.hamp.min, st.hamp.max, st.hamp.softness)
        };
        // softness 0 → full configured accel; softness 1 → 10 % of it.
        let effective_accel_g = self.accel_g * (1.0 - softness.clamp(0.0, 1.0) * 0.9);

        let target_rel = if *out { min } else { max };
        let target_mm = target_rel * self.stroke_mm;
        let speed_mm_s = (velocity * self.max_velocity_mm_s).max(f32::MIN_POSITIVE);

        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: target_mm,
                vel_mm_s: speed_mm_s,
                accel_g: effective_accel_g,
                profile: self.profile,
                soften: self.soften,
            })
            .await;

        // Estimated travel time for the full zone span at this speed.
        let span_mm = ((max - min) * self.stroke_mm).abs();
        let travel_ms = (span_mm / speed_mm_s) * 1000.0;
        // travel_ms is in **milliseconds**; use from_millis, not from_secs_f32.
        // (from_secs_f32(1600.0) = 26 minutes; from_millis(1600) = 1.6 seconds.)
        let travel = Duration::from_millis(travel_ms.max(1.0) as u64) + REVERSAL_MARGIN;

        // Record telemetry and update the direction flag in state.
        metrics::hamp_stroke(*out, travel_ms as f64);
        {
            let mut st = self.state.write().await;
            st.hamp.direction = *out;
        }
        *out = !*out;
        Some(travel)
    }
}

/// `sleep_until` for an optional deadline. Only constructed when the `select!`
/// precondition guarantees `Some`, so the `unwrap` is safe.
async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d).await;
    } else {
        // Never resolves; the `if next.is_some()` guard prevents reaching here.
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
        "#,
        )
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn oscillates_between_zone_ends() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        {
            let mut st = state.write().await;
            st.hamp.running = true;
            st.hamp.velocity = 0.5; // -> 200 mm/s
            st.hamp.min = 0.0;
            st.hamp.max = 1.0; // full 300 mm span
        }
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let task = HampTask::new(state.clone(), cmd_tx, &cfg());
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(HampControl::Start).await.unwrap();

        // First stroke fires ~immediately, toward max (out=false) => 300 mm.
        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert_eq!(pos_mm, 300.0);
                assert_eq!(vel_mm_s, 200.0);
            }
            o => panic!("{o:?}"),
        }

        // 300 mm at 200 mm/s = 1500 ms travel + 10 ms margin. Advance past it.
        tokio::time::advance(Duration::from_millis(1600)).await;
        let second = cmd_rx.recv().await.unwrap();
        match second {
            ActuatorCommand::MoveTo { pos_mm, .. } => assert_eq!(pos_mm, 0.0), // toward min
            o => panic!("{o:?}"),
        }

        // Stop halts further strokes.
        {
            state.write().await.hamp.running = false;
        }
        ctrl_tx.send(HampControl::Stop).await.unwrap();
        tokio::time::advance(Duration::from_millis(3000)).await;
        // Drain any in-flight command, then expect silence.
        let _ = cmd_rx.try_recv();
        assert!(cmd_rx.try_recv().is_err());

        drop(ctrl_tx);
        let _ = h.await;
    }
}
