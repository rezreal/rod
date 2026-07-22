//! Trace — steady oscillation with hand-switch lower-bound setting.
//!
//! Mirror of plumb.rs, but the user sets the LOWER (return) bound rather than
//! the upper bound. The ceiling is fixed at `work_origin + ceiling_depth_mm`
//! (or `max_mm` when `ceiling_depth_mm == 0`).
//!
//! State machine:
//!   * Oscillating — timed back-and-forth between `lower_mm` and `ceiling_mm`.
//!   * HandOff     — switch held → decel-stop → servo off → user pushes rod
//!     inward (to decreasing mm) to set a new return point.
//!   * Resuming    — switch released → capture `position_mm` as new `lower_mm`
//!     → servo on → wait SERVO_SETTLE → begin next stroke outward.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::TraceControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// After ServoOn the IAI controller needs ~50 ms before it reliably accepts
/// a move command.
const SERVO_SETTLE: Duration = Duration::from_millis(50);
/// How often the hand-switch state is sampled.
const SWITCH_TICK: Duration = Duration::from_millis(100);
/// Margin added to estimated travel time before a reversal.
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);
/// Below this stroke span (mm) the task holds still rather than micro-oscillating.
const MIN_STROKE_MM: f32 = 2.0;

pub struct TraceTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    /// Hard upper soft-limit in mm (from `cfg.max_position_mm()`).
    max_mm: f32,
    ceiling_depth_mm: f32,
    default_depth_mm: f32,
    speed_mm_s: f32,
    accel_g: f32,
    profile: MotionProfile,
}

impl TraceTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        TraceTask {
            state,
            cmd_tx,
            max_mm: cfg.max_position_mm(),
            ceiling_depth_mm: cfg.actuator.trace.ceiling_depth_mm,
            default_depth_mm: cfg.actuator.trace.default_depth_mm,
            speed_mm_s: cfg.actuator.trace.speed_mm_s,
            accel_g: cfg.actuator.trace.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
        }
    }

    /// Compute the fixed ceiling from work_origin and config.
    fn ceiling_mm(&self, work_origin: f32) -> f32 {
        if self.ceiling_depth_mm == 0.0 {
            self.max_mm
        } else {
            (work_origin + self.ceiling_depth_mm).min(self.max_mm)
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<TraceControl>) {
        info!("trace task running");
        let mut switch_ticker = tokio::time::interval(SWITCH_TICK);
        switch_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        // `out` is false when the next stroke goes toward ceiling_mm (outward),
        // true when it returns toward lower_mm.
        let mut out = false;
        let mut next: Option<Instant> = None;
        let mut work_origin;
        let mut ceiling_mm = 0.0f32;
        let mut handing_off = false;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(TraceControl::Start) => {
                        let (origin, hand_switch) = {
                            let st = self.state.read().await;
                            let o = match st.work_origin_mm {
                                Some(o) => o,
                                None => {
                                    warn!("trace: no calibration; work origin defaulting to 0 mm");
                                    0.0
                                }
                            };
                            (o, st.hand_switch)
                        };
                        work_origin = origin;
                        ceiling_mm = self.ceiling_mm(work_origin);
                        let default_lower = (ceiling_mm - self.default_depth_mm).max(0.0);
                        handing_off = hand_switch;
                        {
                            let mut st = self.state.write().await;
                            st.trace.active = true;
                            st.trace.lower_mm = default_lower;
                            st.trace.handing_off = handing_off;
                            st.set_mode(AppMode::Trace);
                        }
                        out = false; // first stroke goes outward toward ceiling
                        if handing_off {
                            // Button already held: release servo immediately.
                            let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                            next = None;
                        } else {
                            next = Some(Instant::now());
                        }
                        info!(work_origin, ceiling_mm, default_lower, "trace: started");
                    }

                    Some(TraceControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        if handing_off {
                            let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        }
                        {
                            let mut st = self.state.write().await;
                            st.trace.active = false;
                            st.trace.handing_off = false;
                            st.set_mode(AppMode::Idle);
                        }
                        out = false;
                        next = None;
                        handing_off = false;
                        info!("trace: stopped");
                    }
                },

                _ = switch_ticker.tick() => {
                    let active = self.state.read().await.trace.active;
                    if !active { continue; }

                    let (hand_switch, position_mm) = {
                        let st = self.state.read().await;
                        (st.hand_switch, st.position_mm)
                    };

                    if hand_switch && !handing_off {
                        // Button pressed: stop the rod and release the servo so
                        // the user can push the rod inward to set a new return point.
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        handing_off = true;
                        next = None;
                        self.state.write().await.trace.handing_off = true;
                        info!("trace: servo off; user repositioning lower bound");
                    } else if !hand_switch && handing_off {
                        // Button released: capture position as new lower bound.
                        // Clamp to [0, ceiling_mm - MIN_STROKE_MM].
                        let new_lower = position_mm.clamp(0.0, ceiling_mm - MIN_STROKE_MM);
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        {
                            let mut st = self.state.write().await;
                            st.trace.lower_mm = new_lower;
                            st.trace.handing_off = false;
                        }
                        handing_off = false;
                        out = false; // first stroke after resume goes to ceiling
                        next = Some(Instant::now() + SERVO_SETTLE);
                        info!(new_lower, "trace: lower bound updated; servo re-enabling");
                    }
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    let lower_mm = self.state.read().await.trace.lower_mm;
                    let span = ceiling_mm - lower_mm;
                    if span < MIN_STROKE_MM {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        next = None;
                        continue;
                    }
                    // out=false → going to ceiling (outward, increasing mm)
                    // out=true  → returning to lower_mm (inward, decreasing mm)
                    let pos = if out { lower_mm } else { ceiling_mm };
                    let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                        pos_mm: pos,
                        vel_mm_s: self.speed_mm_s,
                        accel_g: self.accel_g,
                        profile: self.profile,
                                    soften: false,
                    }).await;
                    let travel_ms = (span / self.speed_mm_s) * 1000.0;
                    let travel = Duration::from_millis(travel_ms.max(1.0) as u64) + REVERSAL_MARGIN;
                    out = !out;
                    next = Some(Instant::now() + travel);
                }
            }
        }

        info!("trace task stopped");
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

    fn cfg() -> crate::config::Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.trace]
            ceiling_depth_mm = 0.0
            default_depth_mm = 80.0
            speed_mm_s       = 40.0
            accel_g          = 0.15
        "#,
        )
        .unwrap()
    }

    fn make(
        cfg: &crate::config::Config,
    ) -> (
        TraceTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        (TraceTask::new(state.clone(), cmd_tx, cfg), state, cmd_rx)
    }

    /// Verify that the mode oscillates: first stroke goes to ceiling_mm,
    /// second stroke returns to lower_mm.
    ///
    /// With ceiling_depth_mm=0, ceiling = max_mm = 300 mm (12inch variant).
    /// default_depth_mm=80 → lower = 300 - 80 = 220 mm.
    /// speed=40 mm/s → span=80 mm → travel = 2000 ms + 10 ms margin.
    #[tokio::test(start_paused = true)]
    async fn oscillates_between_lower_and_ceiling() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TraceControl::Start).await.unwrap();

        // First stroke fires at Instant::now() — advance just past the select.
        tokio::time::advance(Duration::from_millis(1)).await;

        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                // ceiling = max_mm = 300 mm; first stroke goes outward (out=false).
                assert!(
                    (pos_mm - 300.0).abs() < 0.1,
                    "expected 300 mm (ceiling), got {pos_mm}"
                );
                assert!((vel_mm_s - 40.0).abs() < 0.1);
            }
            o => panic!("unexpected: {o:?}"),
        }

        // 80 mm @ 40 mm/s = 2000 ms + 10 ms margin → advance past reversal.
        tokio::time::advance(Duration::from_millis(2100)).await;

        let second = cmd_rx.recv().await.unwrap();
        match second {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                // lower_mm = 300 - 80 = 220 mm; second stroke returns (out=true).
                assert!(
                    (pos_mm - 220.0).abs() < 0.1,
                    "expected 220 mm (lower), got {pos_mm}"
                );
            }
            o => panic!("unexpected: {o:?}"),
        }

        ctrl_tx.send(TraceControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain Stop ActuatorCommand
        drop(ctrl_tx);
        let _ = h.await;
    }

    /// Verify that holding the hand switch stops the rod and drops the servo,
    /// and that releasing captures a new lower bound.
    ///
    /// Setup: work_origin=0, ceiling=300, lower starts at 220.
    /// User holds switch → Stop + ServoOn(false).
    /// User pushes rod to 180 mm inward, releases switch → ServoOn(true).
    /// After SERVO_SETTLE (50 ms) + margin → MoveTo(300) (first stroke to ceiling).
    #[tokio::test(start_paused = true)]
    async fn hand_switch_sets_new_lower() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TraceControl::Start).await.unwrap();

        // First stroke fires immediately.
        tokio::time::advance(Duration::from_millis(1)).await;
        let first = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(first, ActuatorCommand::MoveTo { .. }),
            "expected MoveTo, got {first:?}"
        );

        // Button pressed → next switch tick stops the rod and drops the servo.
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;

        let stop = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(stop, ActuatorCommand::Stop),
            "expected Stop, got {stop:?}"
        );
        let servo_off = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(servo_off, ActuatorCommand::ServoOn(false)),
            "expected ServoOn(false), got {servo_off:?}"
        );

        // User pushes rod inward to 180 mm, then releases the button.
        state.write().await.position_mm = 180.0;
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;

        let servo_on = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(servo_on, ActuatorCommand::ServoOn(true)),
            "expected ServoOn(true), got {servo_on:?}"
        );

        // After SERVO_SETTLE (50 ms) the first stroke goes outward to ceiling (300 mm).
        tokio::time::advance(Duration::from_millis(60)).await;
        let resume = cmd_rx.recv().await.unwrap();
        match resume {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!(
                    (pos_mm - 300.0).abs() < 0.1,
                    "expected 300 mm (ceiling) after new lower set, got {pos_mm}"
                );
            }
            o => panic!("unexpected after servo-on: {o:?}"),
        }

        assert!(
            (state.read().await.trace.lower_mm - 180.0).abs() < 0.1,
            "lower_mm not updated in state"
        );

        ctrl_tx.send(TraceControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await;
        drop(ctrl_tx);
        let _ = h.await;
    }
}
