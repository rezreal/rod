//! Endurance games — a family of button-gated programs that measure how long
//! the user stays in control (SSCP extension, not part of Handy FW4).
//!
//! All five games run in this one task, parameterised by [`GameKind`]; only one
//! is active at a time. They share two user inputs:
//!
//!   * the **deadman button** — the client resends `Button { down: true }` every
//!     ~50 ms while held; a gap longer than `deadman_timeout_ms` (or an explicit
//!     `down: false`) counts as released. Each game reads hold/release per its
//!     own rules; and
//!   * the **servo unlock** — with the servo off the rod moves freely by hand,
//!     and the driver's status poll still reports `position_mm`, so we can
//!     *sense* the user pushing/pulling it (used by Hold the Line and Stillness).
//!
//! Live status (`phase`, `intensity`, `level`, `score_s`) is published to
//! [`AppState::game`] for the UI; the web manual documents each game's rules.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::info;

use super::GameControl;
use crate::config::{Config, Games};
use crate::state::{ActuatorCommand, AppMode, AppState, GameKind, GamePhase};

/// Settle after a ServoOn(true) before a move is accepted (same as drill/peck).
const SERVO_SETTLE: Duration = Duration::from_millis(50);
/// Reversal margin for oscillation travel-time estimates (matches HAMP).
const REVERSAL_MARGIN: Duration = Duration::from_millis(10);

/// Why a game loop returned to the dispatcher.
enum Exit {
    /// Client sent Stop, or a game's own end condition fired.
    Stop,
    /// Client started a different game without stopping first.
    Restart(GameKind),
    /// Control channel closed — shut the task down.
    Closed,
}

/// Deadman button tracker: `held` plus the instant the heartbeat lapses.
#[derive(Default)]
struct Button {
    held: bool,
    deadline: Option<Instant>,
}

impl Button {
    fn apply(&mut self, down: bool, timeout: Duration) {
        self.held = down;
        self.deadline = down.then(|| Instant::now() + timeout);
    }
    fn expire(&mut self) {
        self.held = false;
        self.deadline = None;
    }
}

pub struct GameTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    stroke_mm: f32,
    g: Games,
    deadman: Duration,
    tick: Duration,
}

impl GameTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        let g = cfg.actuator.games.clone();
        GameTask {
            state,
            cmd_tx,
            stroke_mm: cfg.stroke_mm(),
            deadman: Duration::from_millis(g.deadman_timeout_ms.max(20)),
            tick: Duration::from_millis(g.tick_ms.max(20)),
            g,
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<GameControl>) {
        info!("games task running");
        loop {
            match ctrl_rx.recv().await {
                None => break,
                Some(GameControl::Start { kind }) => {
                    let mut k = kind;
                    loop {
                        self.begin(k).await;
                        match self.play(k, &mut ctrl_rx).await {
                            Exit::Restart(nk) => k = nk,
                            Exit::Stop => {
                                self.halt().await;
                                break;
                            }
                            Exit::Closed => {
                                self.halt().await;
                                info!("games task stopped");
                                return;
                            }
                        }
                    }
                }
                // Button/Stop while idle: nothing to do.
                Some(_) => {}
            }
        }
        info!("games task stopped");
    }

    async fn play(&self, kind: GameKind, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        match kind {
            GameKind::EdgeRecover => self.edge_recover(rx).await,
            GameKind::HoldTheLine => self.hold_the_line(rx).await,
            GameKind::Gauntlet => self.gauntlet(rx).await,
            GameKind::DeadmansClimb => self.deadmans_climb(rx).await,
            GameKind::Stillness => self.stillness(rx).await,
        }
    }

    // ───────────────────────────── shared helpers ─────────────────────────────

    /// Reset runtime and announce the game; servo handling is per-game.
    async fn begin(&self, kind: GameKind) {
        let mut st = self.state.write().await;
        st.game = crate::state::GameRuntime {
            active: true,
            kind: Some(kind),
            phase: GamePhase::Idle,
            intensity: 0.0,
            level: 0,
            score_s: 0.0,
            holding: false,
        };
        st.set_mode(AppMode::Game);
        info!(game = kind.as_str(), "game started");
    }

    /// Stop motion, park the rod (servo off, brake holding), clear runtime,
    /// return to Idle. The game is over here — unlike the in-game rest phases
    /// (gauntlet rest, stillness) the rod is no longer hand-driven, so we clamp
    /// it rather than free it.
    async fn halt(&self) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        let _ = self.cmd_tx.send(ActuatorCommand::Park).await;
        let mut st = self.state.write().await;
        st.game.active = false;
        st.game.phase = GamePhase::Idle;
        st.game.holding = false;
        st.set_mode(AppMode::Idle);
        info!("game halted");
    }

    async fn servo(&self, on: bool) {
        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(on)).await;
        if on {
            tokio::time::sleep(SERVO_SETTLE).await;
        }
    }

    /// Issue one oscillation stroke toward an end at the given intensity and
    /// return the estimated travel time until the reversal (HAMP-style).
    async fn stroke(&self, out: bool, intensity: f32) -> Duration {
        let v = (self.g.min_velocity_mm_s
            + intensity.clamp(0.0, 1.0) * (self.g.max_velocity_mm_s - self.g.min_velocity_mm_s))
            .max(f32::MIN_POSITIVE);
        let target_rel = if out { self.g.zone_min } else { self.g.zone_max };
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: target_rel * self.stroke_mm,
                vel_mm_s: v,
                accel_g: self.g.accel_g,
                profile: crate::config::MotionProfile::Trapezoid,
                soften: false,
            })
            .await;
        let span_mm = ((self.g.zone_max - self.g.zone_min) * self.stroke_mm).abs();
        Duration::from_millis(((span_mm / v) * 1000.0).max(1.0) as u64) + REVERSAL_MARGIN
    }

    async fn position_mm(&self) -> f32 {
        self.state.read().await.position_mm
    }

    /// Update the parts of the runtime that change every tick.
    async fn publish(&self, phase: GamePhase, intensity: f32, level: u32, score_s: f32, holding: bool) {
        let mut st = self.state.write().await;
        st.game.phase = phase;
        st.game.intensity = intensity.clamp(0.0, 1.0);
        st.game.level = level;
        st.game.score_s = score_s;
        st.game.holding = holding;
    }

    fn ticker(&self) -> tokio::time::Interval {
        let mut t = interval(self.tick);
        t.set_missed_tick_behavior(MissedTickBehavior::Skip);
        t
    }

    // ─────────────────────────── 1. Edge & Recover ────────────────────────────

    async fn edge_recover(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        self.servo(true).await;
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let mut intensity = 0.0f32;
        let mut edges = 0u32;
        let mut score = 0.0f32;
        let mut prev_held = false;
        let mut out = false;
        let mut next = Some(Instant::now());

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    Some(GameControl::Button { down }) => btn.apply(down, self.deadman),
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => btn.expire(),
                _ = sleep_until_opt(next), if next.is_some() => {
                    let d = self.stroke(out, intensity.max(0.05)).await;
                    out = !out;
                    next = Some(Instant::now() + d);
                }
                _ = tick.tick() => {
                    score += dt;
                    let phase = if btn.held {
                        intensity = (intensity + dt / self.g.edge_climb_s.max(0.1)).min(1.0);
                        GamePhase::Active
                    } else {
                        // Count an edge when releasing from a high intensity.
                        if prev_held && intensity > 0.5 { edges += 1; }
                        intensity = (intensity - self.g.edge_backoff_rate * dt).max(0.0);
                        GamePhase::Recover
                    };
                    prev_held = btn.held;
                    self.publish(phase, intensity, edges, score, btn.held).await;
                }
            }
        }
    }

    // ──────────────────────────── 2. Hold the Line ────────────────────────────

    async fn hold_the_line(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        self.servo(true).await;
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let mut score = 0.0f32;
        let mut engaged_s = 0.0f32;
        let mut lines_lost = 0u32;
        // The line the rod must not be pushed past; set on first engage.
        let mut line: Option<f32> = None;
        let target = self.g.zone_max * self.stroke_mm;

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    Some(GameControl::Button { down }) => btn.apply(down, self.deadman),
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => {
                    btn.expire();
                    // Released: relax the push and free the rod.
                    let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                    line = None;
                }
                _ = tick.tick() => {
                    if !btn.held {
                        self.publish(GamePhase::Recover, 0.0, lines_lost, score, false).await;
                        continue;
                    }
                    let pos = self.position_mm().await;
                    let line = line.get_or_insert(pos);
                    engaged_s += dt;
                    score += dt;
                    // Thrust ramps with engaged time.
                    let frac = (engaged_s / self.g.hold_push_ramp_s.max(0.1)).min(1.0);
                    let push = self.g.hold_push_start_pct
                        + ((self.g.hold_push_max_pct - self.g.hold_push_start_pct) as f32 * frac) as u16;
                    let _ = self.cmd_tx.send(ActuatorCommand::MovePush {
                        pos_mm: target,
                        vel_mm_s: self.g.hold_push_velocity_mm_s,
                        accel_g: self.g.accel_g,
                        push_current_pct: push,
                    }).await;
                    // Ground lost if the rod is driven past the line.
                    let mut phase = GamePhase::Hold;
                    if pos > *line + self.g.hold_line_advance_mm {
                        lines_lost += 1;
                        *line = pos; // re-anchor forward
                        phase = GamePhase::Slip;
                    }
                    self.publish(phase, frac, lines_lost, score, true).await;
                }
            }
        }
    }

    // ───────────────────────────── 3. Gauntlet ────────────────────────────────

    async fn gauntlet(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let mut score = 0.0f32;
        let mut completed = 0u32;
        let mut prev_held = false;
        // Phase machine: resting (servo free), or working (oscillating).
        let mut working = false;
        let mut phase_left = self.g.gauntlet_rest_s; // time left in current phase
        let mut out = false;
        let mut next: Option<Instant> = None;
        self.servo(false).await; // start resting, rod free

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    Some(GameControl::Button { down }) => btn.apply(down, self.deadman),
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => btn.expire(),
                _ = sleep_until_opt(next), if next.is_some() && working => {
                    let d = self.stroke(out, 0.85).await; // intervals run hard
                    out = !out;
                    next = Some(Instant::now() + d);
                }
                _ = tick.tick() => {
                    phase_left -= dt;
                    if working {
                        score += dt;
                        // Releasing mid-work aborts the interval (no credit).
                        if !btn.held {
                            working = false;
                            phase_left = self.g.gauntlet_rest_s;
                            next = None;
                            self.servo(false).await;
                        } else if phase_left <= 0.0 {
                            // Interval completed.
                            completed += 1;
                            working = false;
                            phase_left = self.g.gauntlet_rest_s;
                            next = None;
                            self.servo(false).await;
                        }
                        self.publish(GamePhase::Active, 0.85, completed, score, btn.held).await;
                    } else {
                        // Resting: a fresh press starts the next interval.
                        let ready = btn.held && !prev_held;
                        if ready {
                            working = true;
                            phase_left = self.g.gauntlet_work_s
                                + completed as f32 * self.g.gauntlet_work_growth_s;
                            self.servo(true).await;
                            next = Some(Instant::now());
                        } else if phase_left <= 0.0 {
                            // No-show: the gauntlet ends.
                            return Exit::Stop;
                        }
                        self.publish(GamePhase::Rest, 0.0, completed, score, btn.held).await;
                    }
                    prev_held = btn.held;
                }
            }
        }
    }

    // ───────────────────────── 4. Deadman's Climb ─────────────────────────────

    async fn deadmans_climb(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        self.servo(true).await;
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let n = self.g.climb_checkpoints.max(1);
        let mut intensity = 0.0f32;
        let mut level = 0u32; // highest banked checkpoint
        let mut score = 0.0f32;
        let mut out = false;
        let mut next = Some(Instant::now());

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    Some(GameControl::Button { down }) => btn.apply(down, self.deadman),
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => btn.expire(),
                _ = sleep_until_opt(next), if next.is_some() => {
                    let d = self.stroke(out, intensity.max(0.05)).await;
                    out = !out;
                    next = Some(Instant::now() + d);
                }
                _ = tick.tick() => {
                    score += dt;
                    let floor = level as f32 / n as f32; // banked intensity floor
                    let phase = if btn.held {
                        intensity = (intensity + dt / self.g.climb_total_s.max(0.1)).min(1.0);
                        // Bank a checkpoint when crossing the next threshold.
                        while level < n && intensity >= (level + 1) as f32 / n as f32 {
                            level += 1;
                        }
                        GamePhase::Active
                    } else {
                        // Lapse: fall back only to the last banked checkpoint.
                        intensity = (intensity - self.g.edge_backoff_rate * dt).max(floor);
                        GamePhase::Recover
                    };
                    self.publish(phase, intensity, level, score, btn.held).await;
                }
            }
        }
    }

    // ───────────────────────────── 5. Stillness ───────────────────────────────

    async fn stillness(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        // Servo off: the rod is free in the user's hand; we only sense position
        // and tug it occasionally.
        self.servo(false).await;
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let mut score = 0.0f32;
        let center = self.position_mm().await;
        let mut nudge_at = Some(Instant::now() + self.next_nudge_gap());

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    Some(GameControl::Button { down }) => btn.apply(down, self.deadman),
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => btn.expire(),
                _ = sleep_until_opt(nudge_at), if nudge_at.is_some() && btn.held => {
                    // A tug: briefly drive the rod off-center, then free it again.
                    let pos = self.position_mm().await;
                    let dir = if rand::random::<bool>() { 1.0 } else { -1.0 };
                    let target = (pos + dir * self.g.stillness_nudge_mm).clamp(0.0, self.stroke_mm);
                    self.servo(true).await;
                    let _ = self.cmd_tx.send(ActuatorCommand::MoveTo {
                        pos_mm: target,
                        vel_mm_s: self.g.max_velocity_mm_s * 0.4,
                        accel_g: self.g.accel_g,
                        profile: crate::config::MotionProfile::Trapezoid,
                        soften: false,
                    }).await;
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                    nudge_at = Some(Instant::now() + self.next_nudge_gap());
                }
                _ = tick.tick() => {
                    if !btn.held {
                        self.publish(GamePhase::Recover, 0.0, 0, score, false).await;
                        continue;
                    }
                    let dev = (self.position_mm().await - center).abs();
                    let frac = dev / self.g.stillness_tolerance_mm.max(0.1);
                    if dev > self.g.stillness_tolerance_mm {
                        // Moved too far — round over; report the time survived.
                        self.publish(GamePhase::Slip, 1.0, 0, score, true).await;
                        return Exit::Stop;
                    }
                    score += dt;
                    self.publish(GamePhase::Hold, frac, 0, score, true).await;
                }
            }
        }
    }

    fn next_nudge_gap(&self) -> Duration {
        let lo = self.g.stillness_nudge_min_ms;
        let hi = self.g.stillness_nudge_max_ms.max(lo + 1);
        let span = hi - lo;
        Duration::from_millis(lo + (rand::random::<f64>() * span as f64) as u64)
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
    use crate::state::GameKind;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            [actuator.games]
            tick_ms = 100
            deadman_timeout_ms = 150
            edge_climb_s = 1.0
            gauntlet_rest_s = 2.0
            gauntlet_work_s = 1.0
        "#,
        )
        .unwrap()
    }

    fn spawn() -> (
        Arc<RwLock<AppState>>,
        mpsc::Sender<GameControl>,
        mpsc::Receiver<ActuatorCommand>,
        tokio::task::JoinHandle<()>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let h = tokio::spawn(GameTask::new(state.clone(), cmd_tx, &cfg()).run(ctrl_rx));
        (state, ctrl_tx, cmd_rx, h)
    }

    /// Advance one heartbeat: optionally resend the held button (clients resend
    /// every ~50ms; the deadman is 150ms), step the clock, let the task run, and
    /// drain the command channel so it never blocks.
    async fn beat(ctrl: &mpsc::Sender<GameControl>, cmd_rx: &mut mpsc::Receiver<ActuatorCommand>, hold: bool) {
        if hold {
            ctrl.send(GameControl::Button { down: true }).await.unwrap();
        }
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        while cmd_rx.try_recv().is_ok() {}
    }

    #[tokio::test(start_paused = true)]
    async fn edge_recover_climbs_while_held() {
        let (state, ctrl, mut cmd_rx, h) = spawn();
        ctrl.send(GameControl::Start { kind: GameKind::EdgeRecover }).await.unwrap();

        // Hold the button across ~10 heartbeats (climb_s = 1.0).
        for _ in 0..10 {
            beat(&ctrl, &mut cmd_rx, true).await;
        }
        let g = state.read().await.game.clone();
        assert!(g.active && g.kind == Some(GameKind::EdgeRecover));
        assert!(g.intensity > 0.3, "should be climbing: {}", g.intensity);
        assert!(g.holding);

        ctrl.send(GameControl::Stop).await.unwrap();
        drop(ctrl);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn gauntlet_starts_resting_and_works_on_press() {
        let (state, ctrl, mut cmd_rx, h) = spawn();
        ctrl.send(GameControl::Start { kind: GameKind::Gauntlet }).await.unwrap();

        // Settle into the rest phase (no button held).
        for _ in 0..2 {
            beat(&ctrl, &mut cmd_rx, false).await;
        }
        assert_eq!(state.read().await.game.phase, GamePhase::Rest);

        // Holding the button = "ready" → starts a work interval within a tick or two.
        for _ in 0..4 {
            beat(&ctrl, &mut cmd_rx, true).await;
        }
        assert_eq!(state.read().await.game.phase, GamePhase::Active);

        ctrl.send(GameControl::Stop).await.unwrap();
        drop(ctrl);
        let _ = h.await;
    }
}
