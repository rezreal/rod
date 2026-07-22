//! Tempo — rhythm-tapped oscillation.
//!
//! Machine is still until the user taps out a rhythm with the hand switch.
//! Each tap (short press < stop_hold_ms) records the inter-tap interval as the
//! period; oscillation begins once a period is established. A long hold
//! (≥ stop_hold_ms) stops oscillation and clears the tempo. If taps cease for
//! more than `timeout_periods × period_ms`, oscillation auto-stops.
//!
//! Stroke timing:
//!   speed = stroke_span / (period_ms / 2 / 1000)   clamped to max_velocity_mm_s
//!   Each half-stroke (outward or return) takes period_ms/2.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

use super::TempoControl;
use crate::config::{Config, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState};

/// How often hand-switch state is polled.
const SWITCH_TICK: Duration = Duration::from_millis(100);
/// Margin added to estimated travel time before a reversal.
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);

pub struct TempoTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    max_mm: f32,
    max_velocity_mm_s: f32,
    min_period_ms: u64,
    max_period_ms: u64,
    depth_mm: f32,
    accel_g: f32,
    stop_hold_ms: u64,
    timeout_periods: f32,
    profile: MotionProfile,
}

impl TempoTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        TempoTask {
            state,
            cmd_tx,
            max_mm: cfg.max_position_mm(),
            max_velocity_mm_s: cfg.actuator.limits.max_velocity_mm_s,
            min_period_ms: cfg.actuator.tempo.min_period_ms,
            max_period_ms: cfg.actuator.tempo.max_period_ms,
            depth_mm: cfg.actuator.tempo.depth_mm,
            accel_g: cfg.actuator.tempo.accel_g,
            stop_hold_ms: cfg.actuator.tempo.stop_hold_ms,
            timeout_periods: cfg.actuator.tempo.timeout_periods,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
        }
    }

    /// Compute stroke speed from period and span. Speed = span / half_period_s,
    /// clamped to max_velocity_mm_s.
    fn stroke_speed(&self, period_ms: u64, span: f32) -> f32 {
        let half_period_s = period_ms as f32 / 2.0 / 1000.0;
        let speed = span / half_period_s;
        speed.min(self.max_velocity_mm_s).max(f32::MIN_POSITIVE)
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<TempoControl>) {
        info!("tempo task running");
        let mut switch_ticker = tokio::time::interval(SWITCH_TICK);
        switch_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        // Oscillation state.
        let mut out = false; // false = going toward target, true = returning to origin
        let mut next: Option<Instant> = None;
        let mut work_origin = 0.0f32;
        let mut target_mm = 0.0f32;

        // Tempo / tap state (local, not in AppState).
        let mut period_ms: u64 = 0; // 0 = no tempo established
        let mut last_tap: Option<Instant> = None;

        // Edge-detection state.
        let mut prev_held = false;
        let mut press_start: Option<Instant> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,

                    Some(TempoControl::Start) => {
                        work_origin = {
                            let st = self.state.read().await;
                            match st.work_origin_mm {
                                Some(o) => o,
                                None => {
                                    warn!("tempo: no calibration; work origin defaulting to 0 mm");
                                    0.0
                                }
                            }
                        };
                        target_mm = (work_origin + self.depth_mm).min(self.max_mm);
                        {
                            let mut st = self.state.write().await;
                            st.tempo.active = true;
                            st.tempo.period_ms = 0;
                            st.set_mode(AppMode::Tempo);
                        }
                        out = false;
                        next = None;
                        period_ms = 0;
                        last_tap = None;
                        prev_held = false;
                        press_start = None;
                        info!(work_origin, target_mm, "tempo: started");
                    }

                    Some(TempoControl::Stop) => {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        {
                            let mut st = self.state.write().await;
                            st.tempo.active = false;
                            st.tempo.period_ms = 0;
                            st.set_mode(AppMode::Idle);
                        }
                        out = false;
                        next = None;
                        period_ms = 0;
                        last_tap = None;
                        prev_held = false;
                        press_start = None;
                        info!("tempo: stopped");
                    }
                },

                _ = switch_ticker.tick() => {
                    let active = self.state.read().await.tempo.active;
                    if !active { continue; }

                    let held = self.state.read().await.hand_switch;

                    // Rising edge: button just pressed.
                    if held && !prev_held {
                        press_start = Some(Instant::now());
                    }

                    // Falling edge: button released.
                    if !held && prev_held {
                        let held_ms = press_start
                            .map_or(0, |t| Instant::now().duration_since(t).as_millis() as u64);
                        press_start = None;

                        if held_ms >= self.stop_hold_ms {
                            // Long hold: stop oscillation, clear tempo.
                            info!("tempo: long hold — stopping oscillation");
                            if next.is_some() {
                                let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                                next = None;
                            }
                            period_ms = 0;
                            last_tap = None;
                            {
                                let mut st = self.state.write().await;
                                st.tempo.period_ms = 0;
                            }
                        } else {
                            // Short tap (rising-edge tap).
                            let now = Instant::now();

                            if let Some(prev_tap) = last_tap {
                                let elapsed_ms = now
                                    .duration_since(prev_tap)
                                    .as_millis()
                                    .min(u64::MAX as u128) as u64;
                                let new_period =
                                    elapsed_ms.clamp(self.min_period_ms, self.max_period_ms);
                                period_ms = new_period;
                                {
                                    let mut st = self.state.write().await;
                                    st.tempo.period_ms = period_ms;
                                }
                                info!(period_ms, "tempo: period set");

                                // Start oscillation if not already running.
                                if next.is_none() {
                                    out = false;
                                    next = Some(Instant::now());
                                    info!("tempo: oscillation starting");
                                }
                                // If already running, don't change timing — let it continue.
                            }

                            last_tap = Some(now);
                        }

                    }

                    prev_held = held;
                },

                _ = sleep_until_opt(next), if next.is_some() => {
                    // Emit the next stroke.
                    if period_ms == 0 {
                        // No period: just cancel.
                        next = None;
                        continue;
                    }

                    let span = target_mm - work_origin;
                    if span <= 0.0 {
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        next = None;
                        continue;
                    }

                    let speed = self.stroke_speed(period_ms, span);
                    let pos = if out { work_origin } else { target_mm };
                    let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                        pos_mm: pos,
                        vel_mm_s: speed,
                        accel_g: self.accel_g,
                        profile: self.profile,
                                    soften: false,
                    }).await;

                    let half_period = Duration::from_millis(period_ms / 2) + REVERSAL_MARGIN;
                    out = !out;
                    next = Some(Instant::now() + half_period);

                    // Auto-stop check: if taps have ceased for > timeout_periods × period_ms.
                    if let Some(tap_instant) = last_tap {
                        let elapsed = Instant::now().duration_since(tap_instant);
                        let timeout =
                            Duration::from_millis((period_ms as f32 * self.timeout_periods) as u64);
                        if elapsed > timeout {
                            info!("tempo: auto-stop — taps ceased");
                            let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                            next = None;
                            period_ms = 0;
                            last_tap = None;
                            out = false;
                            {
                                let mut st = self.state.write().await;
                                st.tempo.period_ms = 0;
                            }
                        }
                    }
                }
            }
        }

        info!("tempo task stopped");
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
            [actuator.tempo]
            min_period_ms    = 400
            max_period_ms    = 4000
            depth_mm         = 80.0
            accel_g          = 0.2
            stop_hold_ms     = 1000
            timeout_periods  = 2.0
        "#,
        )
        .unwrap()
    }

    fn make(
        cfg: &Config,
    ) -> (
        TempoTask,
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        (TempoTask::new(state.clone(), cmd_tx, cfg), state, cmd_rx)
    }

    /// Two taps 500ms apart establish a 500ms period; verify the first stroke
    /// fires toward target_mm and then returns to work_origin.
    #[tokio::test(start_paused = true)]
    async fn two_taps_establish_tempo() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(20.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TempoControl::Start).await.unwrap();
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        // First tap: press and release quickly (50ms hold < 1000ms stop_hold_ms).
        // yield_now() after each advance guarantees the task sees the correct
        // hand_switch state for that tick before it changes.
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // rising edge
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // falling edge / tap 1

        // After first tap, last_tap is set but no period yet (need 2 taps).
        // No stroke command should have fired.
        assert!(
            cmd_rx.try_recv().is_err(),
            "no stroke should fire after first tap alone"
        );

        // Second tap 500ms after first: press and release.
        // We've already advanced 200ms; advance another 300ms to reach 500ms from first tap.
        tokio::time::advance(Duration::from_millis(300)).await;
        tokio::task::yield_now().await;
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // rising edge 2
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // falling edge / tap 2 — period established

        // Period should now be ~500ms, oscillation fires immediately (next = Instant::now()).
        // Advance a tiny bit to let the stroke arm fire.
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        // First stroke: outward toward target_mm = 20 + 80 = 100 mm.
        let first = cmd_rx.recv().await.unwrap();
        match first {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!(
                    (pos_mm - 100.0).abs() < 0.5,
                    "expected target ~100 mm, got {pos_mm}"
                );
            }
            o => panic!("expected MoveTo, got {o:?}"),
        }

        // Advance half a period (250ms + 10ms margin) to trigger the return stroke.
        tokio::time::advance(Duration::from_millis(260)).await;

        let second = cmd_rx.recv().await.unwrap();
        match second {
            ActuatorCommand::MoveTo { pos_mm, .. } => {
                assert!(
                    (pos_mm - 20.0).abs() < 0.5,
                    "expected origin ~20 mm, got {pos_mm}"
                );
            }
            o => panic!("expected MoveTo (return), got {o:?}"),
        }

        ctrl_tx.send(TempoControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain Stop
        drop(ctrl_tx);
        let _ = h.await;
    }

    /// After establishing a tempo, holding the button for ≥ stop_hold_ms should
    /// stop oscillation; no new MoveTo should appear after the Stop.
    #[tokio::test(start_paused = true)]
    async fn long_hold_stops_tempo() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(0.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TempoControl::Start).await.unwrap();
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        // First tap.
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // rising edge
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // falling edge / tap 1

        // Second tap (500ms later): establishes 500ms period.
        tokio::time::advance(Duration::from_millis(300)).await;
        tokio::task::yield_now().await;
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // rising edge 2
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // falling edge / tap 2 — period established

        // Let the first stroke fire.
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        let _first_stroke = cmd_rx.recv().await.unwrap();
        assert!(matches!(_first_stroke, ActuatorCommand::MoveTo { .. }));

        // Now press and hold for stop_hold_ms (1000ms = 10 ticks of 100ms).
        state.write().await.hand_switch = true;
        // Advance 11 ticks to exceed stop_hold_ms, yielding after each so the
        // task processes the tick while the button is still held.
        for _ in 0..11 {
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
        }
        // Release the button — long hold is detected on the falling edge.
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;

        // The long-hold should have issued a Stop command. Drain any MoveTo commands
        // from ongoing oscillation that arrived before the Stop.
        let stop_cmd = loop {
            let cmd = cmd_rx.recv().await.unwrap();
            if matches!(cmd, ActuatorCommand::Stop) {
                break cmd;
            }
        };
        assert!(matches!(stop_cmd, ActuatorCommand::Stop));

        // Advance well beyond another period — no new MoveTo should appear.
        tokio::time::advance(Duration::from_millis(1000)).await;
        assert!(
            cmd_rx.try_recv().is_err(),
            "no MoveTo should fire after long hold cleared the tempo"
        );

        // Check state reflects period cleared.
        assert_eq!(state.read().await.tempo.period_ms, 0);

        ctrl_tx.send(TempoControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await; // drain Stop
        drop(ctrl_tx);
        let _ = h.await;
    }

    /// After establishing a tempo, if no taps arrive for > timeout_periods × period_ms,
    /// oscillation should auto-stop.
    #[tokio::test(start_paused = true)]
    async fn auto_stop_when_taps_cease() {
        let cfg = cfg();
        let (task, state, mut cmd_rx) = make(&cfg);
        {
            let mut st = state.write().await;
            st.work_origin_mm = Some(10.0);
        }
        let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
        let h = tokio::spawn(task.run(ctrl_rx));

        ctrl_tx.send(TempoControl::Start).await.unwrap();
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        // First tap.
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // rising edge
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // falling edge / tap 1

        // Second tap 500ms later: period = 500ms.
        tokio::time::advance(Duration::from_millis(300)).await;
        tokio::task::yield_now().await;
        state.write().await.hand_switch = true;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // rising edge 2
        state.write().await.hand_switch = false;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await; // falling edge / tap 2 — period established

        // Let first stroke fire.
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        let _first = cmd_rx.recv().await.unwrap();
        assert!(matches!(_first, ActuatorCommand::MoveTo { .. }));

        // Advance 3 × period_ms (1500ms) without tapping — auto-stop should trigger.
        // Each half-period fires a stroke; timeout_periods=2.0 so timeout = 1000ms
        // from last_tap. After 3 half-periods (1500ms total), well past timeout.
        //
        // We need to advance past timeout from last_tap. Last tap was at roughly
        // T=800ms (100+100+300+100+100+100 = 800ms). Timeout = 500*2 = 1000ms.
        // So auto-stop should fire after T=1800ms. Advance in half-period steps.
        tokio::time::advance(Duration::from_millis(260)).await; // return stroke
        let _second = cmd_rx.recv().await.unwrap();
        tokio::time::advance(Duration::from_millis(260)).await; // outward stroke
        let _third = cmd_rx.recv().await.unwrap();
        tokio::time::advance(Duration::from_millis(260)).await; // return stroke — this one should auto-stop

        // The auto-stop logic fires during the stroke arm, which may emit a MoveTo
        // first then a Stop. Drain commands looking for the Stop.
        let mut found_stop = false;
        // Drain up to a few commands.
        for _ in 0..5 {
            match cmd_rx.try_recv() {
                Ok(ActuatorCommand::Stop) => {
                    found_stop = true;
                    break;
                }
                Ok(_) => {} // may be a MoveTo emitted before auto-stop check
                Err(_) => break,
            }
        }
        // If not yet found, advance one more step and drain.
        if !found_stop {
            tokio::time::advance(Duration::from_millis(260)).await;
            for _ in 0..5 {
                match cmd_rx.try_recv() {
                    Ok(ActuatorCommand::Stop) => {
                        found_stop = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }

        assert!(
            found_stop,
            "expected auto-stop after taps ceased for > 2× period"
        );

        // After auto-stop: no further MoveTo commands.
        tokio::time::advance(Duration::from_millis(1000)).await;
        let mut extra_move = false;
        while let Ok(cmd) = cmd_rx.try_recv() {
            if matches!(cmd, ActuatorCommand::MoveTo { .. }) {
                extra_move = true;
                break;
            }
        }
        assert!(!extra_move, "no MoveTo should fire after auto-stop");

        ctrl_tx.send(TempoControl::Stop).await.unwrap();
        let _ = cmd_rx.recv().await;
        drop(ctrl_tx);
        let _ = h.await;
    }
}
