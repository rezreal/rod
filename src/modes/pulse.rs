//! Pulse — heart-rate-reactive oscillation (SSCP extension).
//!
//! Oscillates between the zone ends like HAMP, but the stroke speed tracks a
//! connected heart-rate sensor: `velocity = bpm × factor`, clamped to the
//! configured limits. `factor` (mm/s per BPM) is the program parameter — set at
//! start and adjustable live. With no sensor connected it falls back to a base
//! BPM so the program still runs.
//!
//! BPM is supplied by the BLE-central sensor task (`src/sensors/`) via
//! [`AppState::sensors`]; this task only reads it.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::Instant;
use tracing::info;

use super::PulseControl;
use crate::config::{Config, MotionProfile, Pulse};
use crate::state::{ActuatorCommand, AppMode, AppState};

const SERVO_SETTLE: Duration = Duration::from_millis(50);
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);

pub struct PulseTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    stroke_mm: f32,
    p: Pulse,
    max_velocity_mm_s: f32,
    profile: MotionProfile,
}

impl PulseTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        PulseTask {
            state,
            cmd_tx,
            stroke_mm: cfg.stroke_mm(),
            max_velocity_mm_s: cfg
                .actuator
                .pulse
                .max_velocity_mm_s
                .min(cfg.actuator.limits.max_velocity_mm_s),
            p: cfg.actuator.pulse.clone(),
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
        }
    }

    /// Velocity for a given BPM: `bpm × factor`, clamped to [min, max].
    fn velocity(&self, bpm: u16, factor: f32) -> f32 {
        (bpm as f32 * factor).clamp(self.p.min_velocity_mm_s, self.max_velocity_mm_s)
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<PulseControl>) {
        info!("pulse task running");
        let mut active = false;
        let mut factor = self.p.default_factor;
        let mut out = false;
        let mut next: Option<Instant> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,
                    Some(PulseControl::Start { factor: f }) => {
                        if let Some(f) = f { factor = f.max(0.0); }
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                        tokio::time::sleep(SERVO_SETTLE).await;
                        active = true;
                        out = false;
                        next = Some(Instant::now());
                        {
                            let mut st = self.state.write().await;
                            st.pulse.active = true;
                            st.pulse.factor = factor;
                            st.set_mode(AppMode::Pulse);
                        }
                        info!(factor, "pulse: started");
                    }
                    Some(PulseControl::SetFactor { factor: f }) => {
                        factor = f.max(0.0);
                        self.state.write().await.pulse.factor = factor;
                    }
                    Some(PulseControl::Stop) => {
                        self.halt(&mut active, &mut next).await;
                    }
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    let travel = self.stroke(out, factor).await;
                    out = !out;
                    next = Some(Instant::now() + travel);
                }
            }
        }
        info!("pulse task stopped");
    }

    async fn stroke(&self, out: bool, factor: f32) -> Duration {
        // Live BPM if a sensor is connected, else the configured fallback.
        let bpm = {
            let st = self.state.read().await;
            st.sensors.hr_bpm.unwrap_or(self.p.base_bpm)
        };
        let v = self.velocity(bpm, factor).max(f32::MIN_POSITIVE);
        let target_rel = if out {
            self.p.zone_min
        } else {
            self.p.zone_max
        };

        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: target_rel * self.stroke_mm,
                vel_mm_s: v,
                accel_g: self.p.accel_g,
                profile: self.profile,
                soften: false,
            })
            .await;

        {
            let mut st = self.state.write().await;
            st.pulse.bpm = bpm;
            st.pulse.velocity_mm_s = v;
        }

        let span_mm = ((self.p.zone_max - self.p.zone_min) * self.stroke_mm).abs();
        Duration::from_millis(((span_mm / v) * 1000.0).max(1.0) as u64) + REVERSAL_MARGIN
    }

    async fn halt(&self, active: &mut bool, next: &mut Option<Instant>) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        // Rest with the brake holding the rod (automatic oscillation, like ramp).
        let _ = self.cmd_tx.send(ActuatorCommand::Park).await;
        *active = false;
        *next = None;
        let mut st = self.state.write().await;
        st.pulse.active = false;
        st.set_mode(AppMode::Idle);
        info!("pulse: stopped");
    }
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending::<()>().await,
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
            [actuator.pulse]
            default_factor = 2.0
            min_velocity_mm_s = 30.0
            max_velocity_mm_s = 300.0
            base_bpm = 70
        "#,
        )
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn speed_tracks_bpm_times_factor() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        // 100 bpm × factor 2.0 = 200 mm/s.
        state.write().await.sensors.hr_bpm = Some(100);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let h = tokio::spawn(PulseTask::new(state.clone(), cmd_tx, &cfg()).run(ctrl_rx));

        ctrl_tx
            .send(PulseControl::Start { factor: Some(2.0) })
            .await
            .unwrap();
        // ServoOn then the first stroke.
        assert_eq!(cmd_rx.recv().await.unwrap(), ActuatorCommand::ServoOn(true));
        let v = match cmd_rx.recv().await.unwrap() {
            ActuatorCommand::MoveTo { vel_mm_s, .. } => vel_mm_s,
            o => panic!("{o:?}"),
        };
        assert!((v - 200.0).abs() < 1.0, "expected 200 mm/s, got {v}");

        // Raising the heart rate raises the speed on the next stroke.
        state.write().await.sensors.hr_bpm = Some(150); // ×2 = 300, clamped to max 300
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        let mut v2 = v;
        while let Ok(c) = cmd_rx.try_recv() {
            if let ActuatorCommand::MoveTo { vel_mm_s, .. } = c {
                v2 = vel_mm_s;
            }
        }
        assert!(v2 > v, "faster heart rate → faster strokes: {v} -> {v2}");

        ctrl_tx.send(PulseControl::Stop).await.unwrap();
        drop(ctrl_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn falls_back_to_base_bpm_without_sensor() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let h = tokio::spawn(PulseTask::new(state.clone(), cmd_tx, &cfg()).run(ctrl_rx));

        ctrl_tx
            .send(PulseControl::Start { factor: Some(1.0) })
            .await
            .unwrap();
        let _ = cmd_rx.recv().await; // ServoOn
        let v = match cmd_rx.recv().await.unwrap() {
            ActuatorCommand::MoveTo { vel_mm_s, .. } => vel_mm_s,
            o => panic!("{o:?}"),
        };
        // base_bpm 70 × 1.0 = 70 mm/s (above the 30 min).
        assert!((v - 70.0).abs() < 1.0, "expected base 70 mm/s, got {v}");
        assert_eq!(state.read().await.pulse.bpm, 70);

        ctrl_tx.send(PulseControl::Stop).await.unwrap();
        drop(ctrl_tx);
        let _ = h.await;
    }
}
