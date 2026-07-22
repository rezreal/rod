//! Echo — tap-driven depth-stepping oscillation.
//!
//! The machine waits at the calibrated work-piece origin between button presses.
//! Each short tap fires one outward-and-back stroke to the current target depth,
//! then advances the depth by `step_mm` (capped at `max_depth_mm`).
//! A long hold (≥ `reset_hold_ms`) resets the depth back to the start without
//! firing a stroke, and returns the rod to the origin if it isn't already there.
//!
//! State machine:
//!   * Idle       — waiting at work_origin; next tap queues a stroke.
//!   * StrokingOut — MoveTo(current_depth_mm) in progress; timer fires reversal.
//!   * StrokingBack — MoveTo(work_origin) in progress; timer fires depth-advance + Idle.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::EchoControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// How often the hand-switch state is sampled for tap/hold detection.
const SWITCH_TICK: Duration = Duration::from_millis(100);
/// Margin added to estimated travel time before a phase transition.
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);

/// Internal stroke phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    StrokingOut,
    StrokingBack,
}

pub struct EchoTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    /// Hard upper position limit (mm).
    max_mm: f32,
    /// Initial depth above work_origin (mm) — first stroke target offset.
    start_depth_mm: f32,
    /// How much to advance the target after each completed stroke (mm).
    step_mm: f32,
    /// If > 0: cap = work_origin + max_extra_depth_mm; else cap = max_mm.
    max_extra_depth_mm: f32,
    /// Stroke speed (mm/s).
    speed_mm_s: f32,
    /// Stroke acceleration (G).
    accel_g: f32,
    /// Motion profile for all moves.
    profile: MotionProfile,
    /// How long the button must be held to trigger a reset (ms).
    reset_hold_ms: u64,
}

impl EchoTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        EchoTask {
            state,
            cmd_tx,
            max_mm: cfg.max_position_mm(),
            start_depth_mm: cfg.actuator.echo.start_depth_mm,
            step_mm: cfg.actuator.echo.step_mm,
            max_extra_depth_mm: cfg.actuator.echo.max_extra_depth_mm,
            speed_mm_s: cfg.actuator.echo.speed_mm_s,
            accel_g: cfg.actuator.echo.accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
            reset_hold_ms: cfg.actuator.echo.reset_hold_ms,
        }
    }

    /// Return the absolute depth cap based on config.
    fn depth_cap(&self, work_origin: f32) -> f32 {
        if self.max_extra_depth_mm > 0.0 {
            work_origin + self.max_extra_depth_mm
        } else {
            self.max_mm
        }
    }

    /// Estimate one-way travel time between two positions at the configured speed.
    fn travel_duration(&self, from_mm: f32, to_mm: f32) -> Duration {
        let dist = (to_mm - from_mm).abs();
        let ms = (dist / self.speed_mm_s) * 1000.0;
        Duration::from_millis(ms.max(1.0) as u64) + REVERSAL_MARGIN
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<EchoControl>) {
        info!("echo task running");
        let mut switch_ticker = tokio::time::interval(SWITCH_TICK);
        switch_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut phase = Phase::Idle;
        let mut next: Option<Instant> = None;
        let mut work_origin = 0.0f32;

        // Edge detection state.
        let mut prev_held = false;
        let mut hold_ticks: u32 = 0;
        let mut reset_fired = false;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(EchoControl::Start) => {
                        work_origin = {
                            let st = self.state.read().await;
                            match st.work_origin_mm {
                                Some(o) => o,
                                None => {
                                    warn!("echo: no calibration; work origin defaulting to 0 mm");
                                    0.0
                                }
                            }
                        };
                        let initial_depth = (work_origin + self.start_depth_mm)
                            .min(self.depth_cap(work_origin));
                        {
                            let mut st = self.state.write().await;
                            st.echo.active = true;
                            st.echo.current_depth_mm = initial_depth;
                            st.echo.steps_taken = 0;
                            st.set_mode(AppMode::Echo);
                        }
                        phase = Phase::Idle;
                        next = None;
                        prev_held = false;
                        hold_ticks = 0;
                        reset_fired = false;
                        info!(work_origin, initial_depth, "echo: started");
                    }

                    Some(EchoControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        {
                            let mut st = self.state.write().await;
                            st.echo.active = false;
                            st.set_mode(AppMode::Idle);
                        }
                        phase = Phase::Idle;
                        next = None;
                        prev_held = false;
                        hold_ticks = 0;
                        reset_fired = false;
                        info!("echo: stopped");
                    }
                },

                _ = switch_ticker.tick() => {
                    let active = self.state.read().await.echo.active;
                    if !active { continue; }

                    let held = self.state.read().await.hand_switch;

                    if held && !prev_held {
                        // Rising edge.
                        hold_ticks = 0;
                        reset_fired = false;
                    } else if held {
                        // Continued hold.
                        hold_ticks += 1;
                        let held_ms = hold_ticks as u64 * 100;
                        if held_ms >= self.reset_hold_ms && !reset_fired && phase == Phase::Idle {
                            // Long-hold reset.
                            let initial_depth = (work_origin + self.start_depth_mm)
                                .min(self.depth_cap(work_origin));
                            {
                                let mut st = self.state.write().await;
                                st.echo.current_depth_mm = initial_depth;
                                st.echo.steps_taken = 0;
                            }
                            reset_fired = true;
                            // Return to origin if not already there.
                            let pos = self.state.read().await.position_mm;
                            if (pos - work_origin).abs() > 0.5 {
                                let travel = self.travel_duration(pos, work_origin);
                                let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                    pos_mm: work_origin,
                                    vel_mm_s: self.speed_mm_s,
                                    accel_g: self.accel_g,
                                    profile: self.profile,
                                    soften: false,
                                }).await;
                                // We don't block on this — just fire and continue idle.
                                // The timer is only used during stroke phases; here we
                                // simply let the motor move and stay in Idle.
                                let _ = travel; // travel estimate available if needed
                            }
                            info!(initial_depth, "echo: depth reset by long hold");
                        }
                    } else if !held && prev_held {
                        // Falling edge: short tap fires a stroke if idle and no reset fired.
                        if !reset_fired && phase == Phase::Idle {
                            let current_depth = self.state.read().await.echo.current_depth_mm;
                            let travel = self.travel_duration(work_origin, current_depth);
                            let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                pos_mm: current_depth,
                                vel_mm_s: self.speed_mm_s,
                                accel_g: self.accel_g,
                                profile: self.profile,
                                    soften: false,
                            }).await;
                            phase = Phase::StrokingOut;
                            next = Some(Instant::now() + travel);
                            info!(current_depth, "echo: stroke out");
                        }
                    }

                    prev_held = held;
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    match phase {
                        Phase::StrokingOut => {
                            // Outward stroke complete — begin return.
                            let travel = self.travel_duration(
                                self.state.read().await.echo.current_depth_mm,
                                work_origin,
                            );
                            let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                                pos_mm: work_origin,
                                vel_mm_s: self.speed_mm_s,
                                accel_g: self.accel_g,
                                profile: self.profile,
                                    soften: false,
                            }).await;
                            phase = Phase::StrokingBack;
                            next = Some(Instant::now() + travel);
                            info!("echo: stroke back");
                        }
                        Phase::StrokingBack => {
                            // Return complete — advance depth, return to Idle.
                            {
                                let mut st = self.state.write().await;
                                let cap = self.depth_cap(work_origin);
                                let new_depth = (st.echo.current_depth_mm + self.step_mm).min(cap);
                                st.echo.current_depth_mm = new_depth;
                                st.echo.steps_taken += 1;
                                info!(new_depth, steps = st.echo.steps_taken, "echo: depth advanced");
                            }
                            phase = Phase::Idle;
                            next = None;
                        }
                        Phase::Idle => {
                            // Shouldn't happen — next is None in Idle.
                            next = None;
                        }
                    }
                }
            }
        }

        info!("echo task stopped");
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
    use crate::config::Config;
    use crate::state::AppState;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
            default_motion_profile = "trapezoid"
            [actuator.echo]
            start_depth_mm    = 20.0
            step_mm           = 10.0
            max_extra_depth_mm = 0.0
            speed_mm_s        = 50.0
            accel_g           = 0.2
            reset_hold_ms     = 2000
        "#,
        )
        .unwrap()
    }

    fn make(
        cfg: &Config,
    ) -> (
        EchoTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        (EchoTask::new(state.clone(), cmd_tx, cfg), state, cmd_rx)
    }

    /// Advance the Tokio clock by one switch-tick and yield so the spawned task
    /// has a guaranteed opportunity to process the fired tick before the test
    /// continues. advance().await alone only yields once internally, which can
    /// race with the executor's task ordering; the extra yield_now() drains the
    /// pending wakeup reliably.
    async fn one_tick() {
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
    }

    // ── Test 1: first tap strokes to start_depth, then returns to origin ──────

    #[tokio::test(start_paused = true)]
    async fn first_tap_strokes_to_start_depth() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(10.0); // origin at 10 mm
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(EchoControl::Start).await.unwrap();
        // Let the task process Start.
        tokio::time::advance(Duration::from_millis(1)).await;

        // Simulate a short tap: press then release within one tick window.
        state.write().await.hand_switch = true;
        one_tick().await; // rising edge detected; hold_ticks=0, reset not fired yet
        state.write().await.hand_switch = false;
        one_tick().await; // falling edge → fire stroke out

        // Should receive MoveTo(start_depth = origin + 20 = 30 mm).
        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert!((pos_mm - 30.0).abs() < 0.1, "expected 30 mm, got {pos_mm}");
                assert!((vel_mm_s - 50.0).abs() < 0.1);
            }
            o => panic!("unexpected first command: {o:?}"),
        }

        // 20 mm @ 50 mm/s = 400 ms + 10 ms margin = 410 ms
        tokio::time::advance(Duration::from_millis(420)).await;

        // Should receive MoveTo(work_origin = 10 mm).
        let second = cmd_rx.recv().await.unwrap();
        match second {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!(
                    (pos_mm - 10.0).abs() < 0.1,
                    "expected 10 mm (origin), got {pos_mm}"
                );
            }
            o => panic!("unexpected second command: {o:?}"),
        }

        ctrl_tx.send(EchoControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain Stop ActuatorCommand
        drop(ctrl_tx);
        let _ = h.await;
    }

    // ── Test 2: depth advances by step_mm after each complete stroke ──────────

    #[tokio::test(start_paused = true)]
    async fn depth_advances_after_each_stroke() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(EchoControl::Start).await.unwrap();
        tokio::time::advance(Duration::from_millis(1)).await;

        // First tap.
        state.write().await.hand_switch = true;
        one_tick().await;
        state.write().await.hand_switch = false;
        one_tick().await;

        // MoveTo(20 mm) — outward.
        let out1 = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(out1, ActuatorCommand::MoveTo { pos_mm, .. } if (pos_mm - 20.0).abs() < 0.1),
            "expected out to 20 mm, got {out1:?}"
        );

        // 20 mm @ 50 mm/s = 400 ms + margin.
        tokio::time::advance(Duration::from_millis(420)).await;

        // MoveTo(0 mm) — back.
        let back1 = cmd_rx.recv().await.unwrap();
        assert!(
            matches!(back1, ActuatorCommand::MoveTo { pos_mm, .. } if (pos_mm - 0.0).abs() < 0.1),
            "expected back to 0 mm, got {back1:?}"
        );

        // 20 mm @ 50 mm/s = 400 ms + margin — wait for StrokingBack to complete.
        tokio::time::advance(Duration::from_millis(420)).await;
        // Give the task one more turn: advance() yields once, and the select! loop
        // may need a second iteration to process StrokingBack after switch_ticker.
        tokio::time::advance(Duration::from_millis(1)).await;

        // Depth should now be 30 mm (20 + step 10).
        let depth = state.read().await.echo.current_depth_mm;
        assert!(
            (depth - 30.0).abs() < 0.1,
            "expected depth 30 mm after step, got {depth}"
        );
        let steps = state.read().await.echo.steps_taken;
        assert_eq!(steps, 1, "expected 1 step taken");

        ctrl_tx.send(EchoControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await;
        drop(ctrl_tx);
        let _ = h.await;
    }

    // ── Test 3: long hold resets depth to start ───────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn long_hold_resets_depth() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(5.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(EchoControl::Start).await.unwrap();
        tokio::time::advance(Duration::from_millis(1)).await;

        // Manually advance depth to simulate a few prior strokes.
        state.write().await.echo.current_depth_mm = 55.0;
        state.write().await.echo.steps_taken = 3;

        // Hold button for reset_hold_ms (2000 ms) = 20 ticks of 100 ms.
        // Rising edge on first tick.
        state.write().await.hand_switch = true;
        // Tick 1: rising edge — hold_ticks=0, reset_fired=false.
        one_tick().await;
        // Ticks 2–21: hold_ticks grows; at tick 21 hold_ticks*100 = 2000 >= reset_hold_ms.
        for _ in 0..20 {
            one_tick().await;
        }
        // Extra yield so the task's async write lock inside the reset fires.
        tokio::time::advance(Duration::from_millis(1)).await;

        // Reset should have fired: depth back to start = 5 + 20 = 25 mm.
        let depth = state.read().await.echo.current_depth_mm;
        assert!(
            (depth - 25.0).abs() < 0.1,
            "expected depth 25 mm after reset, got {depth}"
        );
        let steps = state.read().await.echo.steps_taken;
        assert_eq!(steps, 0, "expected steps_taken reset to 0");

        // Release button — no stroke should fire (reset_fired == true).
        state.write().await.hand_switch = false;
        one_tick().await;

        // No MoveTo should be in the channel from the tap path (only possibly the
        // return-to-origin from the reset, which we don't assert on here).
        // Drain any commands and verify none target the old depth (55 mm).
        // We check by consuming available commands within a short window.
        tokio::time::advance(Duration::from_millis(1)).await;
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let ActuatorCommand::MoveTo { pos_mm, .. } = &cmd {
                assert!(
                    (*pos_mm - 55.0).abs() > 0.1,
                    "should not have stroked to old depth 55 mm, got MoveTo({pos_mm})"
                );
            }
        }

        ctrl_tx.send(EchoControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await;
        drop(ctrl_tx);
        let _ = h.await;
    }
}
