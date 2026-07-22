//! HAMP — software oscillation (SPEC §7.2).
//!
//! Maps onto knock-rod's `oscillate(speed, lower, upper, accel)`. Each stroke
//! is followed by a reversal once the controller reports the move **settled**
//! (`DSSE.MOVE` clear and `DSS1.PEND` set), not by guessing the travel time
//! from distance ÷ speed. That guess used to be the only signal: it assumes
//! the actuator is instantly at full commanded speed for the whole stroke,
//! ignoring acceleration entirely. `softness` scales the commanded
//! acceleration down to as little as 10 % of the configured value (see
//! `stroke`), and — if enabled — the software launch-ramp shaper
//! (`crate::shaper`) adds a further fixed-duration ramp on top; neither is
//! visible to a pure distance/speed estimate. The real travel time can end up
//! arbitrarily larger than the guess, which caused the reversal to fire long
//! before the actuator got anywhere near the target: a "wiggle" near the
//! centre of the zone at high speed/softness (the estimate is off by the
//! most there), or a visible stutter under the shaper's fixed launch ramp at
//! low speed (where that fixed ramp eats a large fraction of a short
//! stroke).
//!
//! The distance/speed estimate is now only a padded fallback deadline, in
//! case the controller never reports completion (e.g. moves are silently
//! suppressed while alarmed, see `driver::execute`) — normal operation is
//! entirely driven by real status feedback.
//!
//! The desired oscillation parameters live in `AppState.hamp`; the dispatcher
//! writes them and sends a [`HampControl`] message to (re)evaluate.
//!
//! The zone (`min`/`max`, 0..1) is relative to the calibrated work origin
//! (`AppState.work_origin_mm`, defaulting to 0 mm if uncalibrated), not the
//! physical zero — 0.0 is the origin, 1.0 the far end of the stroke.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tracing::info;

use super::HampControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppState};
use crate::telemetry::metrics;

/// Cadence for re-checking the controller's motion status while a stroke is
/// in flight. Finer than the driver's own ~80 ms status-poll interval just
/// re-reads a value that hasn't changed yet, which is harmless.
const SETTLE_POLL: Duration = Duration::from_millis(20);

/// Minimum time to wait after issuing a move before trusting the "settled"
/// status bits. `is_moving` flips true synchronously when the driver issues
/// the Modbus write, but `positioning_done` (DSS1.PEND) is only cleared on
/// the driver's *next* status poll — so right after sending a new stroke,
/// AppState can still show the *previous* stroke's stale "settled" reading.
/// Without this grace period that stale reading would be mistaken for the
/// new stroke completing instantly.
const SETTLE_GRACE: Duration = Duration::from_millis(50);

/// Multiplier and floor applied to the naive distance/speed estimate to get
/// a fallback deadline. It only needs to be generous, not accurate: real
/// completion is normally detected from live controller status long before
/// this fires.
const SAFETY_FACTOR: u32 = 4;
const SAFETY_FLOOR: Duration = Duration::from_secs(1);
const SAFETY_CAP: Duration = Duration::from_secs(15);

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

/// Fallback wait window for an in-flight stroke.
#[derive(Clone, Copy)]
struct SettleWindow {
    /// Don't trust status bits before this instant (see `SETTLE_GRACE`).
    not_before: Instant,
    /// Give up waiting for real completion and reverse anyway at this instant.
    deadline: Instant,
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
        // `out` toggles which end we travel toward next.
        let mut out = false;
        let mut settle_by: Option<SettleWindow> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,
                    Some(HampControl::Stop) => settle_by = None,
                    // Start / Update: (re)trigger immediately if we should be running.
                    Some(_) => settle_by = self.stroke(&mut out).await,
                },
                _ = self.await_settled(settle_by), if settle_by.is_some() => {
                    settle_by = self.stroke(&mut out).await;
                }
            }
        }
        info!("hamp task stopped");
    }

    /// Wait for the controller to report the in-flight stroke settled, or for
    /// `window.deadline` to pass, whichever comes first.
    async fn await_settled(&self, window: Option<SettleWindow>) {
        let Some(window) = window else {
            // The `if settle_by.is_some()` guard on the select! branch means
            // this is never actually polled when `window` is `None`; pend
            // defensively rather than relying on that.
            std::future::pending::<()>().await;
            return;
        };
        tokio::time::sleep_until(window.not_before).await;
        loop {
            {
                let st = self.state.read().await;
                if !st.is_moving && st.positioning_done {
                    return;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(SETTLE_POLL) => {}
                _ = tokio::time::sleep_until(window.deadline) => return,
            }
        }
    }

    /// Issue one stroke toward the current end and return the fallback
    /// settle window. Returns `None` if oscillation is no longer active.
    async fn stroke(&self, out: &mut bool) -> Option<SettleWindow> {
        let (velocity, min, max, softness, origin) = {
            let st = self.state.read().await;
            if !st.hamp.running || st.hamp.velocity <= 0.0 {
                return None;
            }
            (
                st.hamp.velocity,
                st.hamp.min,
                st.hamp.max,
                st.hamp.softness,
                st.work_origin_mm.unwrap_or(0.0),
            )
        };
        // softness 0 → full configured accel; softness 1 → 10 % of it.
        let effective_accel_g = self.accel_g * (1.0 - softness.clamp(0.0, 1.0) * 0.9);

        // Zone bounds are relative to the calibrated origin, not the physical
        // zero: 0.0 is the origin itself, 1.0 the far end of the stroke (the
        // comfortable-depth ceiling further compresses this downstream — see
        // `driver::depth_scaled`).
        let span_mm = (self.stroke_mm - origin).max(0.0);
        let target_rel = if *out { min } else { max };
        let target_mm = origin + target_rel * span_mm;
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
        let sent_at = Instant::now();

        // Naive constant-velocity estimate — deliberately *not* used to
        // schedule the reversal (see module docs). Only feeds the padded
        // fallback deadline below.
        let zone_span_mm = ((max - min) * span_mm).abs();
        let estimate_ms = ((zone_span_mm / speed_mm_s) * 1000.0).max(1.0).min(u32::MAX as f32);
        metrics::hamp_stroke(*out, estimate_ms as f64);

        let padded =
            Duration::from_millis(estimate_ms as u64).saturating_mul(SAFETY_FACTOR) + SAFETY_FLOOR;
        let deadline = sent_at + padded.min(SAFETY_CAP);

        {
            let mut st = self.state.write().await;
            st.hamp.direction = *out;
        }
        *out = !*out;
        Some(SettleWindow {
            not_before: sent_at + SETTLE_GRACE,
            deadline,
        })
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
    async fn reverses_only_after_controller_reports_settled() {
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

        // Simulate the driver picking up the move (`is_moving` flips
        // synchronously; `positioning_done` starts false by default).
        {
            let mut st = state.write().await;
            st.is_moving = true;
        }

        // The naive distance/speed guess (1500 ms) would have fired a
        // reversal here under the old timer-driven design. It must not:
        // the controller hasn't reported the move settled yet.
        tokio::time::advance(Duration::from_millis(1600)).await;
        assert!(
            cmd_rx.try_recv().is_err(),
            "must not reverse before the controller reports settled"
        );

        // Now the controller reports the move settled.
        {
            let mut st = state.write().await;
            st.is_moving = false;
            st.positioning_done = true;
        }
        tokio::time::advance(SETTLE_POLL + Duration::from_millis(5)).await;
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
        {
            let mut st = state.write().await;
            st.is_moving = true;
            st.positioning_done = false;
        }
        tokio::time::advance(Duration::from_secs(3)).await;
        // Drain any in-flight command, then expect silence.
        let _ = cmd_rx.try_recv();
        assert!(cmd_rx.try_recv().is_err());

        drop(ctrl_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn zone_is_relative_to_calibrated_origin() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(50.0); // calibrated 50 mm into the 300 mm stroke
            st.hamp.running = true;
            st.hamp.velocity = 0.5; // -> 200 mm/s
            st.hamp.min = 0.0; // origin
            st.hamp.max = 1.0; // far end of stroke
        }
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let task = HampTask::new(state.clone(), cmd_tx, &cfg());
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(HampControl::Start).await.unwrap();

        // First stroke goes toward max => origin + 1.0 * (300 - 50) = 300 mm.
        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo { pos_mm, .. } => assert_eq!(pos_mm, 300.0),
            o => panic!("{o:?}"),
        }

        // Simulate the controller settling on that move.
        {
            let mut st = state.write().await;
            st.is_moving = false;
            st.positioning_done = true;
        }
        tokio::time::advance(SETTLE_GRACE + Duration::from_millis(5)).await;
        let second = cmd_rx.recv().await.unwrap();
        match second {
            // Toward min => origin + 0.0 * (300 - 50) = 50 mm (the calibrated origin).
            ActuatorCommand::MoveTo { pos_mm, .. } => assert_eq!(pos_mm, 50.0),
            o => panic!("{o:?}"),
        }

        drop(ctrl_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn falls_back_to_deadline_if_controller_never_reports_settled() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        {
            let mut st = state.write().await;
            st.hamp.running = true;
            st.hamp.velocity = 0.5; // -> 200 mm/s
            st.hamp.min = 0.0;
            st.hamp.max = 1.0; // full 300 mm span, 1500 ms naive estimate
        }
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let task = HampTask::new(state.clone(), cmd_tx, &cfg());
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(HampControl::Start).await.unwrap();
        let _ = cmd_rx.recv().await.unwrap();

        // Controller status never updates (e.g. suppressed while alarmed).
        // 1500 ms estimate * SAFETY_FACTOR(4) + SAFETY_FLOOR(1s) = 7s.
        tokio::time::advance(Duration::from_millis(6900)).await;
        assert!(cmd_rx.try_recv().is_err(), "deadline hasn't passed yet");
        tokio::time::advance(Duration::from_millis(200)).await;
        let second = cmd_rx.recv().await.unwrap();
        assert!(matches!(second, ActuatorCommand::MoveTo { .. }));

        drop(ctrl_tx);
        let _ = h.await;
    }
}
