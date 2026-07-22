//! Plumb — steady oscillation with hand-switch depth-setting.
//!
//! Oscillates at a fixed speed between the calibrated work-piece origin and a
//! configurable target depth. While the hand switch is held the servo is
//! released so the user can physically reposition the rod; on release the
//! current position is captured as the new upper bound and oscillation resumes.
//!
//! State machine:
//!   * Oscillating — timed back-and-forth between `work_origin_mm` and `target_mm`.
//!   * HandOff     — switch held → decel-stop → servo off → user moves rod freely.
//!   * Resuming    — switch released → capture `position_mm` as new `target_mm`
//!     → servo on → wait SERVO_SETTLE → begin next stroke.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::PlumbControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// After ServoOn the IAI controller needs ~50 ms before it reliably accepts
/// a move command (same settle used in drill.rs and peck-probe).
const SERVO_SETTLE: Duration = Duration::from_millis(50);
/// How often the hand-switch state is sampled.
const SWITCH_TICK: Duration = Duration::from_millis(100);
/// Margin added to estimated travel time before a reversal.
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);
/// Below this stroke span (mm) the task holds still rather than micro-oscillating.
const MIN_STROKE_MM: f32 = 2.0;

pub struct PlumbTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    max_mm: f32,
    speed_mm_s: f32,
    default_depth_mm: f32,
    accel_g: f32,
    profile: MotionProfile,
}

impl PlumbTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        PlumbTask {
            state,
            cmd_tx,
            max_mm: cfg.max_position_mm(),
            speed_mm_s: cfg.actuator.plumb.speed_mm_s,
            default_depth_mm: cfg.actuator.plumb.default_depth_mm,
            accel_g: cfg.actuator.plumb.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<PlumbControl>) {
        info!("plumb task running");
        let mut switch_ticker = tokio::time::interval(SWITCH_TICK);
        switch_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        // `out` is false when the next stroke goes toward target_mm (outward),
        // true when it returns toward work_origin_mm.
        let mut out = false;
        let mut next: Option<Instant> = None;
        let mut work_origin = 0.0f32;
        let mut handing_off = false;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(PlumbControl::Start) => {
                        let (origin, hand_switch) = {
                            let st = self.state.read().await;
                            let o = match st.work_origin_mm {
                                Some(o) => o,
                                None => {
                                    warn!("plumb: no calibration; work origin defaulting to 0 mm");
                                    0.0
                                }
                            };
                            (o, st.hand_switch)
                        };
                        work_origin = origin;
                        let default_target = (work_origin + self.default_depth_mm).min(self.max_mm);
                        handing_off = hand_switch;
                        {
                            let mut st = self.state.write().await;
                            st.plumb.active = true;
                            st.plumb.target_mm = default_target;
                            st.plumb.handing_off = handing_off;
                            st.set_mode(AppMode::Plumb);
                        }
                        out = false;
                        if handing_off {
                            // Button already held: release servo immediately.
                            let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                            next = None;
                        } else {
                            next = Some(Instant::now());
                        }
                        info!(work_origin, default_target, "plumb: started");
                    }

                    Some(PlumbControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        if handing_off {
                            let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        }
                        {
                            let mut st = self.state.write().await;
                            st.plumb.active = false;
                            st.plumb.handing_off = false;
                            st.set_mode(AppMode::Idle);
                        }
                        out = false;
                        next = None;
                        handing_off = false;
                        info!("plumb: stopped");
                    }
                },

                _ = switch_ticker.tick() => {
                    let active = self.state.read().await.plumb.active;
                    if !active { continue; }

                    let (hand_switch, position_mm) = {
                        let st = self.state.read().await;
                        (st.hand_switch, st.position_mm)
                    };

                    if hand_switch && !handing_off {
                        // Button pressed: stop the rod and release the servo.
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        handing_off = true;
                        next = None;
                        self.state.write().await.plumb.handing_off = true;
                        info!("plumb: servo off; user repositioning");
                    } else if !hand_switch && handing_off {
                        // Button released: capture position as new upper bound.
                        let new_target = position_mm.clamp(work_origin + MIN_STROKE_MM, self.max_mm);
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        {
                            let mut st = self.state.write().await;
                            st.plumb.target_mm = new_target;
                            st.plumb.handing_off = false;
                        }
                        handing_off = false;
                        out = false; // first stroke goes outward toward the new target
                        next = Some(Instant::now() + SERVO_SETTLE);
                        info!(new_target, "plumb: target updated; servo re-enabling");
                    }
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    let target_mm = self.state.read().await.plumb.target_mm;
                    let span = target_mm - work_origin;
                    if span < MIN_STROKE_MM {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        next = None;
                        continue;
                    }
                    let pos = if out { work_origin } else { target_mm };
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

        info!("plumb task stopped");
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
            [actuator.plumb]
            speed_mm_s   = 50.0
            default_depth_mm = 100.0
            accel_g      = 0.15
        "#,
        )
        .unwrap()
    }

    fn make(
        cfg: &crate::config::Config,
    ) -> (
        PlumbTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        (PlumbTask::new(state.clone(), cmd_tx, cfg), state, cmd_rx)
    }

    #[tokio::test(start_paused = true)]
    async fn oscillates_between_origin_and_default_target() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(20.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(PlumbControl::Start).await.unwrap();

        // First stroke fires at Instant::now() — advance just past the select.
        tokio::time::advance(Duration::from_millis(1)).await;

        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                // target = 20 + 100 = 120 mm; first stroke goes outward.
                assert!(
                    (pos_mm - 120.0).abs() < 0.1,
                    "expected 120 mm, got {pos_mm}"
                );
                assert!((vel_mm_s - 50.0).abs() < 0.1);
            }
            o => panic!("unexpected: {o:?}"),
        }

        // 100 mm @ 50 mm/s = 2000 ms + 10 ms margin → advance past reversal.
        tokio::time::advance(Duration::from_millis(2100)).await;
        let second = cmd_rx.recv().await.unwrap();
        match second {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!(
                    (pos_mm - 20.0).abs() < 0.1,
                    "expected 20 mm (origin), got {pos_mm}"
                );
            }
            o => panic!("unexpected: {o:?}"),
        }

        ctrl_tx.send(PlumbControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain Stop ActuatorCommand
        drop(ctrl_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn hand_switch_drops_servo_and_captures_new_target() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(PlumbControl::Start).await.unwrap();

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

        // User moves rod to 150 mm and releases button.
        state.write().await.position_mm = 150.0;
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;

        let servo_on = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(servo_on, ActuatorCommand::ServoOn(true)),
            "expected ServoOn(true), got {servo_on:?}"
        );

        // After SERVO_SETTLE (50 ms) the first outward stroke goes to 150 mm.
        tokio::time::advance(Duration::from_millis(60)).await;
        let resume = cmd_rx.recv().await.unwrap();
        match resume {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!(
                    (pos_mm - 150.0).abs() < 0.1,
                    "expected 150 mm, got {pos_mm}"
                );
            }
            o => panic!("unexpected after servo-on: {o:?}"),
        }

        assert!(
            (state.read().await.plumb.target_mm - 150.0).abs() < 0.1,
            "target_mm not updated in state"
        );

        ctrl_tx.send(PlumbControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await;
        drop(ctrl_tx);
        let _ = h.await;
    }
}
