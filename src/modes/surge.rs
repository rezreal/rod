//! Surge — hand-switch-driven oscillation with exponential arousal dynamics.
//!
//! Oscillates between the calibrated work-piece origin (from a prior soft-touch
//! calibration) and a configured ceiling, with both speed and stroke depth
//! governed by a single `arousal` value [0..1]:
//!
//! * While the DIPM hand switch is held, arousal rises exponentially toward 1.0
//!   — the rate naturally slows as it approaches the ceiling.
//! * When the switch is released, arousal decays linearly back to 0.0.
//! * Both the lower (return) bound and the upper (outward) bound grow with
//!   arousal, so at low arousal the strokes are shallow and slow; at full
//!   arousal the rod covers the full programmed depth at maximum speed.
//!
//! Oscillation is timer-driven, not PEND-gated (same design as HAMP). No
//! oscillation occurs while arousal is below a minimum stroke threshold; the
//! rod simply holds at the current position.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::SurgeControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// How often arousal is recalculated and the oscillation threshold checked.
const AROUSAL_TICK: Duration = Duration::from_millis(100);
/// Margin added to estimated travel time before a reversal (same as HAMP).
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);
/// Below this stroke span (mm) the task holds still rather than micro-oscillating.
const MIN_STROKE_MM: f32 = 2.0;

pub struct SurgeTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    /// Hard upper soft-limit in mm (from `cfg.max_position_mm()`).
    max_mm: f32,
    rise_rate: f32,
    fall_rate: f32,
    min_speed_mm_s: f32,
    max_speed_mm_s: f32,
    lower_drift_mm: f32,
    max_depth_pct: f32,
    accel_g: f32,
    profile: MotionProfile,
}

impl SurgeTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        SurgeTask {
            state,
            cmd_tx,
            max_mm: cfg.max_position_mm(),
            rise_rate: cfg.actuator.surge.rise_rate,
            fall_rate: cfg.actuator.surge.fall_rate,
            min_speed_mm_s: cfg.actuator.surge.min_speed_mm_s,
            max_speed_mm_s: cfg.actuator.surge.max_speed_mm_s,
            lower_drift_mm: cfg.actuator.surge.lower_drift_mm,
            max_depth_pct: cfg.actuator.surge.max_depth_pct,
            accel_g: cfg.actuator.surge.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
        }
    }

    /// Derive lower bound, upper bound, and speed from current arousal.
    ///
    /// * `lower` = work_origin + arousal × lower_drift_mm
    ///   (return point drifts slightly outward as arousal builds)
    /// * `upper` = work_origin + arousal × (ceiling − work_origin)
    ///   (ceiling = max_mm × max_depth_pct)
    /// * `speed` = min_speed + arousal × (max_speed − min_speed)
    fn motion_params(&self, arousal: f32, work_origin: f32) -> (f32, f32, f32) {
        let lower = work_origin + arousal * self.lower_drift_mm;
        let ceiling = self.max_mm * self.max_depth_pct;
        let upper = (work_origin + arousal * (ceiling - work_origin)).max(lower);
        let speed = (self.min_speed_mm_s + arousal * (self.max_speed_mm_s - self.min_speed_mm_s))
            .max(f32::MIN_POSITIVE);
        (lower, upper, speed)
    }

    /// Issue one stroke and return the estimated travel time, or `None` if
    /// arousal is too low to warrant oscillation.
    async fn stroke(&self, out: &mut bool, work_origin: f32) -> Option<Duration> {
        let arousal = self.state.read().await.surge.arousal;
        let (lower, upper, speed) = self.motion_params(arousal, work_origin);
        let span = upper - lower;
        if span < MIN_STROKE_MM {
            return None;
        }

        let target = if *out { lower } else { upper };
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: target,
                vel_mm_s: speed,
                accel_g: self.accel_g,
                profile: self.profile,
                soften: false,
            })
            .await;

        let travel_ms = (span / speed) * 1000.0;
        let travel = Duration::from_millis(travel_ms.max(1.0) as u64) + REVERSAL_MARGIN;
        *out = !*out;
        Some(travel)
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<SurgeControl>) {
        info!("surge task running");
        let mut arousal_ticker = tokio::time::interval(AROUSAL_TICK);
        arousal_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut out = false;
        let mut next: Option<Instant> = None;
        let mut work_origin = 0.0f32;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(SurgeControl::Start) => {
                        work_origin = {
                            let st = self.state.read().await;
                            match st.work_origin_mm {
                                Some(o) => o,
                                None => {
                                    warn!("surge: no calibration; work origin defaulting to 0 mm");
                                    0.0
                                }
                            }
                        };
                        {
                            let mut st = self.state.write().await;
                            st.surge.active = true;
                            st.surge.arousal = 0.0;
                            st.set_mode(AppMode::Surge);
                        }
                        out = false;
                        next = None;
                        info!(work_origin, "surge: started");
                    }

                    Some(SurgeControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        {
                            let mut st = self.state.write().await;
                            st.surge.active = false;
                            st.surge.arousal = 0.0;
                            st.set_mode(AppMode::Idle);
                        }
                        out = false;
                        next = None;
                        info!("surge: stopped");
                    }
                },

                _ = arousal_ticker.tick() => {
                    let active = self.state.read().await.surge.active;
                    if !active { continue; }

                    let (arousal, held) = {
                        let st = self.state.read().await;
                        (st.surge.arousal, st.hand_switch)
                    };
                    let dt = AROUSAL_TICK.as_secs_f32();
                    let new_arousal = if held {
                        (arousal + dt * self.rise_rate * (1.0 - arousal)).min(1.0)
                    } else {
                        (arousal - dt * self.fall_rate).max(0.0)
                    };
                    self.state.write().await.surge.arousal = new_arousal;

                    let (lower, upper, _) = self.motion_params(new_arousal, work_origin);
                    let can_oscillate = upper - lower >= MIN_STROKE_MM;

                    if next.is_some() && !can_oscillate {
                        // Arousal just dropped below the minimum stroke — stop.
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        next = None;
                    } else if next.is_none() && can_oscillate {
                        // Just crossed the threshold — begin oscillating immediately.
                        next = Some(Instant::now());
                    }
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    match self.stroke(&mut out, work_origin).await {
                        Some(d) => next = Some(Instant::now() + d),
                        None => {
                            let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                            next = None;
                        }
                    }
                }
            }
        }

        info!("surge task stopped");
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
    use tokio::sync::broadcast;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.surge]
            min_speed_mm_s = 10.0
            max_speed_mm_s = 100.0
            lower_drift_mm = 0.0
            max_depth_pct  = 1.0
            accel_g        = 0.1
        "#,
        )
        .unwrap()
    }

    fn make(
        cfg: &Config,
    ) -> (
        SurgeTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (_notif_tx, _) = broadcast::channel::<crate::rpc::RpcMessage>(1);
        (SurgeTask::new(state.clone(), cmd_tx, cfg), state, cmd_rx)
    }

    #[test]
    fn motion_params_scale_with_arousal() {
        let cfg = cfg();
        let (task, _, _) = make(&cfg);
        let work_origin = 50.0f32;

        // At arousal=0: span=0, speed=min.
        let (lo, hi, spd) = task.motion_params(0.0, work_origin);
        assert_eq!(lo, work_origin);
        assert_eq!(hi, work_origin);
        assert_eq!(spd, 10.0);

        // At arousal=1: upper = work_origin + (max_mm - work_origin) = 300mm,
        // lower = work_origin + 0 (lower_drift=0), speed = 100 mm/s.
        let (lo, hi, spd) = task.motion_params(1.0, work_origin);
        assert_eq!(lo, 50.0);
        assert_eq!(hi, 300.0);
        assert_eq!(spd, 100.0);

        // At arousal=0.5: upper = 50 + 0.5*(300-50) = 175, speed = 55 mm/s.
        let (lo, hi, spd) = task.motion_params(0.5, work_origin);
        assert_eq!(lo, 50.0);
        assert!((hi - 175.0).abs() < 0.01);
        assert!((spd - 55.0).abs() < 0.01);
    }

    #[test]
    fn motion_params_lower_drift_at_full_arousal() {
        // With lower_drift_mm = 5: lower moves 5mm past work_origin at arousal=1.
        let cfg: Config = toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.surge]
            lower_drift_mm = 5.0
            max_depth_pct  = 1.0
            min_speed_mm_s = 5.0
            max_speed_mm_s = 80.0
            accel_g = 0.1
        "#,
        )
        .unwrap();
        let (task, _, _) = make(&cfg);
        let (lo, _, _) = task.motion_params(1.0, 50.0);
        assert!((lo - 55.0).abs() < 0.01);
    }

    #[tokio::test(start_paused = true)]
    async fn oscillates_outward_then_inward_at_full_arousal() {
        // Use instant rise_rate so one 100ms arousal tick builds arousal to 1.0.
        let cfg: Config = toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.surge]
            rise_rate      = 1000.0
            fall_rate      = 0.0
            min_speed_mm_s = 10.0
            max_speed_mm_s = 100.0
            lower_drift_mm = 0.0
            max_depth_pct  = 1.0
            accel_g        = 0.1
        "#,
        )
        .unwrap();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(50.0);
            st.hand_switch = true; // button held throughout
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(SurgeControl::Start).await.unwrap();

        // Advance one arousal tick (100ms): arousal → 1.0, oscillation kicks off.
        // The reversal timer is set to Instant::now() which immediately fires in
        // the same select pass, so the first stroke is emitted before advance returns.
        tokio::time::advance(Duration::from_millis(100)).await;

        // First stroke: outward (out=false) → upper_mm = 300 mm at arousal=1.
        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert!(
                    (pos_mm - 300.0).abs() < 0.1,
                    "expected 300 mm, got {pos_mm}"
                );
                assert!((vel_mm_s - 100.0).abs() < 0.1);
            }
            o => panic!("unexpected: {o:?}"),
        }

        // 250mm @ 100mm/s = 2500ms + 10ms margin → advance past the reversal.
        tokio::time::advance(Duration::from_millis(2600)).await;

        // Second stroke: inward (out=true) → lower_mm = 50 mm.
        let second = cmd_rx.recv().await.unwrap();
        match second {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!((pos_mm - 50.0).abs() < 0.1, "expected 50 mm, got {pos_mm}");
            }
            o => panic!("unexpected: {o:?}"),
        }

        ctrl_tx.send(SurgeControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain the Stop ActuatorCommand
        drop(ctrl_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn stops_when_arousal_drops_to_zero() {
        // rise_rate=1000 → instant build; fall_rate=1000 → instant decay.
        let cfg: Config = toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.surge]
            rise_rate      = 1000.0
            fall_rate      = 1000.0
            min_speed_mm_s = 10.0
            max_speed_mm_s = 100.0
            lower_drift_mm = 0.0
            max_depth_pct  = 1.0
            accel_g        = 0.1
        "#,
        )
        .unwrap();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
            st.hand_switch = true; // held: arousal builds on first tick
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(SurgeControl::Start).await.unwrap();

        // First arousal tick: arousal → 1.0 (button held), first stroke fires.
        tokio::time::advance(Duration::from_millis(100)).await;
        let first = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(first, ActuatorCommand::MoveTo { .. }),
            "expected MoveTo, got {first:?}"
        );

        // Release button; next arousal tick drops arousal to 0 → stop issued.
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;

        let stop = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(stop, ActuatorCommand::Stop),
            "expected Stop, got {stop:?}"
        );

        drop(ctrl_tx);
        let _ = h.await;
    }
}
