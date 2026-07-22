//! Impale — slow-extension button-hold program.
//!
//! While the button is held the servo is enabled and the rod extends outward
//! at the configured feed rate. On release the rod stops and the servo is
//! deenergised (brake). After a configurable idle period (`retract_after_s`,
//! default 10 minutes) the rod retracts to home automatically, then the servo
//! is released again.
//!
//! The held button is a deadman: the client resends `Button { down: true }`
//! every ~50 ms and the rod only keeps extending while those heartbeats arrive.
//! An explicit `down: false` or a heartbeat gap longer than `deadman_timeout_ms`
//! both brake the rod — so a dropped release packet or a connection loss mid-
//! extension can never leave the rod advancing unattended.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tracing::info;

use super::ImpaleControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// After ServoOn the IAI controller needs ~50 ms before it reliably accepts a
/// move command (same settle used in drill and peck-probe).
const SERVO_SETTLE: Duration = Duration::from_millis(50);

pub struct ImpaleTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    outward_mm: f32,
    default_feed_rate_mm_s: f32,
    default_retract_after_s: f32,
    retract_speed_mm_s: f32,
    accel_g: f32,
    profile: MotionProfile,
    deadman_timeout: Duration,
}

impl ImpaleTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        ImpaleTask {
            state,
            cmd_tx,
            outward_mm: cfg.max_position_mm(),
            default_feed_rate_mm_s: cfg.actuator.impale.feed_rate_mm_s,
            default_retract_after_s: cfg.actuator.impale.retract_after_s as f32,
            retract_speed_mm_s: cfg.actuator.impale.retract_speed_mm_s,
            accel_g: cfg.actuator.impale.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
            deadman_timeout: Duration::from_millis(cfg.actuator.impale.deadman_timeout_ms),
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<ImpaleControl>) {
        info!("impale task running");
        // Whether the rod is currently moving outward (servo on, button held).
        let mut extending = false;
        // Whether the servo is currently energised (extending or retracting).
        let mut servo_on = false;
        // Deadman deadline: armed while the button is held, re-armed on every
        // heartbeat. Expiry (or an explicit release) brakes the rod.
        let mut deadman: Option<Instant> = None;
        // When to trigger the auto-retract (armed after the rod brakes).
        let mut retract_at: Option<Instant> = None;
        // When the retract move is estimated to be complete.
        let mut retract_done: Option<Instant> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(ImpaleControl::Start { feed_rate_mm_s, retract_after_s }) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        extending = false;
                        servo_on = false;
                        deadman = None;
                        retract_at = None;
                        retract_done = None;
                        {
                            let mut st = self.state.write().await;
                            let rate = feed_rate_mm_s
                                .unwrap_or(self.default_feed_rate_mm_s)
                                .max(f32::MIN_POSITIVE);
                            let hold = retract_after_s
                                .unwrap_or(self.default_retract_after_s)
                                .max(0.0);
                            st.impale.active = true;
                            st.impale.extending = false;
                            st.impale.waiting = false;
                            st.impale.retracting = false;
                            st.impale.feed_rate_mm_s = rate;
                            st.impale.retract_after_s = hold;
                            st.set_mode(AppMode::Impale);
                        }
                        info!("impale: entered mode");
                    }

                    Some(ImpaleControl::SetRetractAfter { retract_after_s }) => {
                        let hold = retract_after_s.max(0.0);
                        self.state.write().await.impale.retract_after_s = hold;
                        // If the rod is braked and waiting, re-arm the timer from
                        // now with the new duration.
                        if retract_at.is_some() {
                            retract_at =
                                Some(Instant::now() + Duration::from_secs_f32(hold));
                        }
                        info!(retract_after_s = hold, "impale: hold duration set");
                    }

                    Some(ImpaleControl::Button { down: true }) => {
                        // (Re)arm the deadman on every heartbeat and cancel any
                        // pending retract.
                        deadman = Some(Instant::now() + self.deadman_timeout);
                        retract_at = None;
                        retract_done = None;
                        if !extending {
                            if !servo_on {
                                let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                                tokio::time::sleep(SERVO_SETTLE).await;
                                servo_on = true;
                            }
                            extending = true;
                            let feed_rate = self.state.read().await.impale.feed_rate_mm_s;
                            let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                pos_mm: self.outward_mm,
                                vel_mm_s: feed_rate,
                                accel_g: self.accel_g,
                                profile: self.profile,
                                soften: false,
                            }).await;
                            {
                                let mut st = self.state.write().await;
                                st.impale.extending = true;
                                st.impale.waiting = false;
                                st.impale.retracting = false;
                            }
                            info!("impale: extending");
                        }
                    }

                    Some(ImpaleControl::Button { down: false }) => {
                        if extending {
                            self.brake_and_arm_retract(
                                &mut extending, &mut servo_on, &mut deadman, &mut retract_at,
                            ).await;
                        }
                    }

                    Some(ImpaleControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        extending = false;
                        servo_on = false;
                        deadman = None;
                        retract_at = None;
                        retract_done = None;
                        {
                            let mut st = self.state.write().await;
                            st.impale.active = false;
                            st.impale.extending = false;
                            st.impale.waiting = false;
                            st.impale.retracting = false;
                            st.set_mode(AppMode::Idle);
                        }
                        info!("impale: stopped");
                    }
                },

                _ = sleep_until_opt(deadman), if deadman.is_some() => {
                    // Heartbeat gap — treat as button released.
                    self.brake_and_arm_retract(
                        &mut extending, &mut servo_on, &mut deadman, &mut retract_at,
                    ).await;
                    info!("impale: deadman expired");
                }

                _ = sleep_until_opt(retract_at), if retract_at.is_some() => {
                    retract_at = None;
                    let current_mm = self.state.read().await.position_mm;
                    if !servo_on {
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        tokio::time::sleep(SERVO_SETTLE).await;
                        servo_on = true;
                    }
                    let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                        pos_mm: 0.0,
                        vel_mm_s: self.retract_speed_mm_s,
                        accel_g: self.accel_g,
                        profile: self.profile,
                        soften: false,
                    }).await;
                    // Estimate travel time with a generous 1 s margin.
                    let travel_ms = if self.retract_speed_mm_s > 0.0 {
                        ((current_mm / self.retract_speed_mm_s) * 1000.0) as u64 + 1000
                    } else {
                        2000
                    };
                    retract_done = Some(Instant::now() + Duration::from_millis(travel_ms));
                    {
                        let mut st = self.state.write().await;
                        st.impale.waiting = false;
                        st.impale.retracting = true;
                    }
                    info!("impale: retracting");
                }

                _ = sleep_until_opt(retract_done), if retract_done.is_some() => {
                    retract_done = None;
                    let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                    servo_on = false;
                    self.state.write().await.impale.retracting = false;
                    info!("impale: retract complete, servo off");
                }
            }
        }

        info!("impale task stopped");
    }

    /// Decel-stop the outward move and park the rod (servo off, holding brake
    /// engaged) so it stays clamped where it was impaled, then arm the auto-
    /// retract timer. Shared by explicit release and deadman expiry.
    ///
    /// Uses `Park` rather than `ServoOn(false)` on purpose: with the global
    /// `release_brake_on_servo_off` policy a plain servo-off would force-release
    /// the holding brake and let the rod hang free. `Park` keeps it clamped.
    async fn brake_and_arm_retract(
        &self,
        extending: &mut bool,
        servo_on: &mut bool,
        deadman: &mut Option<Instant>,
        retract_at: &mut Option<Instant>,
    ) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        let _ = self.cmd_tx.send(ActuatorCommand::Park).await;
        *extending = false;
        *servo_on = false;
        *deadman = None;
        let hold = self.state.read().await.impale.retract_after_s.max(0.0);
        *retract_at = Some(Instant::now() + Duration::from_secs_f32(hold));
        {
            let mut st = self.state.write().await;
            st.impale.extending = false;
            st.impale.waiting = true;
            st.impale.retracting = false;
        }
        info!(retract_after_s = hold, "impale: parked (brake-hold); retract armed");
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d).await;
    } else {
        std::future::pending::<()>().await;
    }
}
