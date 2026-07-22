//! HSP — streaming-script playback (SPEC §7.3). This is the on-device form of
//! "web HSSP": the cloud converts an HSSP script into an HSP point stream
//! (`Setup` → `Add(points)` → `Play`) and pushes it here; direct BLE clients can
//! drive the same messages.
//!
//! Playback walks the point buffer (`AppState.hsp_buffer`) in time order,
//! emitting one `MoveTo` per segment so the actuator arrives at `points[i+1]`
//! exactly at its timestamp. Real-time is mapped to playback-time through an
//! anchor `(real, play_ms)` pair scaled by `playback_rate`, so rate changes and
//! drift-resync (`CurrentTimeSet`) are cheap.
//!
//! Buffer underrun is surfaced as `NotificationHspStarving`; reaching the
//! declared tail point with no loop ends playback (`NotificationHspStateChanged`
//! STOPPED). Threshold crossings raise `NotificationHspThresholdReached` so the
//! cloud can refill long streams.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::Instant;
use tracing::{debug, info};

use super::HspControl;
use crate::config::{Config, MotionProfile};
use crate::rpc::{
    self, HspPlayState, HspState, Notification, NotificationHspStarving,
    NotificationHspStateChanged, NotificationHspThresholdReached, RpcMessage,
};
use crate::state::{ActuatorCommand, AppState};
use crate::telemetry::metrics;
use crate::translator::{Translator, Zone};

pub struct HspTask {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    notif_tx: broadcast::Sender<RpcMessage>,
    stroke_mm: f32,
    max_velocity_mm_s: f32,
    accel_g: f32,
    profile: MotionProfile,
}

/// Live playback cursor.
struct Playback {
    playing: bool,
    /// Index of the point we are currently at / heading to (already issued).
    idx: usize,
    /// Anchor pairing a real instant with a playback timestamp (ms).
    anchor_real: Instant,
    anchor_play_ms: f64,
    rate: f64,
    looped: bool,
    pause_on_starving: bool,
    /// Whether we already raised the threshold notification this stream.
    notified_threshold: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Playback {
            playing: false,
            idx: 0,
            anchor_real: Instant::now(),
            anchor_play_ms: 0.0,
            rate: 1.0,
            looped: false,
            pause_on_starving: false,
            notified_threshold: false,
        }
    }
}

impl Playback {
    /// Real instant at which playback-time `play_ms` occurs.
    fn play_to_real(&self, play_ms: f64) -> Instant {
        let dt_ms = (play_ms - self.anchor_play_ms) / self.rate;
        if dt_ms >= 0.0 {
            self.anchor_real + Duration::from_secs_f64(dt_ms / 1000.0)
        } else {
            self.anchor_real
                .checked_sub(Duration::from_secs_f64(-dt_ms / 1000.0))
                .unwrap_or(self.anchor_real)
        }
    }
}

impl HspTask {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        notif_tx: broadcast::Sender<RpcMessage>,
        cfg: &Config,
    ) -> Self {
        HspTask {
            state,
            cmd_tx,
            notif_tx,
            stroke_mm: cfg.stroke_mm(),
            max_velocity_mm_s: cfg.actuator.limits.max_velocity_mm_s,
            accel_g: cfg.actuator.limits.default_accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
        }
    }

    pub async fn run(self, mut ctrl_rx: mpsc::Receiver<HspControl>) {
        info!("hsp task running");
        let mut pb = Playback::default();
        // Next scheduled segment boundary (real instant) when playing.
        let mut next: Option<Instant> = None;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => break,
                    Some(c) => next = self.handle_ctrl(c, &mut pb).await,
                },
                _ = sleep_until_opt(next), if next.is_some() => {
                    next = self.advance(&mut pb).await;
                }
            }
        }
        info!("hsp task stopped");
    }

    async fn handle_ctrl(&self, c: HspControl, pb: &mut Playback) -> Option<Instant> {
        match c {
            HspControl::Setup | HspControl::Stop => {
                pb.playing = false;
                None
            }
            HspControl::Play {
                start_time,
                playback_rate,
                looped,
                pause_on_starving,
                ..
            } => {
                pb.rate = if playback_rate > 0.0 {
                    playback_rate as f64
                } else {
                    1.0
                };
                pb.looped = looped;
                pb.pause_on_starving = pause_on_starving;
                pb.anchor_play_ms = start_time as f64;
                pb.anchor_real = Instant::now();
                pb.notified_threshold = false;
                self.begin_playback(pb, start_time).await
            }
            HspControl::Pause => {
                pb.playing = false;
                None
            }
            HspControl::Resume { pick_up } => {
                if !pick_up {
                    // Resume where we paused: re-anchor "now" to the current point.
                    if let Some(t) = self.point_time(pb.idx).await {
                        pb.anchor_play_ms = t as f64;
                        pb.anchor_real = Instant::now();
                    }
                }
                pb.playing = true;
                self.schedule_from(pb).await
            }
            HspControl::SetPlaybackRate(r) => {
                self.reanchor_now(pb).await;
                pb.rate = if r > 0.0 { r as f64 } else { 1.0 };
                self.schedule_from(pb).await
            }
            HspControl::SetLoop(l) => {
                pb.looped = l;
                next_if_playing(pb)
            }
            HspControl::SetCurrentTime { current_time, .. } => {
                // Drift resync: jump playback time to current_time.
                pb.anchor_play_ms = current_time as f64;
                pb.anchor_real = Instant::now();
                if let Some(i) = self.index_at_or_after(current_time).await {
                    pb.idx = i;
                }
                self.schedule_from(pb).await
            }
            HspControl::Added => {
                // If we were starving, more points may let us continue.
                if pb.playing {
                    self.schedule_from(pb).await
                } else {
                    None
                }
            }
        }
    }

    /// Set up playback from `start_time`: move to the first relevant point and
    /// schedule the first segment boundary.
    async fn begin_playback(&self, pb: &mut Playback, start_time: i32) -> Option<Instant> {
        let len = self.buffer_len().await;
        if len == 0 {
            self.emit_starving().await;
            pb.playing = false;
            return None;
        }
        pb.idx = self.index_at_or_after(start_time).await.unwrap_or(0);
        pb.playing = true;
        // Head to the starting point immediately.
        self.move_to_point(pb.idx).await;
        self.schedule_from(pb).await
    }

    /// Compute the next wake: the real instant of `points[idx].t`.
    async fn schedule_from(&self, pb: &Playback) -> Option<Instant> {
        if !pb.playing {
            return None;
        }
        let t = self.point_time(pb.idx).await?;
        Some(pb.play_to_real(t as f64))
    }

    /// Fired at a segment boundary: begin moving to the next point.
    async fn advance(&self, pb: &mut Playback) -> Option<Instant> {
        let len = self.buffer_len().await;
        let next_idx = pb.idx + 1;

        if next_idx >= len {
            // Out of buffered points.
            if pb.looped && len > 0 {
                metrics::hsp_loop();
                let note = self.note_loop().await;
                self.emit(note);
                pb.idx = 0;
                if let Some(t0) = self.point_time(0).await {
                    pb.anchor_play_ms = t0 as f64;
                    pb.anchor_real = Instant::now();
                }
                self.move_to_point(0).await;
                return self.schedule_from(pb).await;
            }
            // Either script complete (reached declared tail) or buffer underrun.
            if self.reached_tail(pb.idx).await {
                self.complete(pb).await;
            } else {
                self.emit_starving().await;
                self.set_play_state(HspPlayState::HspStateStarving).await;
                if pb.pause_on_starving {
                    pb.playing = false;
                }
                // Stay anchored; `Added` will reschedule.
            }
            return None;
        }

        // Move toward points[next_idx], arriving at its timestamp.
        self.move_segment(pb.idx, next_idx, pb.rate).await;
        pb.idx = next_idx;
        self.update_current(pb).await;
        self.maybe_threshold(pb).await;
        self.schedule_from(pb).await
    }

    // ───────────────────────── motion ─────────────────────────

    async fn translator(&self) -> Translator {
        let st = self.state.read().await;
        let mut t = Translator::new(self.stroke_mm, self.max_velocity_mm_s);
        t.zone = Zone::new(st.slide_min, st.slide_max);
        t
    }

    /// Issue a move toward `points[i]` (used for the initial seek and loop wrap;
    /// uses a moderate default speed).
    async fn move_to_point(&self, i: usize) {
        let t = self.translator().await;
        let (x, _) = match self.point(i).await {
            Some(p) => p,
            None => return,
        };
        let pos_mm = t.hsp_x_to_mm(x);
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm,
                vel_mm_s: self.max_velocity_mm_s * 0.5,
                accel_g: self.accel_g,
                profile: self.profile,
                // HSP points define the trajectory; never re-shape them.
                soften: false,
            })
            .await;
    }

    /// Move from `from` to `to`, sized so arrival coincides with `to`'s
    /// timestamp at the current playback rate.
    async fn move_segment(&self, from: usize, to: usize, rate: f64) {
        let t = self.translator().await;
        let (Some((x0, t0)), Some((x1, t1))) = (self.point(from).await, self.point(to).await)
        else {
            return;
        };
        let from_mm = t.hsp_x_to_mm(x0);
        let to_mm = t.hsp_x_to_mm(x1);
        let dist = (to_mm - from_mm).abs();
        let dt_ms = (t1.saturating_sub(t0)).max(1) as f64;
        // Real time for the segment is dt_ms / rate, so velocity scales by rate.
        let vel = (dist as f64 / dt_ms * 1000.0 * rate) as f32;
        debug!(from, to, dist, vel, "hsp segment");
        let _ = self
            .cmd_tx
            .send(ActuatorCommand::MoveTo {
                pos_mm: to_mm,
                vel_mm_s: vel.max(f32::MIN_POSITIVE),
                accel_g: self.accel_g,
                profile: self.profile,
                soften: false,
            })
            .await;
        let stream_id = self.state.read().await.hsp.stream_id;
        metrics::hsp_points_processed(1, stream_id);
    }

    // ───────────────────────── buffer access ─────────────────────────

    async fn buffer_len(&self) -> usize {
        self.state.read().await.hsp_buffer.len()
    }

    async fn point(&self, i: usize) -> Option<(u8, u32)> {
        let st = self.state.read().await;
        st.hsp_buffer.get(i).map(|p| (p.x as u8, p.t))
    }

    async fn point_time(&self, i: usize) -> Option<u32> {
        self.point(i).await.map(|(_, t)| t)
    }

    /// First buffer index whose timestamp is >= `t_ms` (or 0 for a non-empty
    /// buffer when nothing is later).
    async fn index_at_or_after(&self, t_ms: i32) -> Option<usize> {
        let st = self.state.read().await;
        st.hsp_buffer
            .iter()
            .position(|p| p.t as i32 >= t_ms)
            .or(if st.hsp_buffer.is_empty() {
                None
            } else {
                Some(0)
            })
    }

    /// True if `idx` is at or past the declared tail point of the stream (so no
    /// more points are coming and playback is genuinely complete).
    async fn reached_tail(&self, idx: usize) -> bool {
        let st = self.state.read().await;
        let tail = st.hsp.tail_point_stream_index;
        // tail < 0 means "unknown / open stream" -> treat exhaustion as starving.
        tail >= 0 && idx as i32 >= tail
    }

    // ───────────────────────── state / notifications ─────────────────────────

    async fn update_current(&self, pb: &Playback) {
        let mut st = self.state.write().await;
        st.hsp.current_point = pb.idx as i32;
        st.hsp.current_time = st.hsp_buffer.get(pb.idx).map(|p| p.t as i32).unwrap_or(0);
    }

    async fn set_play_state(&self, ps: HspPlayState) {
        self.state.write().await.hsp.play_state = ps;
    }

    async fn reanchor_now(&self, pb: &mut Playback) {
        // Preserve current playback time across a rate change.
        if let Some(t) = self.point_time(pb.idx).await {
            pb.anchor_play_ms = t as f64;
            pb.anchor_real = Instant::now();
        }
    }

    async fn complete(&self, pb: &mut Playback) {
        pb.playing = false;
        self.set_play_state(HspPlayState::HspStateStopped).await;
        let state = self.snapshot_state().await;
        self.emit(Notification {
            id: 0,
            notification: Some(
                rpc::notification::Notification::NotificationHspStateChanged(
                    NotificationHspStateChanged { state: Some(state) },
                ),
            ),
        });
        info!("hsp playback complete");
    }

    async fn maybe_threshold(&self, pb: &mut Playback) {
        let (threshold, idx) = {
            let st = self.state.read().await;
            (st.hsp.tail_point_threshold, pb.idx)
        };
        if threshold != 0 && !pb.notified_threshold && idx as u32 >= threshold {
            pb.notified_threshold = true;
            let state = self.snapshot_state().await;
            self.emit(Notification {
                id: 0,
                notification: Some(
                    rpc::notification::Notification::NotificationHspThresholdReached(
                        NotificationHspThresholdReached { state: Some(state) },
                    ),
                ),
            });
        }
    }

    async fn emit_starving(&self) {
        metrics::hsp_starving();
        let state = self.snapshot_state().await;
        self.emit(Notification {
            id: 0,
            notification: Some(rpc::notification::Notification::NotificationHspStarving(
                NotificationHspStarving { state: Some(state) },
            )),
        });
    }

    async fn note_loop(&self) -> Notification {
        let state = self.snapshot_state().await;
        Notification {
            id: 0,
            notification: Some(
                rpc::notification::Notification::NotificationHspStateChanged(
                    NotificationHspStateChanged { state: Some(state) },
                ),
            ),
        }
    }

    /// Build an `HspState` snapshot from current state.
    async fn snapshot_state(&self) -> HspState {
        let st = self.state.read().await;
        HspState {
            play_state: st.hsp.play_state as i32,
            points: st.hsp_buffer.len() as u32,
            max_points: st.hsp.max_points,
            current_point: st.hsp.current_point,
            current_time: st.hsp.current_time,
            r#loop: st.hsp.looped,
            playback_rate: st.hsp.playback_rate,
            first_point_time: st.hsp.first_point_time,
            last_point_time: st.hsp.last_point_time,
            stream_id: st.hsp.stream_id,
            tail_point_stream_index: st.hsp.tail_point_stream_index,
            tail_point_stream_index_threshold: st.hsp.tail_point_threshold,
            pause_on_starving: st.hsp.pause_on_starving,
        }
    }

    fn emit(&self, note: Notification) {
        let _ = self.notif_tx.send(RpcMessage::notification(note));
    }
}

fn next_if_playing(pb: &Playback) -> Option<Instant> {
    if pb.playing {
        Some(Instant::now())
    } else {
        None
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
    use crate::rpc::Point;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
        "#,
        )
        .unwrap()
    }

    async fn setup(
        buffer: Vec<Point>,
        tail: i32,
    ) -> (
        Arc<RwLock<AppState>>,
        mpsc::Receiver<ActuatorCommand>,
        mpsc::Sender<HspControl>,
        tokio::task::JoinHandle<()>,
    ) {
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        {
            let mut st = state.write().await;
            st.hsp_buffer = buffer;
            st.hsp.tail_point_stream_index = tail;
        }
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (notif_tx, _n) = broadcast::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let task = HspTask::new(state.clone(), cmd_tx, notif_tx, &cfg());
        let h = tokio::spawn(task.run(ctrl_rx));
        (state, cmd_rx, ctrl_tx, h)
    }

    #[tokio::test(start_paused = true)]
    async fn plays_points_in_time_order() {
        // 0ms -> x0; 1000ms -> x255 (full stroke).
        let (_state, mut cmd_rx, ctrl_tx, h) =
            setup(vec![Point { t: 0, x: 0 }, Point { t: 1000, x: 255 }], 1).await;

        ctrl_tx
            .send(HspControl::Play {
                start_time: 0,
                server_time: 0,
                playback_rate: 1.0,
                looped: false,
                pause_on_starving: false,
            })
            .await
            .unwrap();

        // Initial seek to point 0 (~0 mm).
        match cmd_rx.recv().await.unwrap() {
            ActuatorCommand::MoveTo { pos_mm, .. } => assert!(pos_mm.abs() < 1.0),
            o => panic!("{o:?}"),
        }

        // At the t=0 boundary, segment 0->1 fires: move to ~300 mm at 300 mm/s.
        tokio::time::advance(Duration::from_millis(10)).await;
        match cmd_rx.recv().await.unwrap() {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert!((pos_mm - 300.0).abs() < 1.0);
                assert!((vel_mm_s - 300.0).abs() < 1.0);
            }
            o => panic!("{o:?}"),
        }

        drop(ctrl_tx);
        let _ = h.await;
    }

    #[tokio::test(start_paused = true)]
    async fn loops_back_to_start() {
        let (_state, mut cmd_rx, ctrl_tx, h) =
            setup(vec![Point { t: 0, x: 0 }, Point { t: 500, x: 128 }], 1).await;
        ctrl_tx
            .send(HspControl::Play {
                start_time: 0,
                server_time: 0,
                playback_rate: 1.0,
                looped: true,
                pause_on_starving: false,
            })
            .await
            .unwrap();

        let _ = cmd_rx.recv().await.unwrap(); // seek to point 0
        tokio::time::advance(Duration::from_millis(10)).await;
        let _ = cmd_rx.recv().await.unwrap(); // segment 0->1
                                              // reaching the end with loop=true wraps to point 0 again
        tokio::time::advance(Duration::from_millis(600)).await;
        match cmd_rx.recv().await.unwrap() {
            ActuatorCommand::MoveTo { pos_mm, .. } => assert!(pos_mm.abs() < 1.0),
            o => panic!("{o:?}"),
        }
        drop(ctrl_tx);
        let _ = h.await;
    }
}
