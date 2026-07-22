//! Endurance games — a family of button-gated programs that measure how long
//! the user stays in control (SSCP extension, not part of Handy FW4).
//!
//! All four games run in this one task, parameterised by [`GameKind`]; only one
//! is active at a time. They share two user inputs:
//!
//!   * the **deadman button** — the client resends `Button { down: true }` every
//!     ~50 ms while held; a gap longer than `deadman_timeout_ms` (or an explicit
//!     `down: false`) counts as released. Each game reads hold/release per its
//!     own rules; Stillness is the one exception — once armed it plays on its
//!     own, since holding a button fights the "stay still" premise; and
//!   * the **servo unlock** — with the servo off the rod moves freely by hand,
//!     and the driver's status poll still reports `position_mm`, so we can
//!     *sense* the user pushing/pulling it (used by Stillness).
//!
//! Live status (`phase`, `intensity`, `level`, `duration_s`) is published to
//! [`AppState::game`] for the UI; the web manual documents each game's rules.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::info;

use super::GameControl;
use crate::config::{Config, Games};
use crate::devices::PiuPiuControl;
use crate::state::{ActuatorCommand, AppMode, AppState, GameKind, GamePhase};

/// Celebration pulse pattern fired on a game win (see [`GameTask::celebrate_win`]).
const TADA_PULSE: Duration = Duration::from_millis(150);
const TADA_GAP: Duration = Duration::from_millis(150);
const TADA_PULSES: u32 = 3;

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

/// Why the arming stage ([`GameTask::await_ready`]) returned.
enum ArmResult {
    /// The ready gesture completed and the start delay elapsed: begin `kind`.
    Ready(GameKind),
    /// Client sent Stop (or a different game's ready gesture also aborted
    /// this way) before the gesture completed.
    Stop,
    /// Control channel closed — shut the task down.
    Closed,
}

/// Counts hardware taps toward the triple-tap "I am ready" gesture. A tap
/// more than `window` after the first one restarts the count at 1, so a slow
/// stray press can't silently accumulate toward an unintended start.
#[derive(Default)]
struct ReadyGate {
    count: u32,
    first: Option<Instant>,
}

impl ReadyGate {
    /// Register a tap; returns `true` once `required` taps have landed
    /// within `window` of the first tap in the current run.
    fn tap(&mut self, required: u32, window: Duration) -> bool {
        let now = Instant::now();
        match self.first {
            Some(t) if now.duration_since(t) <= window => self.count += 1,
            _ => {
                self.first = Some(now);
                self.count = 1;
            }
        }
        self.count >= required
    }
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
    piupiu: mpsc::Sender<PiuPiuControl>,
    stroke_mm: f32,
    g: Games,
    deadman: Duration,
    tick: Duration,
}

impl GameTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        piupiu: mpsc::Sender<PiuPiuControl>,
        cfg: &Config,
    ) -> Self {
        let g = cfg.actuator.games.clone();
        GameTask {
            state,
            cmd_tx,
            piupiu,
            stroke_mm: cfg.stroke_mm(),
            deadman: Duration::from_millis(g.deadman_timeout_ms.max(20)),
            tick: Duration::from_millis(g.tick_ms.max(20)),
            g,
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<GameControl>) {
        info!("games task running");
        'dispatch: loop {
            match ctrl_rx.recv().await {
                None => break,
                Some(GameControl::Start { kind }) => {
                    let mut k = kind;
                    loop {
                        k = match self.await_ready(k, &mut ctrl_rx).await {
                            ArmResult::Ready(k) => k,
                            ArmResult::Stop => continue 'dispatch,
                            ArmResult::Closed => {
                                info!("games task stopped");
                                return;
                            }
                        };
                        self.begin(k).await;
                        match self.play(k, &mut ctrl_rx).await {
                            // Switching games mid-play still requires its own
                            // ready gesture — arm again rather than jumping
                            // straight in.
                            Exit::Restart(nk) => k = nk,
                            Exit::Stop => {
                                self.halt().await;
                                continue 'dispatch;
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

    /// Arm `kind` and block until the client either signals ready (three
    /// hardware taps within [`Games::ready_window_ms`], then a
    /// [`Games::ready_delay_ms`] settle) or aborts (Stop / channel close). A
    /// fresh `Start` while arming re-arms for the new kind instead of
    /// stacking gestures.
    async fn await_ready(&self, kind: GameKind, rx: &mut mpsc::Receiver<GameControl>) -> ArmResult {
        let mut k = kind;
        'arm: loop {
            self.arm(k).await;
            let mut gate = ReadyGate::default();
            let mut go_at: Option<Instant> = None;

            loop {
                tokio::select! {
                    ctrl = rx.recv() => match ctrl {
                        None => return ArmResult::Closed,
                        Some(GameControl::Stop) => {
                            self.disarm().await;
                            return ArmResult::Stop;
                        }
                        Some(GameControl::Start { kind: nk }) => {
                            k = nk;
                            continue 'arm;
                        }
                        Some(GameControl::HardwareTap) if go_at.is_none() => {
                            let ready = gate.tap(
                                self.g.ready_taps.max(1),
                                Duration::from_millis(self.g.ready_window_ms),
                            );
                            self.publish_armed(gate.count).await;
                            if ready {
                                go_at = Some(Instant::now() + Duration::from_millis(self.g.ready_delay_ms));
                            }
                        }
                        // Deadman heartbeats and late taps don't matter before play starts.
                        Some(_) => {}
                    },
                    _ = sleep_until_opt(go_at), if go_at.is_some() => return ArmResult::Ready(k),
                }
            }
        }
    }

    async fn play(&self, kind: GameKind, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        match kind {
            GameKind::EdgeRecover => self.edge_recover(rx).await,
            GameKind::Gauntlet => self.gauntlet(rx).await,
            GameKind::DeadmansClimb => self.deadmans_climb(rx).await,
            GameKind::Stillness => self.stillness(rx).await,
        }
    }

    // ───────────────────────────── shared helpers ─────────────────────────────

    /// Enter the Armed phase for `kind`: game mode is set and the runtime is
    /// visible to the UI, but no motion starts — [`GameRuntime::level`] shows
    /// the hardware tap count as the ready gesture builds.
    async fn arm(&self, kind: GameKind) {
        let mut st = self.state.write().await;
        st.game = crate::state::GameRuntime {
            active: true,
            kind: Some(kind),
            phase: GamePhase::Armed,
            intensity: 0.0,
            level: 0,
            duration_s: 0.0,
            holding: false,
        };
        st.set_mode(AppMode::Game);
        info!(game = kind.as_str(), "game armed, awaiting ready signal");
    }

    /// Abort arming (client sent Stop before the gesture completed). No
    /// motion has started yet, so there's nothing to halt beyond the runtime.
    async fn disarm(&self) {
        let mut st = self.state.write().await;
        st.game.active = false;
        st.game.phase = GamePhase::Idle;
        st.set_mode(AppMode::Idle);
    }

    /// Update the tap count shown to the UI while arming.
    async fn publish_armed(&self, taps: u32) {
        self.state.write().await.game.level = taps;
    }

    /// Reset runtime and announce the game; servo handling is per-game. Only
    /// reached after the ready gesture (see [`GameTask::await_ready`]).
    async fn begin(&self, kind: GameKind) {
        let mut st = self.state.write().await;
        st.game = crate::state::GameRuntime {
            active: true,
            kind: Some(kind),
            phase: GamePhase::Idle,
            intensity: 0.0,
            level: 0,
            duration_s: 0.0,
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
        let target_rel = if out {
            self.g.zone_min
        } else {
            self.g.zone_max
        };
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

    /// Drive to `pos_mm` under motor power and wait out the estimated travel
    /// time, or bail early on Stop / a fresh Start. Used by Stillness to
    /// reach its round-starting position before sensing begins.
    async fn move_to_and_settle(
        &self,
        pos_mm: f32,
        vel_mm_s: f32,
        rx: &mut mpsc::Receiver<GameControl>,
    ) -> Option<Exit> {
        let cur = self.position_mm().await;
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm,
                vel_mm_s,
                accel_g: self.g.accel_g,
                profile: crate::config::MotionProfile::Trapezoid,
                soften: false,
            })
            .await;
        let dist_mm = (pos_mm - cur).abs();
        let travel = Duration::from_millis(((dist_mm / vel_mm_s.max(f32::MIN_POSITIVE)) * 1000.0).max(1.0) as u64)
            + REVERSAL_MARGIN;
        let deadline = Instant::now() + travel;

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Some(Exit::Closed),
                    Some(GameControl::Stop) => return Some(Exit::Stop),
                    Some(GameControl::Start { kind }) => return Some(Exit::Restart(kind)),
                    // Nothing else matters while settling into position.
                    Some(_) => {}
                },
                _ = tokio::time::sleep_until(deadline) => return None,
            }
        }
    }

    async fn position_mm(&self) -> f32 {
        self.state.read().await.position_mm
    }

    /// Update the parts of the runtime that change every tick.
    async fn publish(
        &self,
        phase: GamePhase,
        intensity: f32,
        level: u32,
        duration_s: f32,
        holding: bool,
    ) {
        let entering_win = {
            let mut st = self.state.write().await;
            let entering_win = phase == GamePhase::Win && st.game.phase != GamePhase::Win;
            st.game.phase = phase;
            st.game.intensity = intensity.clamp(0.0, 1.0);
            st.game.level = level;
            st.game.duration_s = duration_s;
            st.game.holding = holding;
            entering_win
        };
        if entering_win {
            self.celebrate_win();
        }
    }

    /// Fire a "tada" — three separate PiuPiu squirts — on the transition into
    /// a win. Runs in the background so it never delays the game loop; a
    /// no-op (silently) if no PiuPiu is connected.
    fn celebrate_win(&self) {
        let state = self.state.clone();
        let piupiu = self.piupiu.clone();
        tokio::spawn(async move {
            if !state.read().await.piupiu.connected {
                return;
            }
            for _ in 0..TADA_PULSES {
                let _ = piupiu.send(PiuPiuControl::Squirt { active: true }).await;
                tokio::time::sleep(TADA_PULSE).await;
                let _ = piupiu.send(PiuPiuControl::Squirt { active: false }).await;
                tokio::time::sleep(TADA_GAP).await;
            }
        });
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
        let mut duration = 0.0f32;
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
                    // The ready gesture already happened; ignore late taps.
                    Some(GameControl::HardwareTap) => {}
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => btn.expire(),
                _ = sleep_until_opt(next), if next.is_some() => {
                    let d = self.stroke(out, intensity.max(0.05)).await;
                    out = !out;
                    next = Some(Instant::now() + d);
                }
                _ = tick.tick() => {
                    duration += dt;
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
                    self.publish(phase, intensity, edges, duration, btn.held).await;
                }
            }
        }
    }

    // ───────────────────────────── 2. Gauntlet ────────────────────────────────

    async fn gauntlet(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let mut duration = 0.0f32;
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
                    // The ready gesture already happened; ignore late taps.
                    Some(GameControl::HardwareTap) => {}
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
                        duration += dt;
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
                        self.publish(GamePhase::Active, 0.85, completed, duration, btn.held).await;
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
                        self.publish(GamePhase::Rest, 0.0, completed, duration, btn.held).await;
                    }
                    prev_held = btn.held;
                }
            }
        }
    }

    // ───────────────────────── 3. Deadman's Climb ─────────────────────────────

    async fn deadmans_climb(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        self.servo(true).await;
        let mut tick = self.ticker();
        let mut btn = Button::default();
        let dt = self.tick.as_secs_f32();
        let n = self.g.climb_checkpoints.max(1);
        let mut intensity = 0.0f32;
        let mut level = 0u32; // highest banked checkpoint
        let mut duration = 0.0f32;
        let mut out = false;
        let mut next = Some(Instant::now());

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    Some(GameControl::Button { down }) => btn.apply(down, self.deadman),
                    // The ready gesture already happened; ignore late taps.
                    Some(GameControl::HardwareTap) => {}
                },
                _ = sleep_until_opt(btn.deadline), if btn.deadline.is_some() => btn.expire(),
                _ = sleep_until_opt(next), if next.is_some() => {
                    let d = self.stroke(out, intensity.max(0.05)).await;
                    out = !out;
                    next = Some(Instant::now() + d);
                }
                _ = tick.tick() => {
                    duration += dt;
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
                    // Top checkpoint banked: the climb is won, regardless of
                    // whether the button is still held at this instant.
                    if level >= n {
                        self.publish(GamePhase::Win, 1.0, level, duration, btn.held).await;
                        return Exit::Stop;
                    }
                    self.publish(phase, intensity, level, duration, btn.held).await;
                }
            }
        }
    }

    // ───────────────────────────── 4. Stillness ───────────────────────────────

    /// Unlike the other four games, Stillness has no deadman-hold requirement:
    /// asking the user to keep a button pressed while trying to stay still
    /// works against the point of the game, and there's no motor-driven
    /// motion here for a deadman to guard against. The round instead opens
    /// with a single motor-driven move to its starting position (see
    /// [`GameTask::move_to_and_settle`]) so the player has a known point to
    /// hold rather than wherever the rod happened to be left; once settled
    /// there, play runs on its own until Stop or the last life is spent.
    async fn stillness(&self, rx: &mut mpsc::Receiver<GameControl>) -> Exit {
        self.servo(true).await;
        let start_pos = self.g.stillness_start_pct.clamp(0.0, 1.0) * self.stroke_mm;
        if let Some(exit) = self
            .move_to_and_settle(start_pos, self.g.min_velocity_mm_s, rx)
            .await
        {
            return exit;
        }

        // Servo off: the rod is free in the user's hand from here; we only
        // sense position. Drift past tolerance costs a life and a
        // micro-vibration warning, not an instant loss — the rod itself
        // never tugs or drives the user again once settled.
        self.servo(false).await;
        let mut tick = self.ticker();
        let dt = self.tick.as_secs_f32();
        let mut duration = 0.0f32;
        let mut center = self.position_mm().await;
        let mut lives = self.g.stillness_lives.max(1);
        let debounce = Duration::from_millis(self.g.stillness_debounce_ms.max(1));
        let mut cooldown_until: Option<Instant> = None;

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None => return Exit::Closed,
                    Some(GameControl::Stop) => return Exit::Stop,
                    Some(GameControl::Start { kind }) => return Exit::Restart(kind),
                    // No hold to track and the ready gesture already happened.
                    Some(GameControl::Button { .. }) | Some(GameControl::HardwareTap) => {}
                },
                _ = tick.tick() => {
                    let pos = self.position_mm().await;
                    let dev = (pos - center).abs();
                    let frac = dev / self.g.stillness_tolerance_mm.max(0.1);
                    let cooled_down = cooldown_until.map_or(true, |t| Instant::now() >= t);
                    if dev > self.g.stillness_tolerance_mm && cooled_down {
                        // Moved too far — buzz a warning, spend a life, re-anchor
                        // so the next attempt starts from where the hand is now.
                        lives -= 1;
                        cooldown_until = Some(Instant::now() + debounce);
                        center = pos;
                        if lives == 0 {
                            self.publish(GamePhase::Slip, 1.0, 0, duration, true).await;
                            return Exit::Stop;
                        }
                        self.vibrate().await;
                        self.publish(GamePhase::Slip, frac, lives, duration, true).await;
                        continue;
                    }
                    if dev <= self.g.stillness_tolerance_mm {
                        duration += dt;
                    }
                    self.publish(GamePhase::Hold, frac, lives, duration, true).await;
                }
            }
        }
    }

    /// Brief micro-vibration feedback: a small pulse away from center and back,
    /// servo off again once it settles.
    async fn vibrate(&self) {
        let pos = self.position_mm().await;
        let dir = if rand::random::<bool>() { 1.0 } else { -1.0 };
        let target = (pos + dir * self.g.stillness_vibration_mm).clamp(0.0, self.stroke_mm);
        self.servo(true).await;
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: target,
                vel_mm_s: self.g.max_velocity_mm_s * 0.5,
                accel_g: self.g.accel_g,
                profile: crate::config::MotionProfile::Trapezoid,
                soften: false,
            })
            .await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
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
            ready_taps = 3
            ready_window_ms = 1000
            ready_delay_ms = 10
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
        let (piupiu_tx, _piupiu_rx) = mpsc::channel(16);
        let h = tokio::spawn(GameTask::new(state.clone(), cmd_tx, piupiu_tx, &cfg()).run(ctrl_rx));
        (state, ctrl_tx, cmd_rx, h)
    }

    /// Advance one heartbeat: optionally resend the held button (clients resend
    /// every ~50ms; the deadman is 150ms), step the clock, let the task run, and
    /// drain the command channel so it never blocks.
    async fn beat(
        ctrl: &mpsc::Sender<GameControl>,
        cmd_rx: &mut mpsc::Receiver<ActuatorCommand>,
        hold: bool,
    ) {
        if hold {
            ctrl.send(GameControl::Button { down: true }).await.unwrap();
        }
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        while cmd_rx.try_recv().is_ok() {}
    }

    /// Send the hardware triple-tap ready gesture and let the post-tap start
    /// delay elapse, carrying the task from Armed into actual play.
    async fn ready(ctrl: &mpsc::Sender<GameControl>, cmd_rx: &mut mpsc::Receiver<ActuatorCommand>) {
        for _ in 0..3 {
            ctrl.send(GameControl::HardwareTap).await.unwrap();
            tokio::task::yield_now().await;
        }
        beat(ctrl, cmd_rx, false).await;
    }

    #[tokio::test(start_paused = true)]
    async fn arming_requires_three_taps_before_play_starts() {
        let (state, ctrl, mut cmd_rx, h) = spawn();
        ctrl.send(GameControl::Start {
            kind: GameKind::EdgeRecover,
        })
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(state.read().await.game.phase, GamePhase::Armed);

        // Two taps: still armed, not yet playing.
        ctrl.send(GameControl::HardwareTap).await.unwrap();
        tokio::task::yield_now().await;
        ctrl.send(GameControl::HardwareTap).await.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(state.read().await.game.phase, GamePhase::Armed);
        assert_eq!(state.read().await.game.level, 2, "tap count so far");

        // Web/app Button heartbeats don't count as taps — still armed.
        ctrl.send(GameControl::Button { down: true }).await.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(state.read().await.game.phase, GamePhase::Armed);
        assert_eq!(state.read().await.game.level, 2);

        // Third tap completes the gesture; play starts after the delay.
        ready(&ctrl, &mut cmd_rx).await;
        assert_ne!(state.read().await.game.phase, GamePhase::Armed);
        assert!(state.read().await.game.active);

        ctrl.send(GameControl::Stop).await.unwrap();
        drop(ctrl);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn edge_recover_climbs_while_held() {
        let (state, ctrl, mut cmd_rx, h) = spawn();
        ctrl.send(GameControl::Start {
            kind: GameKind::EdgeRecover,
        })
        .await
        .unwrap();
        ready(&ctrl, &mut cmd_rx).await;

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
        ctrl.send(GameControl::Start {
            kind: GameKind::Gauntlet,
        })
        .await
        .unwrap();
        ready(&ctrl, &mut cmd_rx).await;

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

    /// Fast approach velocity (keeps the start-position move short in test
    /// time) plus tighter lives/tolerance for deterministic drift checks.
    fn spawn_stillness() -> (
        Arc<RwLock<AppState>>,
        mpsc::Sender<GameControl>,
        mpsc::Receiver<ActuatorCommand>,
        tokio::task::JoinHandle<()>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let (piupiu_tx, _piupiu_rx) = mpsc::channel(16);
        let mut c = cfg();
        c.actuator.games.min_velocity_mm_s = 3000.0;
        c.actuator.games.stillness_lives = 3;
        c.actuator.games.stillness_tolerance_mm = 5.0;
        let h = tokio::spawn(GameTask::new(state.clone(), cmd_tx, piupiu_tx, &c).run(ctrl_rx));
        (state, ctrl_tx, cmd_rx, h)
    }

    #[tokio::test(start_paused = true)]
    async fn stillness_moves_to_start_then_tracks_drift_without_holding() {
        let (state, ctrl, mut cmd_rx, h) = spawn_stillness();
        ctrl.send(GameControl::Start {
            kind: GameKind::Stillness,
        })
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(state.read().await.game.phase, GamePhase::Armed);

        for _ in 0..3 {
            ctrl.send(GameControl::HardwareTap).await.unwrap();
            tokio::task::yield_now().await;
        }

        // The ready-delay settle, the servo-on settle, and the approach move
        // to the start position are three separately-scheduled sleeps in
        // sequence — each only registers its timer once the previous one
        // fires — so step time forward in small increments rather than one
        // big jump (see the win-celebration test above for the same issue).
        for _ in 0..10 {
            tokio::time::advance(Duration::from_millis(20)).await;
            tokio::task::yield_now().await;
        }

        // The round opened with a single motor-driven move to 60% of the
        // 300mm stroke — the player never had to touch a button for this.
        let mut moved_to = None;
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let ActuatorCommand::MoveTo { pos_mm, .. } = cmd {
                moved_to = Some(pos_mm);
            }
        }
        assert_eq!(moved_to, Some(180.0), "60% of the 300mm stroke");

        let g = state.read().await.game.clone();
        assert_eq!(g.phase, GamePhase::Hold);
        assert_eq!(g.level, 3, "full lives, nothing lost while still");
        assert!(g.holding, "stillness always reports holding — no hold gesture exists");

        // Hold still for a few ticks: duration accrues, still no button held.
        for _ in 0..3 {
            beat(&ctrl, &mut cmd_rx, false).await;
        }
        let g = state.read().await.game.clone();
        assert!(g.duration_s > 0.2, "should accrue duration without holding: {}", g.duration_s);
        assert_eq!(g.level, 3);

        // Drift past tolerance without ever touching a button — Stillness
        // has no deadman, so this alone must cost a life. The life-loss tick
        // chains its own vibration sleep before publishing, and the round
        // re-anchors on the drifted spot afterward (so `phase` flips back to
        // `Hold` on the very next regular tick) — `level` is the durable
        // signal to assert on here, not the transient `Slip` phase.
        state.write().await.position_mm = 180.0 + 10.0; // tolerance is 5mm
        for _ in 0..15 {
            tokio::time::advance(Duration::from_millis(20)).await;
            tokio::task::yield_now().await;
        }
        while cmd_rx.try_recv().is_ok() {}
        let g = state.read().await.game.clone();
        assert_eq!(g.level, 2, "one life lost from drift alone, no button involved");

        ctrl.send(GameControl::Stop).await.unwrap();
        drop(ctrl);
        let _ = h.await;
    }

    /// Small climb (2 checkpoints, near-instant) so the win fires in a couple
    /// of ticks; sets up its own channels (rather than `spawn()`) to observe
    /// the PiuPiu control channel directly.
    #[allow(clippy::type_complexity)]
    fn spawn_climb() -> (
        Arc<RwLock<AppState>>,
        mpsc::Sender<GameControl>,
        mpsc::Receiver<ActuatorCommand>,
        mpsc::Receiver<PiuPiuControl>,
        tokio::task::JoinHandle<()>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let (piupiu_tx, piupiu_rx) = mpsc::channel(16);
        let mut c = cfg();
        c.actuator.games.climb_checkpoints = 2;
        c.actuator.games.climb_total_s = 0.2;
        let h = tokio::spawn(GameTask::new(state.clone(), cmd_tx, piupiu_tx, &c).run(ctrl_rx));
        (state, ctrl_tx, cmd_rx, piupiu_rx, h)
    }

    /// With `climb_checkpoints = 2` and `climb_total_s = 0.2` (dt = 0.1s per
    /// tick), intensity crosses the 2nd/last checkpoint on exactly the 2nd
    /// held beat, so 3 beats deterministically reaches the win. The game
    /// phase itself flips straight back to `Idle` in the same tick (the
    /// dispatcher auto-stops on win) — a snapshot read can miss the
    /// momentary `Win`, so these tests assert on the celebration's actual
    /// observable effect (the PiuPiu channel) instead of the transient phase.
    async fn climb_to_win(ctrl: &mpsc::Sender<GameControl>, cmd_rx: &mut mpsc::Receiver<ActuatorCommand>) {
        ready(ctrl, cmd_rx).await;
        for _ in 0..3 {
            beat(ctrl, cmd_rx, true).await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn climb_win_fires_three_piupiu_pulses_when_connected() {
        let (state, ctrl, mut cmd_rx, mut piupiu_rx, h) = spawn_climb();
        state.write().await.piupiu.connected = true;
        ctrl.send(GameControl::Start {
            kind: GameKind::DeadmansClimb,
        })
        .await
        .unwrap();
        climb_to_win(&ctrl, &mut cmd_rx).await;

        // Let the background celebration task run its three pulses to
        // completion. Each `sleep` only registers its *next* timer once
        // polled after the previous one fires, so drive time forward in
        // small steps rather than one big jump.
        for _ in 0..(2 * TADA_PULSES + 2) {
            tokio::time::advance(TADA_PULSE.max(TADA_GAP)).await;
            tokio::task::yield_now().await;
        }

        let mut pulses = Vec::new();
        while let Ok(c) = piupiu_rx.try_recv() {
            pulses.push(c);
        }
        let on = pulses
            .iter()
            .filter(|c| matches!(c, PiuPiuControl::Squirt { active: true }))
            .count();
        let off = pulses
            .iter()
            .filter(|c| matches!(c, PiuPiuControl::Squirt { active: false }))
            .count();
        assert_eq!((on, off), (3, 3), "expected 3 on/off squirt pairs: {pulses:?}");

        drop(ctrl);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn climb_win_sends_nothing_when_piupiu_disconnected() {
        let (_state, ctrl, mut cmd_rx, mut piupiu_rx, h) = spawn_climb();
        // No `state.piupiu.connected = true` — stays disconnected.
        ctrl.send(GameControl::Start {
            kind: GameKind::DeadmansClimb,
        })
        .await
        .unwrap();
        climb_to_win(&ctrl, &mut cmd_rx).await;

        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;

        assert!(
            piupiu_rx.try_recv().is_err(),
            "no PiuPiu connected: the win celebration must stay silent"
        );

        drop(ctrl);
        let _ = h.await;
    }
}
