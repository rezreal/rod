//! Tide — steady oscillation with hand-switch speed control.
//!
//! Oscillates at a fixed speed between the calibrated work-piece origin and a
//! configured target depth. While the hand switch is held the speed decreases
//! at a configured rate, floored at `min_speed_mm_s`. When the switch is
//! released the speed climbs back up at the same rate, capped at
//! `max_speed_mm_s`.
//!
//! Speed is re-evaluated on every `SPEED_TICK` (100 ms); each stroke uses the
//! speed current at the moment of reversal. Oscillation starts immediately when
//! `TideControl::Start` is received.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::TideControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// How often the speed is re-evaluated and the hand-switch state is sampled.
const SPEED_TICK: Duration = Duration::from_millis(100);
/// Margin added to estimated travel time before a reversal.
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);
/// Below this stroke span (mm) the task holds still rather than micro-oscillating.
const MIN_STROKE_MM: f32 = 2.0;

pub struct TideTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    /// Hard upper soft-limit in mm (from `cfg.max_position_mm()`).
    max_mm: f32,
    min_speed_mm_s: f32,
    max_speed_mm_s: f32,
    default_depth_mm: f32,
    speed_adjust_rate: f32,
    accel_g: f32,
    profile: MotionProfile,
}

impl TideTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        TideTask {
            state,
            cmd_tx,
            max_mm: cfg.max_position_mm(),
            min_speed_mm_s: cfg.actuator.tide.min_speed_mm_s,
            max_speed_mm_s: cfg.actuator.tide.max_speed_mm_s,
            default_depth_mm: cfg.actuator.tide.default_depth_mm,
            speed_adjust_rate: cfg.actuator.tide.speed_adjust_rate,
            accel_g: cfg.actuator.tide.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<TideControl>) {
        info!("tide task running");
        let mut speed_ticker = tokio::time::interval(SPEED_TICK);
        speed_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        // `out` is false when the next stroke goes toward target_mm (outward),
        // true when it returns toward work_origin_mm.
        let mut out = false;
        let mut next: Option<Instant> = None;
        let mut work_origin = 0.0f32;
        let mut target_mm = 0.0f32;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(TideControl::Start) => {
                        let origin = {
                            let st = self.state.read().await;
                            match st.work_origin_mm {
                                Some(o) => o,
                                None => {
                                    warn!("tide: no calibration; work origin defaulting to 0 mm");
                                    0.0
                                }
                            }
                        };
                        work_origin = origin;
                        target_mm = (work_origin + self.default_depth_mm).min(self.max_mm);
                        {
                            let mut st = self.state.write().await;
                            st.tide.active = true;
                            st.tide.speed_mm_s = self.max_speed_mm_s;
                            st.tide.target_mm = target_mm;
                            st.set_mode(AppMode::Tide);
                        }
                        out = false;
                        next = Some(Instant::now());
                        info!(work_origin, target_mm, "tide: started");
                    }

                    Some(TideControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        {
                            let mut st = self.state.write().await;
                            st.tide.active = false;
                            st.set_mode(AppMode::Idle);
                        }
                        out = false;
                        next = None;
                        info!("tide: stopped");
                    }
                },

                _ = speed_ticker.tick() => {
                    let active = self.state.read().await.tide.active;
                    if !active { continue; }

                    let (current_speed, held) = {
                        let st = self.state.read().await;
                        (st.tide.speed_mm_s, st.hand_switch)
                    };
                    let dt = SPEED_TICK.as_secs_f32();
                    let new_speed = if held {
                        (current_speed - dt * self.speed_adjust_rate).max(self.min_speed_mm_s)
                    } else {
                        (current_speed + dt * self.speed_adjust_rate).min(self.max_speed_mm_s)
                    };
                    self.state.write().await.tide.speed_mm_s = new_speed;
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    let (speed, active) = {
                        let st = self.state.read().await;
                        (st.tide.speed_mm_s, st.tide.active)
                    };
                    if !active {
                        next = None;
                        continue;
                    }
                    let span = target_mm - work_origin;
                    if span < MIN_STROKE_MM {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        next = None;
                        continue;
                    }
                    let pos = if out { work_origin } else { target_mm };
                    let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                        pos_mm: pos,
                        vel_mm_s: speed,
                        accel_g: self.accel_g,
                        profile: self.profile,
                                    soften: false,
                    }).await;
                    let travel_ms = (span / speed) * 1000.0;
                    let travel = Duration::from_millis(travel_ms.max(1.0) as u64) + REVERSAL_MARGIN;
                    out = !out;
                    next = Some(Instant::now() + travel);
                }
            }
        }

        info!("tide task stopped");
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

    fn cfg_with(
        min_speed: f32,
        max_speed: f32,
        default_depth: f32,
        speed_adjust_rate: f32,
    ) -> crate::config::Config {
        toml::from_str(&format!(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.tide]
            min_speed_mm_s     = {min_speed}
            max_speed_mm_s     = {max_speed}
            default_depth_mm   = {default_depth}
            speed_adjust_rate  = {speed_adjust_rate}
            accel_g            = 0.15
            "#,
            min_speed = min_speed,
            max_speed = max_speed,
            default_depth = default_depth,
            speed_adjust_rate = speed_adjust_rate,
        ))
        .unwrap()
    }

    fn make(
        cfg: &crate::config::Config,
    ) -> (
        TideTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        (TideTask::new(state.clone(), cmd_tx, cfg), state, cmd_rx)
    }

    /// With no button held, the first stroke goes to `target_mm` at
    /// `max_speed_mm_s` (speed starts at max and is never adjusted downward
    /// because the switch is not held).
    #[tokio::test(start_paused = true)]
    async fn oscillates_at_max_speed_by_default() {
        let cfg = cfg_with(10.0, 80.0, 100.0, 20.0);
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(20.0);
            st.hand_switch = false;
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TideControl::Start).await.unwrap();

        // next = Instant::now() on Start, so advancing by 1ms fires the stroke arm.
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
                // Speed at start is max_speed_mm_s = 80.
                assert!(
                    (vel_mm_s - 80.0).abs() < 0.1,
                    "expected 80 mm/s, got {vel_mm_s}"
                );
            }
            o => panic!("unexpected command: {o:?}"),
        }

        // Verify state reflects max speed.
        assert!(
            (state.read().await.tide.speed_mm_s - 80.0).abs() < 0.1,
            "state.tide.speed_mm_s should be 80"
        );
        assert!(
            (state.read().await.tide.target_mm - 120.0).abs() < 0.1,
            "state.tide.target_mm should be 120"
        );

        ctrl_tx.send(TideControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain Stop ActuatorCommand
        drop(ctrl_tx);
        let _ = h.await;
    }

    /// With `speed_adjust_rate = 100 mm/s/s`, one 100ms tick held reduces
    /// speed by exactly 10 mm/s (100 × 0.1s = 10).
    #[tokio::test(start_paused = true)]
    async fn button_held_slows_speed() {
        // Use a large speed_adjust_rate so one tick gives a measurable change.
        let cfg = cfg_with(10.0, 80.0, 100.0, 100.0);
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
            st.hand_switch = false;
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TideControl::Start).await.unwrap();
        tokio::time::advance(Duration::from_millis(1)).await;

        // Confirm the task started with max speed.
        assert!(
            (state.read().await.tide.speed_mm_s - 80.0).abs() < 0.1,
            "initial speed should be max (80)"
        );

        // Hold the button and let one SPEED_TICK fire.
        state.write().await.hand_switch = true;
        tokio::time::advance(SPEED_TICK).await;

        // 100 mm/s/s × 0.1 s = 10 mm/s reduction; 80 - 10 = 70.
        let speed = state.read().await.tide.speed_mm_s;
        assert!(
            (speed - 70.0).abs() < 0.5,
            "expected ~70 mm/s after one held tick, got {speed}"
        );

        // Drain any commands issued.
        while let Ok(c) = cmd_rx.try_recv() {
            let _ = c;
        }

        ctrl_tx.send(TideControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await;
        drop(ctrl_tx);
        let _ = h.await;
    }
}
