//! Learn — teach-and-repeat (SSCP extension).
//!
//! A one-button record/playback program:
//!   1. **Armed** — servo off, the rod moves freely by hand. Press to record.
//!   2. **Recording** — the rod stays free; the user moves the tip while the
//!      status poll's `position_mm` is sampled. Press to stop.
//!   3. **Ready** — the raw samples are simplified to *Stützpunkte* (support
//!      points) with a Ramer–Douglas–Peucker pass: redundant near-collinear
//!      samples are dropped, turning points kept. Press to play.
//!   4. **Playing** — the servo drives the rod through the support points on a
//!      loop, preserving the original timing (so it imitates the *speed* of the
//!      hand motion, not just the path). Press to re-arm and record anew.
//!
//! Recording relies on the controller reporting `PNOW` with the servo off —
//! the same fact the peck-probe depends on.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::info;

use super::LearnControl;
use crate::config::{Config, Learn, MotionProfile};
use crate::state::{ActuatorCommand, AppMode, AppState, LearnPhase};

const SERVO_SETTLE: Duration = Duration::from_millis(50);

/// Ramer–Douglas–Peucker-style simplification of a position-vs-time curve.
/// Distance is measured vertically (in mm) against the linear interpolation, so
/// `eps` is a millimetre tolerance. Always keeps the first and last sample; caps
/// the result at `max` points by uniform subsampling if necessary.
fn simplify(pts: &[(u32, f32)], eps: f32, max: usize) -> Vec<(u32, f32)> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    rdp(pts, 0, pts.len() - 1, eps.max(0.01), &mut keep);
    let mut out: Vec<(u32, f32)> = pts
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect();
    if max >= 2 && out.len() > max {
        let step = out.len() as f32 / max as f32;
        let mut sub = Vec::with_capacity(max + 1);
        let mut i = 0.0f32;
        while (i as usize) < out.len() {
            sub.push(out[i as usize]);
            i += step;
        }
        let last = *out.last().unwrap();
        if sub.last() != Some(&last) {
            sub.push(last);
        }
        out = sub;
    }
    out
}

fn rdp(pts: &[(u32, f32)], lo: usize, hi: usize, eps: f32, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let (t0, y0) = pts[lo];
    let (t1, y1) = pts[hi];
    let dt = (t1 - t0) as f32;
    let mut best = 0.0f32;
    let mut bi = 0usize;
    for (i, &(t, y)) in pts.iter().enumerate().take(hi).skip(lo + 1) {
        let interp = if dt.abs() < f32::EPSILON {
            y0
        } else {
            y0 + (y1 - y0) * ((t - t0) as f32 / dt)
        };
        let d = (y - interp).abs();
        if d > best {
            best = d;
            bi = i;
        }
    }
    if best > eps {
        keep[bi] = true;
        rdp(pts, lo, bi, eps, keep);
        rdp(pts, bi, hi, eps, keep);
    }
}

pub struct LearnTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    l: Learn,
    profile: MotionProfile,
    max_velocity_mm_s: f32,
}

impl LearnTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        cfg: &Config,
    ) -> Self {
        LearnTask {
            state,
            cmd_tx,
            l: cfg.actuator.learn.clone(),
            profile: cfg.motion_profile().unwrap_or(MotionProfile::Trapezoid),
            max_velocity_mm_s: cfg
                .actuator
                .learn
                .max_velocity_mm_s
                .min(cfg.actuator.limits.max_velocity_mm_s),
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<LearnControl>) {
        info!("learn task running");
        loop {
            match ctrl_rx.recv().await {
                None => break,
                Some(LearnControl::Start) => self.session(&mut ctrl_rx).await,
                Some(_) => {}
            }
        }
        info!("learn task stopped");
    }

    async fn session(&self, rx: &mut mpsc::Receiver<LearnControl>) {
        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
        let mut phase = LearnPhase::Armed;
        let mut buffer: Vec<(u32, f32)> = Vec::new();
        let mut waypoints: Vec<(u32, f32)> = Vec::new();
        let mut rec_start: Option<Instant> = None;
        let mut play_idx = 0usize;
        let mut next_seg: Option<Instant> = None;

        let max_samples =
            ((self.l.max_record_s * 1000.0) / self.l.sample_ms.max(1) as f32) as usize;
        let mut sample = interval(Duration::from_millis(self.l.sample_ms.max(10)));
        sample.set_missed_tick_behavior(MissedTickBehavior::Skip);

        {
            let mut st = self.state.write().await;
            st.learn = crate::state::LearnRuntime::default();
            st.learn.active = true;
            st.set_mode(AppMode::Learn);
        }
        self.publish(phase, 0, 0).await;
        info!("learn: armed");

        loop {
            tokio::select! {
                ctrl = rx.recv() => match ctrl {
                    None | Some(LearnControl::Stop) => { self.halt().await; return; }
                    Some(LearnControl::Start) => {
                        // Re-arm from scratch.
                        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                        phase = LearnPhase::Armed;
                        buffer.clear();
                        waypoints.clear();
                        next_seg = None;
                        self.publish(phase, 0, 0).await;
                    }
                    Some(LearnControl::Button) => {
                        phase = match phase {
                            LearnPhase::Armed => {
                                buffer.clear();
                                rec_start = Some(Instant::now());
                                info!("learn: recording");
                                LearnPhase::Recording
                            }
                            LearnPhase::Recording => {
                                waypoints = simplify(&buffer, self.l.simplify_epsilon_mm, self.l.max_waypoints);
                                info!(samples = buffer.len(), waypoints = waypoints.len(), "learn: ready");
                                LearnPhase::Ready
                            }
                            LearnPhase::Ready => {
                                if waypoints.len() >= 2 {
                                    let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(true)).await;
                                    tokio::time::sleep(SERVO_SETTLE).await;
                                    self.move_to(waypoints[0].1, self.max_velocity_mm_s * 0.5).await;
                                    play_idx = 0;
                                    next_seg = Some(Instant::now() + self.seg_time(&waypoints, 0));
                                    info!("learn: playing");
                                    LearnPhase::Playing
                                } else {
                                    // Nothing usable recorded — go back to armed.
                                    LearnPhase::Armed
                                }
                            }
                            LearnPhase::Playing => {
                                let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
                                let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
                                next_seg = None;
                                buffer.clear();
                                waypoints.clear();
                                info!("learn: re-armed");
                                LearnPhase::Armed
                            }
                        };
                        let (pts, wps) = (buffer.len() as u32, waypoints.len() as u32);
                        self.publish(phase, pts, wps).await;
                    }
                },

                _ = sample.tick(), if matches!(phase, LearnPhase::Recording) => {
                    if buffer.len() < max_samples {
                        let t = rec_start.map(|s| s.elapsed().as_millis() as u32).unwrap_or(0);
                        let pos = self.state.read().await.position_mm;
                        buffer.push((t, pos));
                        self.publish(phase, buffer.len() as u32, 0).await;
                    }
                }

                _ = sleep_until_opt(next_seg), if next_seg.is_some() => {
                    let n = waypoints.len();
                    if n < 2 { next_seg = None; continue; }
                    let to = (play_idx + 1) % n;
                    let dt = self.seg_time(&waypoints, play_idx);
                    let dist = (waypoints[to].1 - waypoints[play_idx].1).abs();
                    let vel = (dist / dt.as_secs_f32().max(0.001))
                        .clamp(f32::MIN_POSITIVE, self.max_velocity_mm_s);
                    self.move_to(waypoints[to].1, vel).await;
                    play_idx = to;
                    next_seg = Some(Instant::now() + self.seg_time(&waypoints, play_idx));
                }
            }
        }
    }

    /// Time of the segment starting at waypoint `from` (to the next, wrapping).
    /// The wrap segment (last → first) uses the configured loop gap since the
    /// recording doesn't define it.
    fn seg_time(&self, wps: &[(u32, f32)], from: usize) -> Duration {
        let n = wps.len();
        let to = (from + 1) % n;
        if to == 0 {
            Duration::from_millis(self.l.loop_gap_ms.max(1))
        } else {
            Duration::from_millis((wps[to].0.saturating_sub(wps[from].0)).max(1) as u64)
        }
    }

    async fn move_to(&self, pos_mm: f32, vel_mm_s: f32) {
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm,
                vel_mm_s,
                accel_g: self.l.accel_g,
                profile: self.profile,
                soften: false,
            })
            .await;
    }

    async fn publish(&self, phase: LearnPhase, points: u32, waypoints: u32) {
        let mut st = self.state.write().await;
        st.learn.phase = phase;
        st.learn.points = points;
        st.learn.waypoints = waypoints;
    }

    async fn halt(&self) {
        let _ = self.cmd_tx.send(ActuatorCommand::Stop).await;
        let _ = self.cmd_tx.send(ActuatorCommand::ServoOn(false)).await;
        let mut st = self.state.write().await;
        st.learn.active = false;
        st.learn.phase = LearnPhase::Armed;
        st.set_mode(AppMode::Idle);
        info!("learn halted");
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
            [actuator.learn]
            sample_ms = 50
        "#,
        )
        .unwrap()
    }

    #[test]
    fn simplify_keeps_turning_points_drops_collinear() {
        // A straight ramp then a peak: collinear run should collapse to ends.
        let pts: Vec<(u32, f32)> = (0..=10).map(|i| (i * 100, i as f32)).collect();
        let out = simplify(&pts, 0.5, 100);
        assert_eq!(out.len(), 2, "a straight line → 2 support points: {out:?}");

        // A triangle: up to 5 then down to 0 — should keep the apex.
        let mut tri: Vec<(u32, f32)> = (0..=5).map(|i| (i * 100, i as f32)).collect();
        tri.extend((1..=5).map(|i| ((5 + i) * 100, 5.0 - i as f32)));
        let out = simplify(&tri, 0.5, 100);
        assert_eq!(out.len(), 3, "triangle → start, apex, end: {out:?}");
        assert_eq!(out[1].1, 5.0);
    }

    #[tokio::test(start_paused = true)]
    async fn phase_cycle_advances_on_press() {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        state.write().await.position_mm = 100.0;
        let (cmd_tx, mut cmd_rx) = mpsc::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let h = tokio::spawn(LearnTask::new(state.clone(), cmd_tx, &cfg()).run(ctrl_rx));

        let settle = |ms: u64| async move {
            tokio::time::advance(Duration::from_millis(ms)).await;
            tokio::task::yield_now().await;
        };

        ctrl_tx.send(LearnControl::Start).await.unwrap();
        settle(10).await;
        assert_eq!(state.read().await.learn.phase, LearnPhase::Armed);

        // Press → recording; sample a little (position constant → 2 waypoints).
        ctrl_tx.send(LearnControl::Button).await.unwrap();
        for _ in 0..6 { settle(50).await; while cmd_rx.try_recv().is_ok() {} }
        assert_eq!(state.read().await.learn.phase, LearnPhase::Recording);
        assert!(state.read().await.learn.points > 0);

        // Press → ready (simplified).
        ctrl_tx.send(LearnControl::Button).await.unwrap();
        settle(10).await;
        assert_eq!(state.read().await.learn.phase, LearnPhase::Ready);
        assert!(state.read().await.learn.waypoints >= 2);

        // Press → playing.
        ctrl_tx.send(LearnControl::Button).await.unwrap();
        for _ in 0..4 { settle(50).await; while cmd_rx.try_recv().is_ok() {} }
        assert_eq!(state.read().await.learn.phase, LearnPhase::Playing);

        // Press → back to armed.
        ctrl_tx.send(LearnControl::Button).await.unwrap();
        settle(10).await;
        assert_eq!(state.read().await.learn.phase, LearnPhase::Armed);

        ctrl_tx.send(LearnControl::Stop).await.unwrap();
        drop(ctrl_tx);
        let _ = h.await;
    }
}
