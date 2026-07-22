//! Impale — slow-extension button-hold program.
//!
//! While the button is held the servo is enabled and the rod extends outward
//! at the configured feed rate. On release the rod decel-stops and the servo
//! is left energised to hold position: this unit has no working mechanical
//! brake (confirmed on hardware — the controller reports the holding brake
//! engaged via BKRL, but the rod still back-drives freely), so de-energising
//! would just let it hang loose. Keeping the servo on is the only way to
//! actually resist hand force. After a configurable idle period
//! (`retract_after_s`, default 10 minutes) the rod retracts to home
//! automatically, then the servo is released.
//!
//! Reaching that retract deadline without an explicit `Stop` is the win
//! condition for a hold cycle (`ImpaleRuntime::won`). The deadline is armed
//! once, on the first release, and keeps running in the background across
//! any number of further extend/release cycles — pressing the button again
//! only pauses its *effect* (no auto-retract while actively extending), it
//! does not push the deadline back. Only `Stop` (or a new `Start`) clears it.
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
                            st.impale.retract_deadline = None;
                            st.impale.won = false;
                            st.set_mode(AppMode::Impale);
                        }
                        info!("impale: entered mode");
                    }

                    Some(ImpaleControl::SetRetractAfter { retract_after_s }) => {
                        let hold = retract_after_s.max(0.0);
                        // If the rod is braked and waiting, re-arm the timer from
                        // now with the new duration.
                        if retract_at.is_some() {
                            retract_at =
                                Some(Instant::now() + Duration::from_secs_f32(hold));
                        }
                        {
                            let mut st = self.state.write().await;
                            st.impale.retract_after_s = hold;
                            if st.impale.retract_deadline.is_some() {
                                st.impale.retract_deadline =
                                    Some(std::time::Instant::now() + Duration::from_secs_f32(hold));
                            }
                        }
                        info!(retract_after_s = hold, "impale: hold duration set");
                    }

                    Some(ImpaleControl::Button { down: true }) => {
                        // (Re)arm the deadman on every heartbeat. Deliberately
                        // does NOT touch retract_at/retract_deadline — an
                        // already-armed retract deadline keeps running in the
                        // background; the sleep_until_opt(retract_at) arm below
                        // is guarded on `!extending` so it just won't fire
                        // while we're actively extending again.
                        deadman = Some(Instant::now() + self.deadman_timeout);
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
                            st.impale.retract_deadline = None;
                            st.impale.won = false;
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

                // Only fires while not actively extending — see the
                // Button{down:true} arm above: an armed deadline keeps ticking
                // in the background while extending resumes, but auto-retract
                // itself waits until the rod is released again.
                _ = sleep_until_opt(retract_at), if retract_at.is_some() && !extending => {
                    // Reached the retract deadline without an explicit Stop —
                    // that's the win condition for this hold cycle.
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
                        st.impale.retract_deadline = None;
                        st.impale.won = true;
                    }
                    info!("impale: won (retract deadline reached); retracting");
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

    /// Decel-stop the outward move and hold the rod where it was impaled,
    /// then arm the auto-retract timer — unless one is already running, in
    /// which case it's left untouched (a previous release already started
    /// the clock; resuming and releasing again must not push it back).
    /// Shared by explicit release and deadman expiry.
    ///
    /// Deliberately does *not* de-energise the servo (`ServoOn(false)` /
    /// `Park`): this unit's holding brake doesn't actually grip (verified on
    /// hardware — BKRL reads "engaged" but the rod still moves by hand), so
    /// cutting servo power would leave it free either way. Leaving the servo
    /// on lets closed-loop position hold do the clamping instead.
    async fn brake_and_arm_retract(
        &self,
        extending: &mut bool,
        servo_on: &mut bool,
        deadman: &mut Option<Instant>,
        retract_at: &mut Option<Instant>,
    ) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        *extending = false;
        debug_assert!(*servo_on, "extending implies the servo was already on");
        *deadman = None;
        if retract_at.is_none() {
            let hold = self.state.read().await.impale.retract_after_s.max(0.0);
            *retract_at = Some(Instant::now() + Duration::from_secs_f32(hold));
            let mut st = self.state.write().await;
            st.impale.retract_deadline =
                Some(std::time::Instant::now() + Duration::from_secs_f32(hold));
            // A fresh deadline means any earlier win is done and dusted —
            // clear it so the next one can fire its own banner/chime.
            st.impale.won = false;
            info!(retract_after_s = hold, "impale: retract timer armed");
        }
        {
            let mut st = self.state.write().await;
            st.impale.extending = false;
            st.impale.waiting = true;
            st.impale.retracting = false;
        }
        info!("impale: holding position (servo-hold, no mechanical brake)");
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d).await;
    } else {
        std::future::pending::<()>().await;
    }
}
