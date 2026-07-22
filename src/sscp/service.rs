//! SSCP characteristic helpers (Linux / BlueZ only).
//!
//! The four SSCP characteristics live **inside the existing Handy FW4 service**
//! (`SERVICE_UUID` in `transport/ble.rs`). Handy clients ignore unknown UUIDs;
//! the Rod web app discovers them after connecting via the Handy UUID.
//!
//! No new GATT service is registered; no advertisement change is needed.

// Characteristic UUIDs (within the Handy FW4 service).
pub const CHAR_TELEMETRY: &str = "7e400002-b5a3-f393-e0a9-e50e24dc4179"; // Notify
pub const CHAR_COMMAND:   &str = "7e400003-b5a3-f393-e0a9-e50e24dc4179"; // Write
pub const CHAR_ACK:       &str = "7e400004-b5a3-f393-e0a9-e50e24dc4179"; // Notify
pub const CHAR_DEV_INFO:  &str = "7e400005-b5a3-f393-e0a9-e50e24dc4179"; // Read

#[cfg(target_os = "linux")]
pub use imp::{make_sscp_characteristics, spawn_sscp_tasks, SscpControls};

#[cfg(target_os = "linux")]
mod imp {
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;

    use bluer::gatt::local::{
        characteristic_control, Characteristic, CharacteristicControl,
        CharacteristicControlEvent, CharacteristicNotify, CharacteristicNotifyMethod,
        CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod,
    };
    use bluer::gatt::CharacteristicWriter;
    use bluer::Uuid;
    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{mpsc, RwLock};
    use tracing::{info, warn};

    use super::*;
    use crate::config::Config;
    use crate::modes::{
        CycleControl, DrillControl, EchoControl, GameControl, ImpaleControl, LearnControl,
        ModeControls, PlumbControl, PulseControl, RampControl, SurgeControl, TempoControl,
        TideControl, TraceControl,
    };
    use crate::rpc::{
        request::Params, Request, RequestHampStart, RequestHampStop,
        RequestHampVelocitySet, RequestHampZoneSet, RequestHdspStop, RequestHdspXpVpSet,
    };
    use crate::rpc::dispatch::Dispatcher;
    use crate::sscp::{build_telemetry, Command, CommandAck};
    use crate::state::{AppState, BridgeCommand};

    const MTU: usize = 512;
    /// 10 Hz — enough for smooth UI, half the BLE write pressure vs. the 80 ms
    /// Modbus poll cadence.  MissedTickBehavior::Skip prevents burst catch-up
    /// when a write_all takes longer than the interval (BLE backpressure).
    const TELEMETRY_INTERVAL: Duration = Duration::from_millis(100);

    /// I/O handles returned alongside the characteristics.
    pub struct SscpControls {
        pub tel_control: CharacteristicControl,
        pub cmd_control: CharacteristicControl,
        pub ack_control: CharacteristicControl,
        pub modes: ModeControls,
    }

    /// Build the four SSCP `Characteristic` structs.
    /// Caller appends them to the existing Handy FW4 service's characteristics vec.
    pub fn make_sscp_characteristics(cfg: &Config, modes: ModeControls) -> anyhow::Result<(Vec<Characteristic>, SscpControls)> {
        use anyhow::Context as _;

        let tel_uuid      = Uuid::from_str(CHAR_TELEMETRY).context("telemetry UUID")?;
        let cmd_uuid      = Uuid::from_str(CHAR_COMMAND).context("command UUID")?;
        let ack_uuid      = Uuid::from_str(CHAR_ACK).context("ack UUID")?;
        let dev_info_uuid = Uuid::from_str(CHAR_DEV_INFO).context("dev-info UUID")?;

        let dev_info_json = {
            let info = crate::sscp::DeviceInfo {
                firmware_version: env!("CARGO_PKG_VERSION"),
                stroke_mm: cfg.stroke_mm(),
                device_name: "Rod".to_string(),
                sscp_version: 1,
            };
            serde_json::to_vec(&info).unwrap_or_default()
        };

        let (tel_control, tel_handle) = characteristic_control();
        let (cmd_control, cmd_handle) = characteristic_control();
        let (ack_control, ack_handle) = characteristic_control();

        let chars = vec![
            // Telemetry: device → app, Notify, ~80 ms.
            Characteristic {
                uuid: tel_uuid,
                notify: Some(CharacteristicNotify {
                    notify: true,
                    method: CharacteristicNotifyMethod::Io,
                    ..Default::default()
                }),
                control_handle: tel_handle,
                ..Default::default()
            },
            // Command: app → device, Write (with and without response).
            Characteristic {
                uuid: cmd_uuid,
                write: Some(CharacteristicWrite {
                    write: true,
                    write_without_response: true,
                    method: CharacteristicWriteMethod::Io,
                    ..Default::default()
                }),
                control_handle: cmd_handle,
                ..Default::default()
            },
            // Ack: device → app, Notify.
            Characteristic {
                uuid: ack_uuid,
                notify: Some(CharacteristicNotify {
                    notify: true,
                    method: CharacteristicNotifyMethod::Io,
                    ..Default::default()
                }),
                control_handle: ack_handle,
                ..Default::default()
            },
            // DevInfo: app reads once after connect.
            Characteristic {
                uuid: dev_info_uuid,
                read: Some(CharacteristicRead {
                    read: true,
                    fun: Box::new(move |_req| {
                        let data = dev_info_json.clone();
                        Box::pin(async move { Ok(data) })
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ];

        Ok((chars, SscpControls { tel_control, cmd_control, ack_control, modes }))
    }

    /// Spawn the telemetry poll and command handler tasks.
    pub fn spawn_sscp_tasks(
        state: Arc<RwLock<AppState>>,
        dispatcher: Dispatcher,
        bridge_tx: mpsc::Sender<BridgeCommand>,
        cfg: Config,
        controls: SscpControls,
    ) {
        tokio::spawn(telemetry_task(state.clone(), cfg, controls.tel_control));
        tokio::spawn(command_task(
            state,
            dispatcher,
            bridge_tx,
            controls.modes,
            controls.cmd_control,
            controls.ack_control,
        ));
    }

    // ── Telemetry task ───────────────────────────────────────────────────────

    async fn telemetry_task(
        state: Arc<RwLock<AppState>>,
        cfg: Config,
        mut tel_control: CharacteristicControl,
    ) {
        let mut writer: Option<CharacteristicWriter> = None;
        let mut ticker = tokio::time::interval(TELEMETRY_INTERVAL);
        // Skip rather than burst: if write_all blocks past the interval (BLE
        // backpressure), the missed tick is dropped instead of queuing up.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Last payload sent — skip the write if the frame is identical.
        let mut last_json: Vec<u8> = Vec::new();

        loop {
            tokio::select! {
                evt = tel_control.next() => match evt {
                    Some(CharacteristicControlEvent::Notify(w)) => {
                        info!("SSCP central subscribed to telemetry");
                        last_json.clear(); // force a full frame on new subscriber
                        writer = Some(w);
                    }
                    Some(_) => {}
                    None => break,
                },
                _ = ticker.tick() => {
                    let Some(w) = writer.as_mut() else { continue };
                    let json = {
                        let st = state.read().await;
                        match serde_json::to_vec(&build_telemetry(&st, &cfg)) {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "SSCP telemetry serialise failed");
                                continue;
                            }
                        }
                    };
                    // Skip write if nothing changed (e.g. machine idle between polls).
                    if json == last_json {
                        continue;
                    }
                    if let Err(e) = w.write_all(&json).await {
                        warn!(error = %e, "SSCP telemetry notify failed; dropping subscriber");
                        writer = None;
                    } else {
                        last_json = json;
                    }
                }
            }
        }
    }

    // ── Command task ─────────────────────────────────────────────────────────

    async fn command_task(
        state: Arc<RwLock<AppState>>,
        dispatcher: Dispatcher,
        bridge_tx: mpsc::Sender<BridgeCommand>,
        modes: ModeControls,
        mut cmd_control: CharacteristicControl,
        mut ack_control: CharacteristicControl,
    ) {
        let (ack_tx, mut ack_rx) = mpsc::channel::<Vec<u8>>(16);
        let mut ack_writer: Option<CharacteristicWriter> = None;
        loop {
            tokio::select! {
                evt = ack_control.next() => match evt {
                    Some(CharacteristicControlEvent::Notify(w)) => {
                        info!("SSCP central subscribed to ack");
                        ack_writer = Some(w);
                    }
                    Some(_) => {}
                    None => break,
                },
                ack_bytes = ack_rx.recv() => {
                    let Some(bytes) = ack_bytes else { break };
                    if let Some(w) = ack_writer.as_mut() {
                        if let Err(e) = w.write_all(&bytes).await {
                            warn!(error = %e, "SSCP ack notify failed");
                            ack_writer = None;
                        }
                    }
                },
                evt = cmd_control.next() => {
                    let Some(CharacteristicControlEvent::Write(req)) = evt else { continue };
                    let mut reader = match req.accept() {
                        Ok(r) => r,
                        Err(e) => { warn!(error = %e, "SSCP cmd accept failed"); continue; }
                    };
                    let state2  = state.clone();
                    let disp2   = dispatcher.clone();
                    let bridge2 = bridge_tx.clone();
                    let modes2  = modes.clone();
                    let ack2    = ack_tx.clone();
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; MTU];
                        let n = match reader.read(&mut buf).await {
                            Ok(n) if n > 0 => n,
                            _ => return,
                        };
                        handle_command(&buf[..n], state2, disp2, bridge2, modes2, ack2).await;
                    });
                }
            }
        }
    }

    async fn handle_command(
        bytes: &[u8],
        state: Arc<RwLock<AppState>>,
        dispatcher: Dispatcher,
        bridge_tx: mpsc::Sender<BridgeCommand>,
        modes: ModeControls,
        ack_tx: mpsc::Sender<Vec<u8>>,
    ) {
        let seq: u32 = serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|v| v.get("seq").and_then(|s| s.as_u64()))
            .unwrap_or(0) as u32;

        let cmd: Command = match serde_json::from_slice(bytes) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "SSCP command deserialise failed");
                send_ack(&ack_tx, seq, false, Some(e.to_string())).await;
                return;
            }
        };

        let result = dispatch_command(cmd, state, dispatcher, bridge_tx, modes).await;
        let (ok, error) = match result {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e)),
        };
        send_ack(&ack_tx, seq, ok, error).await;
    }

    async fn dispatch_command(
        cmd: Command,
        state: Arc<RwLock<AppState>>,
        dispatcher: Dispatcher,
        bridge_tx: mpsc::Sender<BridgeCommand>,
        modes: ModeControls,
    ) -> Result<(), String> {
        match cmd {
            Command::StopAll => {
                dispatcher.stop_everything().await;
            }
            Command::HampStart => {
                dispatcher.handle_request(Request {
                    id: 0,
                    params: Some(Params::RequestHampStart(RequestHampStart {})),
                }).await;
            }
            Command::HampStop => {
                dispatcher.handle_request(Request {
                    id: 0,
                    params: Some(Params::RequestHampStop(RequestHampStop {})),
                }).await;
            }
            Command::HampConfig { velocity, zone_min, zone_max, softness } => {
                if let Some(v) = velocity {
                    dispatcher.handle_request(Request {
                        id: 0,
                        params: Some(Params::RequestHampVelocitySet(
                            RequestHampVelocitySet { velocity: v },
                        )),
                    }).await;
                }
                if let (Some(mn), Some(mx)) = (zone_min, zone_max) {
                    dispatcher.handle_request(Request {
                        id: 0,
                        params: Some(Params::RequestHampZoneSet(
                            RequestHampZoneSet { min: mn, max: mx },
                        )),
                    }).await;
                }
                if let Some(s) = softness {
                    // No Handy RPC equivalent — write directly; HampTask picks it up.
                    state.write().await.hamp.softness = s.clamp(0.0, 1.0);
                }
            }
            Command::HdspMove { position_pct, velocity_pct } => {
                dispatcher.handle_request(Request {
                    id: 0,
                    params: Some(Params::RequestHdspXpVpSet(RequestHdspXpVpSet {
                        xp: position_pct,
                        vp: velocity_pct,
                        stop_on_target: false,
                    })),
                }).await;
            }
            Command::HdspStop => {
                dispatcher.handle_request(Request {
                    id: 0,
                    params: Some(Params::RequestHdspStop(RequestHdspStop {})),
                }).await;
            }
            Command::ResetAlarm => {
                let (reply, rx) = tokio::sync::oneshot::channel();
                bridge_tx.send(BridgeCommand::ResetAlarm { reply })
                    .await.map_err(|e| e.to_string())?;
                rx.await.map_err(|e| e.to_string())?.map_err(|e| e)?;
            }
            Command::Calibrate => {
                // Use the spring-back peck-probe: it releases the servo at each
                // step to sense contact with no sustained thrust, so it can't
                // push through a soft/elastic target (unlike push-to-contact,
                // which only stops on a firm stall). Push-to-contact remains
                // available via the debug console.
                let (reply, rx) = tokio::sync::oneshot::channel();
                bridge_tx.send(BridgeCommand::PeckProbe { reply })
                    .await.map_err(|e| e.to_string())?;
                rx.await.map_err(|e| e.to_string())?.map_err(|e| e)?;
            }
            Command::DrillStart { feed_rate_mm_s } => {
                modes.drill.send(DrillControl::Start { feed_rate_mm_s })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::DrillPush { feed_rate_mm_s } => {
                modes.drill.send(DrillControl::Push { feed_rate_mm_s })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::DrillConfig { feed_rate_mm_s } => {
                modes.drill.send(DrillControl::SetFeedRate { feed_rate_mm_s })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::DrillStop => {
                modes.drill.send(DrillControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::RampStart { duration_s } => {
                modes.ramp.send(RampControl::Start { duration_s })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::RampNudge { delta } => {
                modes.ramp.send(RampControl::Nudge { delta })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::RampStop => {
                modes.ramp.send(RampControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::GameStart { kind } => {
                let k = crate::state::GameKind::parse(&kind)
                    .ok_or_else(|| format!("unknown game kind {kind:?}"))?;
                modes.game.send(GameControl::Start { kind: k })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::GameButton { down } => {
                modes.game.send(GameControl::Button { down })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::GameStop => {
                modes.game.send(GameControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::CycleStart => {
                modes.cycle.send(CycleControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::CycleButton { down } => {
                modes.cycle.send(CycleControl::Button { down })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::CycleStop => {
                modes.cycle.send(CycleControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::LearnStart => {
                modes.learn.send(LearnControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::LearnButton => {
                modes.learn.send(LearnControl::Button)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::LearnStop => {
                modes.learn.send(LearnControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::PulseStart { factor } => {
                modes.pulse.send(PulseControl::Start { factor })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::PulseSetFactor { factor } => {
                modes.pulse.send(PulseControl::SetFactor { factor })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::PulseStop => {
                modes.pulse.send(PulseControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::ImpaleStart { feed_rate_mm_s, retract_after_s } => {
                modes.impale.send(ImpaleControl::Start { feed_rate_mm_s, retract_after_s })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::ImpaleButton { down } => {
                modes.impale.send(ImpaleControl::Button { down })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::ImpaleConfig { retract_after_s } => {
                modes.impale.send(ImpaleControl::SetRetractAfter { retract_after_s })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::ImpaleStop => {
                modes.impale.send(ImpaleControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::CoyoteSetStrength { a, b } => {
                modes.coyote.send(crate::devices::CoyoteControl::SetStrength { a, b })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::CoyoteFollow { enable, scale } => {
                modes.coyote.send(crate::devices::CoyoteControl::Follow { enable, scale })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::CoyoteStop => {
                modes.coyote.send(crate::devices::CoyoteControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::HrConnect => {
                modes.sensors.send(crate::sensors::SensorControl::Connect)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::HrDisconnect => {
                modes.sensors.send(crate::sensors::SensorControl::Disconnect)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::SetMaxDepth { mm } => {
                bridge_tx.send(BridgeCommand::SetMaxDepth { mm })
                    .await.map_err(|e| e.to_string())?;
            }
            Command::PlumbStart => {
                modes.plumb.send(PlumbControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::PlumbStop => {
                modes.plumb.send(PlumbControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::SurgeStart => {
                modes.surge.send(SurgeControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::SurgeStop => {
                modes.surge.send(SurgeControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::TideStart => {
                modes.tide.send(TideControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::TideStop => {
                modes.tide.send(TideControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::EchoStart => {
                modes.echo.send(EchoControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::EchoStop => {
                modes.echo.send(EchoControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::TraceStart => {
                modes.trace.send(TraceControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::TraceStop => {
                modes.trace.send(TraceControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::TempoStart => {
                modes.tempo.send(TempoControl::Start)
                    .await.map_err(|e| e.to_string())?;
            }
            Command::TempoStop => {
                modes.tempo.send(TempoControl::Stop)
                    .await.map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    async fn send_ack(ack_tx: &mpsc::Sender<Vec<u8>>, seq: u32, ok: bool, error: Option<String>) {
        let ack = CommandAck { seq, ok, error };
        match serde_json::to_vec(&ack) {
            Ok(bytes) => { let _ = ack_tx.send(bytes).await; }
            Err(e) => warn!(error = %e, "SSCP ack serialise failed"),
        }
    }
}
