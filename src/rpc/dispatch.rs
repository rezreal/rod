//! RPC dispatch: turn an inbound `Request` into a `Response` (plus side effects
//! — actuator commands, mode-task control, state mutation, notifications).
//! Shared by both transports (SPEC §6). BLE and cloud decode an `RpcMessage`
//! and hand each `Request` to [`Dispatcher::handle_request`].

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, warn};

use crate::config::{Config, MotionProfile};
use crate::modes::handswitch::HandEdge;
use crate::modes::{
    CycleControl, DrillControl, EchoControl, GameControl, HampControl, HspControl, ImpaleControl,
    LearnControl, ModeControls, PlumbControl, PulseControl, RampControl, SurgeControl,
    TempoControl, TideControl, TraceControl,
};
use crate::rpc::{request::Params, response::Result as Res, *};
use crate::state::{ActuatorCommand, AppMode, AppState};
use crate::translator::{Translator, Zone};

/// Handles requests from any transport. Cheap to clone (all fields are shared
/// handles), so each transport task can hold its own copy.
#[derive(Clone)]
pub struct Dispatcher {
    state: Arc<RwLock<AppState>>,
    cmd_tx: mpsc::Sender<ActuatorCommand>,
    notif_tx: broadcast::Sender<RpcMessage>,
    hamp_tx: mpsc::Sender<HampControl>,
    hsp_tx: mpsc::Sender<HspControl>,
    modes: ModeControls,
    stroke_mm: f32,
    max_velocity_mm_s: f32,
    default_accel_g: f32,
    profile: MotionProfile,
}

impl Dispatcher {
    pub fn new(
        state: Arc<RwLock<AppState>>,
        cmd_tx: mpsc::Sender<ActuatorCommand>,
        notif_tx: broadcast::Sender<RpcMessage>,
        hamp_tx: mpsc::Sender<HampControl>,
        hsp_tx: mpsc::Sender<HspControl>,
        modes: ModeControls,
        cfg: &Config,
    ) -> Self {
        Dispatcher {
            state,
            cmd_tx,
            notif_tx,
            hamp_tx,
            hsp_tx,
            modes,
            stroke_mm: cfg.stroke_mm(),
            max_velocity_mm_s: cfg.actuator.limits.max_velocity_mm_s,
            default_accel_g: cfg.actuator.limits.default_accel_g,
            profile: cfg.motion_profile().unwrap_or(MotionProfile::SCurve),
        }
    }

    /// Build a translator that reflects the current stroke zone.
    async fn translator(&self) -> Translator {
        let st = self.state.read().await;
        let mut t = Translator::new(self.stroke_mm, self.max_velocity_mm_s);
        t.zone = Zone::new(st.slide_min, st.slide_max);
        t
    }

    async fn send_cmd(&self, cmd: ActuatorCommand) {
        if let Err(e) = self.cmd_tx.send(cmd).await {
            warn!(error = %e, "actuator command channel closed");
        }
    }

    fn emit(&self, note: Notification) {
        let _ = self.notif_tx.send(RpcMessage::notification(note));
    }

    /// Dispatch a single request, returning the response to send back to the
    /// originating transport.
    pub async fn handle_request(&self, req: Request) -> Response {
        let id = req.id;
        let Some(params) = req.params else {
            return Response::err(
                id,
                HandyErrorCodes::ErrorUnknownRequestType as i32,
                "empty request params",
            );
        };
        debug!(id, kind = params_label(&params), "dispatch request");

        match params {
            // ───────────── housekeeping ─────────────
            Params::RequestConnectionKeyGet(_) => {
                let key = self
                    .state
                    .read()
                    .await
                    .connection_key
                    .clone()
                    .unwrap_or_default();
                ok(
                    id,
                    Res::ResponseConnectionKeyGet(ResponseConnectionKeyGet { key }),
                )
            }
            Params::RequestModeGet(_) => {
                let st = self.state.read().await;
                ok(
                    id,
                    Res::ResponseModeGet(ResponseModeGet {
                        mode: app_mode_to_proto(st.mode) as i32,
                        mode_session_id: st.mode_session_id,
                    }),
                )
            }
            Params::RequestModeSet(m) => {
                let target = proto_mode_to_app(m.mode);
                let sid = self.apply_mode(target).await;
                ok(
                    id,
                    Res::ResponseModeSet(ResponseModeSet {
                        mode: m.mode,
                        mode_session_id: sid,
                    }),
                )
            }
            Params::RequestStopCurrentMode(_) => {
                self.stop_everything().await;
                Response::blank(id)
            }
            Params::RequestConnectionModeGet(_) => ok(
                id,
                Res::ResponseConnectionModeGet(ResponseConnectionModeGet {
                    mode: ConnectionMode::WifiAndBle as i32,
                }),
            ),
            Params::RequestConnectionModeSet(_) => Response::blank(id),
            Params::RequestCapabilitiesGet(_) => ok(
                id,
                Res::ResponseCapabilitiesGet(ResponseCapabilitiesGet {
                    vulva_oriented: false,
                    battery: false,
                    slider: 1,
                    lra: 0,
                    erm: 0,
                    external_memory: false,
                    rgb_led_indicator: false,
                    led_matrix: false,
                    led_matrix_leds_x: 0,
                    led_matrix_leds_y: 0,
                    rgb_ring: false,
                    rgb_ring_leds: 0,
                    battery_capacity: 0,
                    battery_driver: BatteryDriver::NotSet as i32,
                    ble_mtu: 512,
                    ws_buffer_size: 2048,
                }),
            ),
            Params::RequestSessionIdsGet(_) => {
                let st = self.state.read().await;
                ok(
                    id,
                    Res::ResponseSessionIdsGet(ResponseSessionIdsGet {
                        boot_session_id: st.boot_session_id,
                        socket_session_id: st.socket_session_id,
                        mode_session_id: st.mode_session_id,
                    }),
                )
            }
            Params::RequestBatteryGet(_) => ok(
                id,
                // Mains-powered rig: report a healthy "battery".
                Res::ResponseBatteryGet(ResponseBatteryGet {
                    state: Some(BatteryState {
                        level: 100,
                        charger_connected: true,
                        charging_complete: true,
                        ..Default::default()
                    }),
                }),
            ),
            Params::RequestClockOffsetSet(c) => {
                self.state.write().await.clock_offset_ms = c.clock_offset;
                ok(
                    id,
                    Res::ResponseClockOffsetSet(ResponseClockOffsetSet {
                        time: 0,
                        clock_offset: c.clock_offset,
                        rtd: c.rtd,
                    }),
                )
            }
            Params::RequestClockOffsetGet(_) => {
                let off = self.state.read().await.clock_offset_ms;
                ok(
                    id,
                    Res::ResponseClockOffsetGet(ResponseClockOffsetGet {
                        time: 0,
                        clock_offset: off,
                        rtd: 0,
                    }),
                )
            }

            // ───────────── HAMP ─────────────
            Params::RequestHampStart(_) => {
                {
                    let mut st = self.state.write().await;
                    st.hamp.running = true;
                    st.set_mode(AppMode::Hamp);
                }
                let _ = self.hamp_tx.send(HampControl::Start).await;
                self.hamp_resp(id, |s| {
                    Res::ResponseHampStart(ResponseHampStart { state: s })
                })
                .await
            }
            Params::RequestHampStop(_) => {
                self.state.write().await.hamp.running = false;
                let _ = self.hamp_tx.send(HampControl::Stop).await;
                self.hamp_resp(id, |s| Res::ResponseHampStop(ResponseHampStop { state: s }))
                    .await
            }
            Params::RequestHampVelocitySet(v) => {
                {
                    let mut st = self.state.write().await;
                    st.hamp.velocity = v.velocity.clamp(0.0, 1.0);
                }
                let _ = self.hamp_tx.send(HampControl::Update).await;
                self.hamp_resp(id, |s| {
                    Res::ResponseHampVelocitySet(ResponseHampVelocitySet { state: s })
                })
                .await
            }
            Params::RequestHampZoneSet(z) => {
                {
                    let mut st = self.state.write().await;
                    let zone = Zone::new(z.min, z.max);
                    st.hamp.min = zone.min;
                    st.hamp.max = zone.max;
                }
                let _ = self.hamp_tx.send(HampControl::Update).await;
                self.hamp_resp(id, |s| {
                    Res::ResponseHampZoneSet(ResponseHampZoneSet { state: s })
                })
                .await
            }
            Params::RequestHampStateGet(_) => {
                self.hamp_resp(id, |s| {
                    Res::ResponseHampStateGet(ResponseHampStateGet { state: s })
                })
                .await
            }

            // ───────────── HDSP (OK/Error only) ─────────────
            Params::RequestHdspXaVaSet(r) => {
                self.hdsp_move(r.xa, r.va, true, false).await;
                Response::blank(id)
            }
            Params::RequestHdspXpVaSet(r) => {
                let mm = self.translator().await.rel_to_mm(r.xp);
                self.hdsp_move(mm, r.va, false, false).await;
                Response::blank(id)
            }
            Params::RequestHdspXpVpSet(r) => {
                let t = self.translator().await;
                let mm = t.rel_to_mm(r.xp);
                let vel = t.vel_pct_to_mm_s(r.vp);
                self.hdsp_move(mm, vel, false, false).await;
                Response::blank(id)
            }
            Params::RequestHdspXaTSet(r) => {
                self.hdsp_move_duration(r.xa, r.t, true).await;
                Response::blank(id)
            }
            Params::RequestHdspXpTSet(r) => {
                let mm = self.translator().await.rel_to_mm(r.xp);
                self.hdsp_move_duration(mm, r.t, false).await;
                Response::blank(id)
            }
            Params::RequestHdspXaVpSet(r) => {
                let vel = self.translator().await.vel_pct_to_mm_s(r.vp);
                self.hdsp_move(r.xa, vel, true, false).await;
                Response::blank(id)
            }
            Params::RequestHdspStop(_) => {
                self.send_cmd(ActuatorCommand::Stop).await;
                Response::blank(id)
            }

            // ───────────── slider / calibration ─────────────
            Params::RequestSliderStrokeGet(_) => {
                ok(id, Res::ResponseSliderStrokeGet(self.stroke_get().await))
            }
            Params::RequestSliderStrokeSet(s) => {
                let zone = Zone::new(s.min, s.max);
                {
                    let mut st = self.state.write().await;
                    st.slide_min = zone.min;
                    st.slide_max = zone.max;
                }
                let g = self.stroke_get().await;
                self.emit(Notification {
                    id: 0,
                    notification: Some(notification::Notification::NotificationStrokeChanged(
                        NotificationStrokeChanged {
                            min: g.min,
                            max: g.max,
                            min_absolute: g.min_absolute,
                            max_absolute: g.max_absolute,
                        },
                    )),
                });
                ok(
                    id,
                    Res::ResponseSliderStrokeSet(ResponseSliderStrokeSet {
                        min: g.min,
                        max: g.max,
                        min_absolute: g.min_absolute,
                        max_absolute: g.max_absolute,
                    }),
                )
            }
            Params::RequestSliderStateGet(_) => {
                let st = self.state.read().await;
                ok(
                    id,
                    Res::ResponseSliderStateGet(ResponseSliderStateGet {
                        position: if self.stroke_mm > 0.0 {
                            st.position_mm / self.stroke_mm
                        } else {
                            0.0
                        },
                        position_absolute: st.position_mm,
                        motor_temp: st.motor_temp_c,
                        speed_absolute: st.speed_mm_s,
                        dir: st.target_mm >= st.position_mm,
                        motor_position: 0,
                        motor_temp_adc_value: 0,
                    }),
                )
            }
            Params::RequestSliderCalibrate(_) => {
                self.apply_mode(AppMode::Homing).await;
                self.send_cmd(ActuatorCommand::Home).await;
                // Homing runs asynchronously in the driver; report accepted.
                ok(
                    id,
                    Res::ResponseSliderCalibrate(ResponseSliderCalibrate { success: true }),
                )
            }

            // ───────────── HSP (script playback) ─────────────
            Params::RequestHspSetup(s) => {
                {
                    let mut st = self.state.write().await;
                    st.hsp_buffer.clear();
                    st.hsp = crate::state::HspRuntime {
                        stream_id: s.stream_id,
                        play_state: HspPlayState::HspStateStopped,
                        ..Default::default()
                    };
                    st.set_mode(AppMode::Hsp);
                }
                let _ = self.hsp_tx.send(HspControl::Setup).await;
                self.hsp_resp(id, |s| Res::ResponseHspSetup(ResponseHspSetup { state: s }))
                    .await
            }
            Params::RequestHspAdd(a) => {
                {
                    let mut st = self.state.write().await;
                    if a.flush {
                        st.hsp_buffer.clear();
                    }
                    st.hsp_buffer.extend(a.points.iter().cloned());
                    if a.tail_point_threshold != 0 {
                        st.hsp.tail_point_threshold = a.tail_point_threshold;
                    }
                    // Absolute index of the last point in the stream — lets the
                    // HSP task tell "script complete" from "buffer underrun".
                    st.hsp.tail_point_stream_index = a.tail_point_stream_index as i32;
                    refresh_hsp_buffer_times(&mut st);
                }
                let _ = self.hsp_tx.send(HspControl::Added).await;
                crate::telemetry::metrics::hsp_points_added(a.points.len() as u64);
                self.hsp_resp(id, |s| Res::ResponseHspAdd(ResponseHspAdd { state: s }))
                    .await
            }
            Params::RequestHspFlush(_) => {
                {
                    let mut st = self.state.write().await;
                    st.hsp_buffer.clear();
                    refresh_hsp_buffer_times(&mut st);
                }
                self.hsp_resp(id, |s| Res::ResponseHspFlush(ResponseHspFlush { state: s }))
                    .await
            }
            Params::RequestHspPlay(p) => {
                {
                    let mut st = self.state.write().await;
                    st.hsp.looped = p.r#loop;
                    st.hsp.playback_rate = if p.playback_rate > 0.0 {
                        p.playback_rate
                    } else {
                        1.0
                    };
                    st.hsp.pause_on_starving = p.pause_on_starving;
                    st.hsp.play_state = HspPlayState::HspStatePlaying;
                    st.set_mode(AppMode::Hsp);
                }
                let _ = self
                    .hsp_tx
                    .send(HspControl::Play {
                        start_time: p.start_time,
                        server_time: p.server_time,
                        playback_rate: p.playback_rate,
                        looped: p.r#loop,
                        pause_on_starving: p.pause_on_starving,
                    })
                    .await;
                self.hsp_resp(id, |s| Res::ResponseHspPlay(ResponseHspPlay { state: s }))
                    .await
            }
            Params::RequestHspStop(_) => {
                self.state.write().await.hsp.play_state = HspPlayState::HspStateStopped;
                let _ = self.hsp_tx.send(HspControl::Stop).await;
                self.send_cmd(ActuatorCommand::Stop).await;
                self.hsp_resp(id, |s| Res::ResponseHspStop(ResponseHspStop { state: s }))
                    .await
            }
            Params::RequestHspPause(_) => {
                self.state.write().await.hsp.play_state = HspPlayState::HspStatePaused;
                let _ = self.hsp_tx.send(HspControl::Pause).await;
                self.hsp_resp(id, |s| Res::ResponseHspPause(ResponseHspPause { state: s }))
                    .await
            }
            Params::RequestHspResume(r) => {
                self.state.write().await.hsp.play_state = HspPlayState::HspStatePlaying;
                let _ = self
                    .hsp_tx
                    .send(HspControl::Resume { pick_up: r.pick_up })
                    .await;
                self.hsp_resp(id, |s| {
                    Res::ResponseHspResume(ResponseHspResume { state: s })
                })
                .await
            }
            Params::RequestHspStateGet(_) => {
                self.hsp_resp(id, |s| {
                    Res::ResponseHspStateGet(ResponseHspStateGet { state: s })
                })
                .await
            }
            Params::RequestHspCurrentTimeSet(c) => {
                let _ = self
                    .hsp_tx
                    .send(HspControl::SetCurrentTime {
                        current_time: c.current_time,
                        server_time: c.server_time,
                        filter: c.filter,
                    })
                    .await;
                self.hsp_resp(id, |s| {
                    Res::ResponseHspCurrentTimeSet(ResponseHspCurrentTimeSet { state: s })
                })
                .await
            }
            Params::RequestHspThresholdSet(t) => {
                self.state.write().await.hsp.tail_point_threshold = t.tail_point_threshold;
                self.hsp_resp(id, |s| {
                    Res::ResponseHspThresholdSet(ResponseHspThresholdSet { state: s })
                })
                .await
            }
            Params::RequestHspPauseOnStarvingSet(p) => {
                self.state.write().await.hsp.pause_on_starving = p.pause_on_starving;
                self.hsp_resp(id, |s| {
                    Res::ResponseHspPauseOnStarvingSet(ResponseHspPauseOnStarvingSet { state: s })
                })
                .await
            }
            Params::RequestHspPlaybackRateSet(r) => {
                let rate = if r.playback_rate > 0.0 {
                    r.playback_rate
                } else {
                    1.0
                };
                self.state.write().await.hsp.playback_rate = rate;
                let _ = self.hsp_tx.send(HspControl::SetPlaybackRate(rate)).await;
                self.hsp_resp(id, |s| {
                    Res::ResponseHspPlaybackRateSet(ResponseHspPlaybackRateSet { state: s })
                })
                .await
            }
            Params::RequestHspLoopSet(l) => {
                self.state.write().await.hsp.looped = l.r#loop;
                let _ = self.hsp_tx.send(HspControl::SetLoop(l.r#loop)).await;
                self.hsp_resp(id, |s| {
                    Res::ResponseHspLoopSet(ResponseHspLoopSet { state: s })
                })
                .await
            }

            // ───────────── wifi (stub: BLE bridge, no managed wifi) ─────────────
            // The official app queries wifi status during connection setup and
            // aborts if it errors. We have no wifi stack, so report a clean
            // "disconnected" status; that satisfies the app over BLE.
            Params::RequestWifiStatusGet(_) => ok(
                id,
                Res::ResponseWifiStatusGet(ResponseWifiStatusGet {
                    ap_info: None,
                    state: WifiState::Disconnected as i32,
                    failed_reason: 0,
                    socket_connected: false,
                    ssid: String::new(),
                }),
            ),

            // ───────────── unsupported (HVP/HRPP/etc.) ─────────────
            other => {
                warn!(kind = params_label(&other), detail = ?other, "unsupported request");
                Response::err(
                    id,
                    HandyErrorCodes::ErrorNotImplemented as i32,
                    format!("{} not implemented by this bridge", params_label(&other)),
                )
            }
        }
    }

    // ───────────────────────── helpers ─────────────────────────

    /// Switch coarse mode, stopping the previous mode's task as needed.
    async fn apply_mode(&self, mode: AppMode) -> u32 {
        let prev = {
            let st = self.state.read().await;
            st.mode
        };
        if prev == AppMode::Hamp && mode != AppMode::Hamp {
            self.state.write().await.hamp.running = false;
            let _ = self.hamp_tx.send(HampControl::Stop).await;
        }
        if prev == AppMode::Hsp && mode != AppMode::Hsp {
            let _ = self.hsp_tx.send(HspControl::Stop).await;
        }
        if prev == AppMode::Drill && mode != AppMode::Drill {
            let _ = self.modes.drill.send(DrillControl::Stop).await;
        }
        if prev == AppMode::Ramp && mode != AppMode::Ramp {
            let _ = self.modes.ramp.send(RampControl::Stop).await;
        }
        if prev == AppMode::Game && mode != AppMode::Game {
            let _ = self.modes.game.send(GameControl::Stop).await;
        }
        if prev == AppMode::Cycle && mode != AppMode::Cycle {
            let _ = self.modes.cycle.send(CycleControl::Stop).await;
        }
        if prev == AppMode::Learn && mode != AppMode::Learn {
            let _ = self.modes.learn.send(LearnControl::Stop).await;
        }
        if prev == AppMode::Pulse && mode != AppMode::Pulse {
            let _ = self.modes.pulse.send(PulseControl::Stop).await;
        }
        if prev == AppMode::Impale && mode != AppMode::Impale {
            let _ = self.modes.impale.send(ImpaleControl::Stop).await;
        }
        if prev == AppMode::Plumb && mode != AppMode::Plumb {
            let _ = self.modes.plumb.send(PlumbControl::Stop).await;
        }
        if prev == AppMode::Surge && mode != AppMode::Surge {
            let _ = self.modes.surge.send(SurgeControl::Stop).await;
        }
        if prev == AppMode::Tide && mode != AppMode::Tide {
            let _ = self.modes.tide.send(TideControl::Stop).await;
        }
        if prev == AppMode::Echo && mode != AppMode::Echo {
            let _ = self.modes.echo.send(EchoControl::Stop).await;
        }
        if prev == AppMode::Trace && mode != AppMode::Trace {
            let _ = self.modes.trace.send(TraceControl::Stop).await;
        }
        if prev == AppMode::Tempo && mode != AppMode::Tempo {
            let _ = self.modes.tempo.send(TempoControl::Stop).await;
        }
        self.state.write().await.set_mode(mode)
    }

    /// Route a hardware hand-switch edge to the active program, mirroring that
    /// program's on-screen button (see `src/modes/handswitch.rs`). Both inputs
    /// feed the same control channels, so they're interchangeable — except the
    /// games' ready gesture, which is hardware-only (see [`GameControl::HardwareTap`]).
    pub async fn hand_switch(&self, edge: HandEdge) {
        let mode = self.state.read().await.mode;
        match mode {
            // Deadman: resend the push pulse while held; the drill task drops the
            // servo on its own when the heartbeat lapses after release.
            AppMode::Drill => {
                if matches!(edge, HandEdge::Press | HandEdge::Hold) {
                    let _ = self
                        .modes
                        .drill
                        .send(DrillControl::Push {
                            feed_rate_mm_s: None,
                        })
                        .await;
                }
            }
            // Deadman heartbeat: held = down, released = up. A fresh press
            // also counts as a hardware tap toward the triple-tap ready
            // gesture (ignored by the game task once play has started).
            AppMode::Game => match edge {
                HandEdge::Press => {
                    let _ = self.modes.game.send(GameControl::HardwareTap).await;
                    let _ = self
                        .modes
                        .game
                        .send(GameControl::Button { down: true })
                        .await;
                }
                HandEdge::Hold => {
                    let _ = self
                        .modes
                        .game
                        .send(GameControl::Button { down: true })
                        .await;
                }
                HandEdge::Release => {
                    let _ = self
                        .modes
                        .game
                        .send(GameControl::Button { down: false })
                        .await;
                }
            },
            // Deadman heartbeat: held extends the rod, released brakes it and
            // arms the auto-retract timer (mirrors the on-screen hold button).
            AppMode::Impale => match edge {
                HandEdge::Press | HandEdge::Hold => {
                    let _ = self
                        .modes
                        .impale
                        .send(ImpaleControl::Button { down: true })
                        .await;
                }
                HandEdge::Release => {
                    let _ = self
                        .modes
                        .impale
                        .send(ImpaleControl::Button { down: false })
                        .await;
                }
            },
            // Forward edges; the cycle task times short=next / long(≥2s)=pause.
            AppMode::Cycle => match edge {
                HandEdge::Press => {
                    let _ = self
                        .modes
                        .cycle
                        .send(CycleControl::Button { down: true })
                        .await;
                }
                HandEdge::Release => {
                    let _ = self
                        .modes
                        .cycle
                        .send(CycleControl::Button { down: false })
                        .await;
                }
                HandEdge::Hold => {}
            },
            // Single tap advances the phase machine.
            AppMode::Learn => {
                if edge == HandEdge::Press {
                    let _ = self.modes.learn.send(LearnControl::Button).await;
                }
            }
            // A tap builds intensity (mirrors the on-screen "build" nudge).
            AppMode::Ramp => {
                if edge == HandEdge::Press {
                    let _ = self
                        .modes
                        .ramp
                        .send(RampControl::Nudge { delta: 0.1 })
                        .await;
                }
            }
            // Toggle/host modes: the bridge only tracks the *running* program, not
            // the UI tab, so the switch can't start an idle one. While one runs, a
            // press stops it — a handy physical kill-switch.
            AppMode::Pulse | AppMode::Hamp | AppMode::Hsp | AppMode::Hdsp => {
                if edge == HandEdge::Press {
                    self.apply_mode(AppMode::Idle).await;
                }
            }
            // These programs read the hand-switch bit directly in their own
            // task loops (tap/hold is their core input), so the central router
            // must not also act on their edges.
            AppMode::Plumb
            | AppMode::Surge
            | AppMode::Tide
            | AppMode::Echo
            | AppMode::Trace
            | AppMode::Tempo => {}
            AppMode::Idle | AppMode::Homing | AppMode::Uninitialized => {}
        }
    }

    /// Stop all active motion and return to Idle (transport loss / StopCurrentMode).
    pub async fn stop_everything(&self) {
        {
            let mut st = self.state.write().await;
            st.hamp.running = false;
            st.hsp.play_state = HspPlayState::HspStateStopped;
            st.set_mode(AppMode::Idle);
        }
        let _ = self.hamp_tx.send(HampControl::Stop).await;
        let _ = self.hsp_tx.send(HspControl::Stop).await;
        let _ = self.modes.drill.send(DrillControl::Stop).await;
        let _ = self.modes.ramp.send(RampControl::Stop).await;
        let _ = self.modes.game.send(GameControl::Stop).await;
        let _ = self.modes.cycle.send(CycleControl::Stop).await;
        let _ = self.modes.learn.send(LearnControl::Stop).await;
        let _ = self.modes.pulse.send(PulseControl::Stop).await;
        let _ = self.modes.impale.send(ImpaleControl::Stop).await;
        let _ = self.modes.plumb.send(PlumbControl::Stop).await;
        let _ = self.modes.surge.send(SurgeControl::Stop).await;
        let _ = self.modes.tide.send(TideControl::Stop).await;
        let _ = self.modes.echo.send(EchoControl::Stop).await;
        let _ = self.modes.trace.send(TraceControl::Stop).await;
        let _ = self.modes.tempo.send(TempoControl::Stop).await;
        // Safety: StopAll must also zero any external e-stim device.
        let _ = self
            .modes
            .coyote
            .send(crate::devices::CoyoteControl::Stop)
            .await;
        self.send_cmd(ActuatorCommand::Stop).await;
    }

    /// Issue an HDSP move with an explicit velocity.
    async fn hdsp_move(
        &self,
        mut pos_mm: f32,
        vel_mm_s: f32,
        absolute: bool,
        _stop_on_target: bool,
    ) {
        if !absolute {
            // already converted to mm by the caller for percent variants
        }
        pos_mm = self.translator().await.clamp_mm(pos_mm);
        self.apply_mode(AppMode::Hdsp).await;
        crate::telemetry::metrics::hdsp_command();
        self.send_cmd(ActuatorCommand::MoveTo {
            pos_mm,
            vel_mm_s,
            accel_g: self.default_accel_g,
            profile: self.profile,
            // HDSP is realtime direct control; never insert ramp latency.
            soften: false,
        })
        .await;
    }

    /// Issue an HDSP move that should complete in `t_ms`, deriving the velocity
    /// from the distance to travel (SPEC §7.4).
    async fn hdsp_move_duration(&self, target_mm: f32, t_ms: u32, _absolute: bool) {
        let target_mm = self.translator().await.clamp_mm(target_mm);
        let current = self.state.read().await.position_mm;
        let vel = Translator::duration_to_vel(target_mm - current, t_ms);
        self.apply_mode(AppMode::Hdsp).await;
        crate::telemetry::metrics::hdsp_command();
        self.send_cmd(ActuatorCommand::MoveTo {
            pos_mm: target_mm,
            vel_mm_s: vel,
            accel_g: self.default_accel_g,
            profile: self.profile,
            soften: false,
        })
        .await;
    }

    async fn stroke_get(&self) -> ResponseSliderStrokeGet {
        let st = self.state.read().await;
        ResponseSliderStrokeGet {
            min: st.slide_min,
            max: st.slide_max,
            min_absolute: st.slide_min * self.stroke_mm,
            max_absolute: st.slide_max * self.stroke_mm,
        }
    }

    async fn hamp_state(&self) -> HampState {
        let st = self.state.read().await;
        HampState {
            play_state: st.hamp.play_state() as i32,
            velocity: st.hamp.velocity,
            direction: st.hamp.direction,
            min: st.hamp.min,
            max: st.hamp.max,
        }
    }

    /// Build a HAMP response: read current `HampState`, hand it to a closure
    /// that wraps it in the appropriate `Response*` variant.
    async fn hamp_resp(&self, id: u32, f: impl FnOnce(Option<HampState>) -> Res) -> Response {
        let state = Some(self.hamp_state().await);
        ok(id, f(state))
    }

    async fn hsp_state(&self) -> HspState {
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

    async fn hsp_resp(&self, id: u32, f: impl FnOnce(Option<HspState>) -> Res) -> Response {
        let state = Some(self.hsp_state().await);
        ok(id, f(state))
    }
}

/// Build an OK response with a result.
fn ok(id: u32, result: Res) -> Response {
    Response {
        id,
        result: Some(result),
        error: None,
    }
}

/// Recompute the buffer head/tail point times after a buffer change.
fn refresh_hsp_buffer_times(st: &mut AppState) {
    st.hsp.first_point_time = st.hsp_buffer.first().map(|p| p.t).unwrap_or(0);
    st.hsp.last_point_time = st.hsp_buffer.last().map(|p| p.t).unwrap_or(0);
}

fn app_mode_to_proto(m: AppMode) -> Mode {
    match m {
        AppMode::Hamp => Mode::Hamp,
        AppMode::Hdsp => Mode::Hdsp,
        AppMode::Hsp => Mode::Hsp,
        _ => Mode::Idle,
    }
}

fn proto_mode_to_app(mode: i32) -> AppMode {
    match Mode::try_from(mode).unwrap_or(Mode::Idle) {
        Mode::Hamp => AppMode::Hamp,
        Mode::Hdsp => AppMode::Hdsp,
        Mode::Hsp | Mode::Hssp => AppMode::Hsp,
        _ => AppMode::Idle,
    }
}

/// Short label for a request (telemetry / logging); `"Empty"` if no params.
pub fn request_label(req: &Request) -> &'static str {
    match &req.params {
        Some(p) => params_label(p),
        None => "Empty",
    }
}

/// Short label for a request variant (telemetry / logging).
fn params_label(p: &Params) -> &'static str {
    macro_rules! m {
        ($($v:ident),* $(,)?) => {
            match p { $(Params::$v(_) => stringify!($v),)* #[allow(unreachable_patterns)] _ => "Unknown" }
        };
    }
    m!(
        RequestConnectionKeyGet,
        RequestModeGet,
        RequestModeSet,
        RequestStopCurrentMode,
        RequestConnectionModeGet,
        RequestConnectionModeSet,
        RequestCapabilitiesGet,
        RequestSessionIdsGet,
        RequestBatteryGet,
        RequestClockOffsetSet,
        RequestClockOffsetGet,
        RequestHampStart,
        RequestHampStop,
        RequestHampVelocitySet,
        RequestHampZoneSet,
        RequestHampStateGet,
        RequestHdspXaVaSet,
        RequestHdspXpVaSet,
        RequestHdspXpVpSet,
        RequestHdspXaTSet,
        RequestHdspXpTSet,
        RequestHdspXaVpSet,
        RequestHdspStop,
        RequestSliderStrokeGet,
        RequestSliderStrokeSet,
        RequestSliderStateGet,
        RequestSliderCalibrate,
        RequestHspSetup,
        RequestHspAdd,
        RequestHspFlush,
        RequestHspPlay,
        RequestHspStop,
        RequestHspPause,
        RequestHspResume,
        RequestHspStateGet,
        RequestHspCurrentTimeSet,
        RequestHspThresholdSet,
        RequestHspPauseOnStarvingSet,
        RequestHspPlaybackRateSet,
        RequestHspLoopSet,
    )
}

/// Transport loss handler: stop everything (SPEC §10). Re-exported for transports.
pub async fn on_transport_lost(d: &Dispatcher) {
    d.stop_everything().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::{HampControl, HspControl};

    struct Harness {
        d: Dispatcher,
        state: Arc<RwLock<AppState>>,
        cmd_rx: mpsc::Receiver<ActuatorCommand>,
        hamp_rx: mpsc::Receiver<HampControl>,
        hsp_rx: mpsc::Receiver<HspControl>,
    }

    fn harness() -> Harness {
        let cfg: Config = toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            max_velocity_mm_s = 400.0
            default_accel_g = 0.3
        "#,
        )
        .unwrap();
        let state = Arc::new(RwLock::new(AppState::new("uid".into(), 7)));
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (notif_tx, _notif_rx) = broadcast::channel(16);
        let (hamp_tx, hamp_rx) = mpsc::channel(16);
        let (hsp_tx, hsp_rx) = mpsc::channel(16);
        let (drill_tx, _drill_rx) = mpsc::channel(16);
        let (ramp_tx, _ramp_rx) = mpsc::channel(16);
        let (game_tx, _game_rx) = mpsc::channel(16);
        let (cycle_tx, _cycle_rx) = mpsc::channel(16);
        let (learn_tx, _learn_rx) = mpsc::channel(16);
        let (pulse_tx, _pulse_rx) = mpsc::channel(16);
        let (impale_tx, _impale_rx) = mpsc::channel(16);
        let (coyote_tx, _coyote_rx) = mpsc::channel(16);
        let (sensor_tx, _sensor_rx) = mpsc::channel(16);
        let (plumb_tx, _plumb_rx) = mpsc::channel(16);
        let (surge_tx, _surge_rx) = mpsc::channel(16);
        let (tide_tx, _tide_rx) = mpsc::channel(16);
        let (echo_tx, _echo_rx) = mpsc::channel(16);
        let (trace_tx, _trace_rx) = mpsc::channel(16);
        let (tempo_tx, _tempo_rx) = mpsc::channel(16);
        let modes = crate::modes::ModeControls {
            drill: drill_tx,
            ramp: ramp_tx,
            game: game_tx,
            cycle: cycle_tx,
            learn: learn_tx,
            pulse: pulse_tx,
            impale: impale_tx,
            coyote: coyote_tx,
            sensors: sensor_tx,
            plumb: plumb_tx,
            surge: surge_tx,
            tide: tide_tx,
            echo: echo_tx,
            trace: trace_tx,
            tempo: tempo_tx,
        };
        let d = Dispatcher::new(
            state.clone(),
            cmd_tx,
            notif_tx,
            hamp_tx,
            hsp_tx,
            modes,
            &cfg,
        );
        Harness {
            d,
            state,
            cmd_rx,
            hamp_rx,
            hsp_rx,
        }
    }

    fn req(params: Params) -> Request {
        Request {
            params: Some(params),
            id: 42,
        }
    }

    #[tokio::test]
    async fn hamp_start_sets_running_and_returns_state() {
        let mut h = harness();
        let resp =
            h.d.handle_request(req(Params::RequestHampStart(RequestHampStart {})))
                .await;
        assert_eq!(resp.id, 42);
        assert!(matches!(
            resp.result,
            Some(Res::ResponseHampStart(ResponseHampStart { state: Some(_) }))
        ));
        assert!(h.state.read().await.hamp.running);
        assert_eq!(h.state.read().await.mode, AppMode::Hamp);
        assert_eq!(h.hamp_rx.try_recv().unwrap(), HampControl::Start);
    }

    #[tokio::test]
    async fn hdsp_xava_emits_move_command() {
        let mut h = harness();
        let resp =
            h.d.handle_request(req(Params::RequestHdspXaVaSet(RequestHdspXaVaSet {
                xa: 150.0,
                va: 200.0,
                stop_on_target: false,
            })))
            .await;
        // HDSP replies blank (OK).
        assert!(resp.result.is_none() && resp.error.is_none());
        let cmd = h.cmd_rx.try_recv().unwrap();
        match cmd {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert_eq!(pos_mm, 150.0);
                assert_eq!(vel_mm_s, 200.0);
            }
            other => panic!("expected MoveTo, got {other:?}"),
        }
        assert_eq!(h.state.read().await.mode, AppMode::Hdsp);
    }

    #[tokio::test]
    async fn hdsp_xpvp_maps_percent_through_full_stroke() {
        let mut h = harness();
        // 50% position, 50% velocity on a 300mm / 400mm·s⁻¹ rig.
        h.d.handle_request(req(Params::RequestHdspXpVpSet(RequestHdspXpVpSet {
            xp: 0.5,
            vp: 0.5,
            stop_on_target: true,
        })))
        .await;
        match h.cmd_rx.try_recv().unwrap() {
            ActuatorCommand::MoveTo {
                pos_mm, vel_mm_s, ..
            } => {
                assert_eq!(pos_mm, 150.0);
                assert_eq!(vel_mm_s, 200.0);
            }
            other => panic!("expected MoveTo, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn slider_stroke_set_updates_zone_and_absolutes() {
        let h = harness();
        let resp =
            h.d.handle_request(req(Params::RequestSliderStrokeSet(
                RequestSliderStrokeSet {
                    min: 0.25,
                    max: 0.75,
                },
            )))
            .await;
        match resp.result {
            Some(Res::ResponseSliderStrokeSet(s)) => {
                assert_eq!(s.min, 0.25);
                assert_eq!(s.max, 0.75);
                assert_eq!(s.min_absolute, 75.0); // 0.25 * 300
                assert_eq!(s.max_absolute, 225.0);
            }
            other => panic!("unexpected: {other:?}"),
        }
        let st = h.state.read().await;
        assert_eq!((st.slide_min, st.slide_max), (0.25, 0.75));
    }

    #[tokio::test]
    async fn hsp_setup_then_add_buffers_points() {
        let mut h = harness();
        h.d.handle_request(req(Params::RequestHspSetup(RequestHspSetup {
            stream_id: 9,
        })))
        .await;
        assert_eq!(h.hsp_rx.try_recv().unwrap(), HspControl::Setup);
        let pts = vec![Point { t: 0, x: 0 }, Point { t: 100, x: 255 }];
        let resp =
            h.d.handle_request(req(Params::RequestHspAdd(RequestHspAdd {
                points: pts,
                flush: false,
                tail_point_stream_index: 1,
                tail_point_threshold: 0,
            })))
            .await;
        match resp.result {
            Some(Res::ResponseHspAdd(a)) => {
                let s = a.state.unwrap();
                assert_eq!(s.points, 2);
                assert_eq!(s.stream_id, 9);
                assert_eq!(s.last_point_time, 100);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(h.hsp_rx.try_recv().unwrap(), HspControl::Added);
    }

    #[tokio::test]
    async fn unsupported_request_returns_not_implemented() {
        let mut h = harness();
        let resp =
            h.d.handle_request(req(Params::RequestHvpStart(RequestHvpStart {})))
                .await;
        let err = resp.error.unwrap();
        assert_eq!(err.code, HandyErrorCodes::ErrorNotImplemented as i32);
        // no actuator command emitted
        assert!(h.cmd_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn capabilities_advertise_single_slider_and_mtu() {
        let h = harness();
        let resp =
            h.d.handle_request(req(Params::RequestCapabilitiesGet(
                RequestCapabilitiesGet {},
            )))
            .await;
        match resp.result {
            Some(Res::ResponseCapabilitiesGet(c)) => {
                assert_eq!(c.slider, 1);
                assert_eq!(c.ble_mtu, 512);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
