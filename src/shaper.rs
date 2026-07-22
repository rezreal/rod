//! Motion shaper — software jerk-limiting in front of the Modbus driver.
//!
//! The IAI controller only does trapezoidal moves (constant acceleration; the
//! S-curve CTLF bits don't exist on this hardware — see
//! docs/knock-rod-protocol-notes.md §1). To soften the *launch* of each stroke,
//! where jerk is worst (start from rest and oscillation reversals), this task
//! sits between the modes and the driver on the actuator-command channel:
//!
//!   * a [`ActuatorCommand::MoveTo`] with `soften: true` is expanded into a
//!     short sequence of sub-moves whose commanded velocity ramps from
//!     `start_velocity_frac · v` up to the full `v` along an ease-in curve, all
//!     toward the same target — approximating an S-curve leading edge out of the
//!     controller's trapezoid primitives;
//!   * every other command (plain moves, `Stop`, `ServoOn`, …) is forwarded
//!     unchanged, and any incoming command **preempts** an in-flight ramp.
//!
//! The driver only ever sees plain `soften: false` moves: the original soften
//! request is consumed here, and the emitted sub-moves are unshaped.
//!
//! Note: the oscillation modes estimate reversal timing from the *full* stroke
//! velocity, so a long ramp slightly shortens each stroke's reach. Keep
//! `ramp_ms` modest relative to a stroke's duration (the zone margins absorb the
//! small deficit).

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::debug;

use crate::config::{Config, MotionProfile};
use crate::state::ActuatorCommand;

pub struct Shaper {
    out: mpsc::Sender<ActuatorCommand>,
    step: Duration,
    ramp: Duration,
    start_frac: f32,
    curve_exp: f32,
}

/// State of an in-flight launch ramp.
struct Seq {
    pos_mm: f32,
    vel_mm_s: f32,
    accel_g: f32,
    /// Total number of sub-moves in the ramp.
    steps: u32,
    /// 1-based index of the next sub-move to emit.
    next: u32,
    /// When the next sub-move is due.
    deadline: Instant,
}

impl Shaper {
    pub fn new(out: mpsc::Sender<ActuatorCommand>, cfg: &Config) -> Self {
        let s = &cfg.actuator.softening;
        Shaper {
            out,
            step: Duration::from_millis(s.step_ms.max(1)),
            ramp: Duration::from_millis(s.ramp_ms),
            start_frac: s.start_velocity_frac.clamp(0.0, 1.0),
            curve_exp: s.curve_exp.max(0.1),
        }
    }

    /// Forward commands, expanding `soften` moves into a ramped sequence, until
    /// the input channel closes.
    pub async fn run(self, mut rx: mpsc::Receiver<ActuatorCommand>) {
        let mut seq: Option<Seq> = None;
        loop {
            tokio::select! {
                biased; // a new command preempts the in-flight ramp
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        ActuatorCommand::MoveTo {
                            pos_mm, vel_mm_s, accel_g, soften: true, ..
                        } => {
                            seq = self.begin(pos_mm, vel_mm_s, accel_g).await;
                        }
                        other => {
                            // Any non-soften command cancels shaping and passes through.
                            seq = None;
                            if self.out.send(other).await.is_err() { break; }
                        }
                    }
                }
                _ = sleep_until_opt(seq.as_ref().map(|s| s.deadline)), if seq.is_some() => {
                    if let Some(s) = seq.as_mut() {
                        self.emit(s).await;
                        if s.next > s.steps { seq = None; }
                    }
                }
            }
        }
        debug!("shaper task stopped");
    }

    /// Begin a ramp toward `pos_mm` at full `vel_mm_s`. Emits the first sub-move
    /// immediately. Returns `Some(seq)` if more sub-moves remain, or `None` if
    /// the move wasn't worth ramping (single move already sent).
    async fn begin(&self, pos_mm: f32, vel_mm_s: f32, accel_g: f32) -> Option<Seq> {
        let steps = (self.ramp.as_secs_f32() / self.step.as_secs_f32()).ceil() as u32;
        if steps <= 1 || vel_mm_s <= 0.0 {
            // Nothing to ramp — forward the full move unshaped.
            self.send_sub(pos_mm, vel_mm_s, accel_g).await;
            return None;
        }
        let mut s = Seq {
            pos_mm,
            vel_mm_s,
            accel_g,
            steps,
            next: 1,
            deadline: Instant::now(),
        };
        self.emit(&mut s).await;
        if s.next > s.steps {
            None
        } else {
            Some(s)
        }
    }

    /// Emit the current sub-move (`s.next`), then advance and reschedule.
    async fn emit(&self, s: &mut Seq) {
        let t = (s.next as f32 / s.steps as f32).clamp(0.0, 1.0);
        let eased = t.powf(self.curve_exp);
        let frac = self.start_frac + (1.0 - self.start_frac) * eased;
        let v = (s.vel_mm_s * frac).max(f32::MIN_POSITIVE);
        self.send_sub(s.pos_mm, v, s.accel_g).await;
        s.next += 1;
        s.deadline = Instant::now() + self.step;
    }

    async fn send_sub(&self, pos_mm: f32, vel_mm_s: f32, accel_g: f32) {
        let _ = self
            .out
            .send(ActuatorCommand::MoveTo {
                pos_mm,
                vel_mm_s,
                accel_g,
                profile: MotionProfile::Trapezoid,
                soften: false,
            })
            .await;
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

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            [actuator.softening]
            enable = true
            step_ms = 40
            ramp_ms = 120
            start_velocity_frac = 0.25
            curve_exp = 2.0
        "#,
        )
        .unwrap()
    }

    fn vel(c: &ActuatorCommand) -> f32 {
        match c {
            ActuatorCommand::MoveTo { vel_mm_s, .. } => *vel_mm_s,
            o => panic!("expected MoveTo, got {o:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn soften_expands_into_rising_velocity_ramp() {
        let (out_tx, mut out_rx) = mpsc::channel(32);
        let (in_tx, in_rx) = mpsc::channel(32);
        let h = tokio::spawn(Shaper::new(out_tx, &cfg()).run(in_rx));

        in_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: 200.0,
                vel_mm_s: 300.0,
                accel_g: 0.3,
                profile: MotionProfile::SCurve,
                soften: true,
            })
            .await
            .unwrap();

        // ceil(120/40) = 3 sub-moves, the first immediately then one per 40ms.
        let mut vels = vec![vel(&out_rx.recv().await.unwrap())];
        for _ in 0..2 {
            tokio::time::advance(Duration::from_millis(40)).await;
            vels.push(vel(&out_rx.recv().await.unwrap()));
        }
        // Strictly increasing, starting low, ending at full speed.
        assert!(vels[0] < vels[1] && vels[1] < vels[2], "rising: {vels:?}");
        assert!(
            vels[0] < 120.0,
            "first sub-move should be slow: {}",
            vels[0]
        );
        assert!(
            (vels[2] - 300.0).abs() < 1.0,
            "last reaches full v: {}",
            vels[2]
        );

        // Every emitted sub-move is unshaped so the driver never re-shapes.
        drop(in_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn new_command_preempts_in_flight_ramp() {
        let (out_tx, mut out_rx) = mpsc::channel(32);
        let (in_tx, in_rx) = mpsc::channel(32);
        let h = tokio::spawn(Shaper::new(out_tx, &cfg()).run(in_rx));

        in_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: 200.0,
                vel_mm_s: 300.0,
                accel_g: 0.3,
                profile: MotionProfile::SCurve,
                soften: true,
            })
            .await
            .unwrap();
        // First sub-move of the first ramp.
        let _ = out_rx.recv().await.unwrap();

        // A Stop arrives mid-ramp: it must pass straight through and cancel.
        in_tx.send(ActuatorCommand::Stop).await.unwrap();
        assert_eq!(out_rx.recv().await.unwrap(), ActuatorCommand::Stop);

        // No further sub-moves from the cancelled ramp.
        tokio::time::advance(Duration::from_millis(200)).await;
        tokio::task::yield_now().await;
        assert!(out_rx.try_recv().is_err(), "ramp should be cancelled");

        drop(in_tx);
        let _ = h.await;
    }
}
