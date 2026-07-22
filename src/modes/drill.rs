//! Drill — interactive program (SSCP extension, not part of Handy FW4).
//!
//! The servo is off by default; the rod can be moved freely by hand.
//! While the deadman button is held (push pulses at 10–20 ms), the servo is
//! enabled and the rod advances outward at the configured feed rate.
//! Releasing the button (no pulse within the deadman window) stops the rod and
//! drops the servo.
//!
//! The deadman window must be longer than the maximum push interval (20 ms);
//! the default is 50 ms.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tracing::info;

use super::DrillControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// After ServoOn the IAI controller needs ~50 ms before it reliably accepts a
/// move command (same settle used in peck-probe).
const SERVO_SETTLE: Duration = Duration::from_millis(50);

pub struct DrillTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    /// Target position for an outward push: the far end of the stroke.
    outward_mm: f32,
    accel_g: f32,
    profile: MotionProfile,
    deadman_timeout: Duration,
    default_feed_rate_mm_s: f32,
}

impl DrillTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        DrillTask {
            state,
            cmd_tx,
            outward_mm: cfg.max_position_mm(),
            accel_g: cfg.actuator.drill.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
            deadman_timeout: Duration::from_millis(cfg.actuator.drill.deadman_timeout_ms),
            default_feed_rate_mm_s: cfg.actuator.drill.default_feed_rate_mm_s,
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<DrillControl>) {
        info!("drill task running");
        let mut pushing = false;
        let mut deadman: Option<Instant> = None;
        let mut feed_rate = self.default_feed_rate_mm_s;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(DrillControl::Start { feed_rate_mm_s }) => {
                        if let Some(r) = feed_rate_mm_s {
                            feed_rate = r.max(f32::MIN_POSITIVE);
                        }
                        // Decel-stop current motion, then drop the servo so the
                        // rod is free to move.
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        pushing = false;
                        deadman = None;
                        {
                            let mut st = self.state.write().await;
                            st.drill.active = true;
                            st.drill.pushing = false;
                            st.drill.feed_rate_mm_s = feed_rate;
                            st.set_mode(AppMode::Drill);
                        }
                        info!(feed_rate, "drill: entered drill mode");
                    }

                    Some(DrillControl::Push { feed_rate_mm_s }) => {
                        // Issue the outward move only when something actually
                        // changes — session start or a new feed rate. The rod is
                        // already heading to the far end, so re-commanding it on
                        // every deadman pulse only floods the command queue and
                        // delays the eventual Stop behind that backlog (the rod
                        // would then keep going well after release).
                        let mut reissue = false;

                        // Override feed rate if the pulse carries one.
                        if let Some(r) = feed_rate_mm_s {
                            let r = r.max(f32::MIN_POSITIVE);
                            if (r - feed_rate).abs() > f32::EPSILON {
                                feed_rate = r;
                                self.state.write().await.drill.feed_rate_mm_s = feed_rate;
                                reissue = true;
                            }
                        }

                        if !pushing {
                            // First pulse of a new push session: enable servo
                            // and wait for the controller to energise before
                            // issuing the move.
                            let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                            tokio::time::sleep(SERVO_SETTLE).await;
                            pushing = true;
                            self.state.write().await.drill.pushing = true;
                            reissue = true;
                        }

                        if reissue {
                            let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                pos_mm: self.outward_mm,
                                vel_mm_s: feed_rate,
                                accel_g: self.accel_g,
                                profile: self.profile,
                                // Deadman-paced pulses arrive faster than a ramp;
                                // never soften (it would stall the push).
                                soften: false,
                            }).await;
                        }

                        deadman = Some(Instant::now() + self.deadman_timeout);
                    }

                    Some(DrillControl::SetFeedRate { feed_rate_mm_s }) => {
                        let r = feed_rate_mm_s.max(f32::MIN_POSITIVE);
                        feed_rate = r;
                        self.state.write().await.drill.feed_rate_mm_s = feed_rate;
                        // Re-issue the move so the controller picks up the new
                        // velocity without waiting for the next deadman pulse.
                        if pushing {
                            let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                pos_mm: self.outward_mm,
                                vel_mm_s: feed_rate,
                                accel_g: self.accel_g,
                                profile: self.profile,
                                soften: false,
                            }).await;
                        }
                    }

                    Some(DrillControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        pushing = false;
                        deadman = None;
                        {
                            let mut st = self.state.write().await;
                            st.drill.active = false;
                            st.drill.pushing = false;
                            st.set_mode(AppMode::Idle);
                        }
                        info!("drill: stopped");
                    }
                },

                _ = sleep_until_opt(deadman), if deadman.is_some() => {
                    // Deadman expired — button released.
                    let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                    let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                    pushing = false;
                    deadman = None;
                    self.state.write().await.drill.pushing = false;
                    info!("drill: deadman expired, servo released");
                }
            }
        }

        info!("drill task stopped");
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d).await;
    } else {
        std::future::pending::<()>().await;
    }
}
