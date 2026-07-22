//! `rod` binary: load config, bring up the actuator, and run the
//! transports. Task graph per SPEC §8.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info};

use rod::config::Config;
use rod::modbus::driver::{ModbusDriver, SerialBus};
use rod::modes::cycle::CycleTask;
use rod::modes::drill::DrillTask;
use rod::modes::echo::EchoTask;
use rod::modes::games::GameTask;
use rod::modes::hamp::HampTask;
use rod::modes::hsp::HspTask;
use rod::modes::impale::ImpaleTask;
use rod::modes::learn::LearnTask;
use rod::modes::plumb::PlumbTask;
use rod::modes::pulse::PulseTask;
use rod::modes::ramp::RampTask;
use rod::modes::surge::SurgeTask;
use rod::modes::tempo::TempoTask;
use rod::modes::tide::TideTask;
use rod::modes::trace::TraceTask;
use rod::modes::ModeControls;
use rod::rpc::dispatch::Dispatcher;
use rod::shaper::Shaper;
use rod::state::AppState;
use rod::telemetry;
use rod::transport::{ble, cloud};

/// Channel depths. Actuator commands stay shallow so movement stays current.
const CMD_CHANNEL: usize = 32;
const NOTIF_CHANNEL: usize = 256;
const CTRL_CHANNEL: usize = 32;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _telemetry = telemetry::init()?;

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));
    let mut cfg = Config::load(&config_path)
        .with_context(|| format!("loading config from {}", config_path.display()))?;
    info!(path = %config_path.display(), variant = %cfg.actuator.variant, "config loaded");

    // BLE UID: use the configured value, else load/generate+persist (SPEC §9.1).
    cfg.ble.uid = resolve_uid(&cfg.ble.uid)?;
    info!(uid = %cfg.ble.uid, hw_model = cfg.ble.hw_model, "device identity");

    // Shared state + buses.
    let boot_session_id: u32 = rand::random();
    let mut app_state = AppState::new(cfg.ble.uid.clone(), boot_session_id);
    if !cfg.ble.connection_key.is_empty() {
        app_state.connection_key = Some(cfg.ble.connection_key.clone());
    }
    // Global max-depth ceiling: persisted value (clamped to stroke) or full stroke.
    let stroke_mm = cfg.stroke_mm();
    app_state.max_depth_mm = rod::modbus::driver::load_max_depth()
        .map(|v| v.min(stroke_mm))
        .unwrap_or(stroke_mm);
    let state = Arc::new(RwLock::new(app_state));
    let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL);
    // Modes/dispatcher send here; the motion shaper forwards to the driver,
    // expanding softened strokes into ramped sub-moves (see src/shaper.rs).
    let (shape_tx, shape_rx) = mpsc::channel(CMD_CHANNEL);
    let (bridge_tx, bridge_rx) = mpsc::channel(CTRL_CHANNEL);
    let (notif_tx, _notif_keepalive) = broadcast::channel(NOTIF_CHANNEL);
    let (hamp_tx, hamp_rx) = mpsc::channel(CTRL_CHANNEL);
    let (hsp_tx, hsp_rx) = mpsc::channel(CTRL_CHANNEL);
    let (drill_tx, drill_rx) = mpsc::channel(CTRL_CHANNEL);
    let (ramp_tx, ramp_rx) = mpsc::channel(CTRL_CHANNEL);
    let (game_tx, game_rx) = mpsc::channel(CTRL_CHANNEL);
    let (cycle_tx, cycle_rx) = mpsc::channel(CTRL_CHANNEL);
    let (learn_tx, learn_rx) = mpsc::channel(CTRL_CHANNEL);
    let (pulse_tx, pulse_rx) = mpsc::channel(CTRL_CHANNEL);
    let (impale_tx, impale_rx) = mpsc::channel(CTRL_CHANNEL);
    let (plumb_tx, plumb_rx) = mpsc::channel(CTRL_CHANNEL);
    let (surge_tx, surge_rx) = mpsc::channel(CTRL_CHANNEL);
    let (tide_tx, tide_rx) = mpsc::channel(CTRL_CHANNEL);
    let (echo_tx, echo_rx) = mpsc::channel(CTRL_CHANNEL);
    let (trace_tx, trace_rx) = mpsc::channel(CTRL_CHANNEL);
    let (tempo_tx, tempo_rx) = mpsc::channel(CTRL_CHANNEL);
    let (coyote_tx, coyote_rx) = mpsc::channel(CTRL_CHANNEL);
    let (sensor_tx, sensor_rx) = mpsc::channel(CTRL_CHANNEL);

    // ── Modbus driver ──
    // Start *disconnected*: the driver opens the port, runs the §9 startup, and
    // owns it from inside its run loop — and re-establishes the link on hot-plug
    // (port-independent scan). This lets the bridge boot and advertise even with
    // the actuator unplugged; `AppState.actuator_connected` reflects the link.
    let bus = SerialBus::disconnected(
        &cfg.actuator.serial_device,
        cfg.actuator.baud_rate,
        cfg.actuator.modbus_slave,
    );
    let driver = ModbusDriver::new(bus, state.clone(), notif_tx.clone(), &cfg);
    tokio::spawn(driver.run(cmd_rx, bridge_rx));

    // ── Motion shaper (software jerk-limiting; transparent unless a stroke
    //    opts in via soften) ──
    tokio::spawn(Shaper::new(cmd_tx.clone(), &cfg).run(shape_rx));

    // ── Mode tasks (send through the shaper, not straight to the driver) ──
    tokio::spawn(HampTask::new(state.clone(), shape_tx.clone(), &cfg).run(hamp_rx));
    tokio::spawn(HspTask::new(state.clone(), shape_tx.clone(), notif_tx.clone(), &cfg).run(hsp_rx));
    tokio::spawn(DrillTask::new(state.clone(), shape_tx.clone(), &cfg).run(drill_rx));
    tokio::spawn(RampTask::new(state.clone(), shape_tx.clone(), &cfg).run(ramp_rx));
    tokio::spawn(GameTask::new(state.clone(), shape_tx.clone(), &cfg).run(game_rx));
    tokio::spawn(CycleTask::new(state.clone(), shape_tx.clone(), &cfg).run(cycle_rx));
    tokio::spawn(LearnTask::new(state.clone(), shape_tx.clone(), &cfg).run(learn_rx));
    tokio::spawn(PulseTask::new(state.clone(), shape_tx.clone(), &cfg).run(pulse_rx));
    tokio::spawn(ImpaleTask::new(state.clone(), shape_tx.clone(), &cfg).run(impale_rx));
    tokio::spawn(PlumbTask::new(state.clone(), shape_tx.clone(), &cfg).run(plumb_rx));
    tokio::spawn(SurgeTask::new(state.clone(), shape_tx.clone(), &cfg).run(surge_rx));
    tokio::spawn(TideTask::new(state.clone(), shape_tx.clone(), &cfg).run(tide_rx));
    tokio::spawn(EchoTask::new(state.clone(), shape_tx.clone(), &cfg).run(echo_rx));
    tokio::spawn(TraceTask::new(state.clone(), shape_tx.clone(), &cfg).run(trace_rx));
    tokio::spawn(TempoTask::new(state.clone(), shape_tx.clone(), &cfg).run(tempo_rx));

    // ── Biosensors (BLE central) ──
    // Always spawn the task so the UI can pair on demand; if configured to
    // auto-connect, kick it off now (preserving the old boot behavior).
    {
        let s = state.clone();
        let adapter = cfg.ble.adapter.clone();
        let name = cfg.sensors.heart_rate.name.clone();
        tokio::spawn(async move { rod::sensors::run(s, sensor_rx, adapter, name).await });
        if cfg.sensors.heart_rate.enable {
            let _ = sensor_tx.send(rod::sensors::SensorControl::Connect).await;
        }
    }

    // ── External BLE actuators: DG-LAB Coyote (runs only if enabled) ──
    if cfg.devices.coyote.enable {
        let s = state.clone();
        let adapter = cfg.ble.adapter.clone();
        let ccfg = cfg.devices.coyote.clone();
        tokio::spawn(async move { rod::devices::coyote::run(s, coyote_rx, ccfg, adapter).await });
    }

    // ── Dispatcher (shared by all transports) ──
    let modes = ModeControls {
        drill: drill_tx.clone(),
        ramp: ramp_tx.clone(),
        game: game_tx.clone(),
        cycle: cycle_tx.clone(),
        learn: learn_tx.clone(),
        pulse: pulse_tx.clone(),
        impale: impale_tx.clone(),
        plumb: plumb_tx.clone(),
        surge: surge_tx.clone(),
        tide: tide_tx.clone(),
        echo: echo_tx.clone(),
        trace: trace_tx.clone(),
        tempo: tempo_tx.clone(),
        coyote: coyote_tx.clone(),
        sensors: sensor_tx.clone(),
    };
    let dispatcher = Dispatcher::new(
        state.clone(),
        shape_tx.clone(),
        notif_tx.clone(),
        hamp_tx,
        hsp_tx,
        modes.clone(),
        &cfg,
    );

    // ── Hardware hand-switch watcher: drives the active program as its button ──
    tokio::spawn(rod::modes::handswitch::run(
        state.clone(),
        dispatcher.clone(),
    ));

    // ── Transports ──
    if cfg.transports.enable_ble {
        let d = dispatcher.clone();
        let n = notif_tx.clone();
        let c = cfg.clone();
        let b = bridge_tx.clone();
        let s = state.clone();
        let m = modes.clone();
        tokio::spawn(async move { ble::run(&c, s, d, n, b, m).await });
    }
    if cfg.transports.enable_cloud {
        let d = dispatcher.clone();
        let n = notif_tx.subscribe();
        let c = cfg.clone();
        tokio::spawn(async move { cloud::run(&c, d, n).await });
    }

    // ── Local raw-Modbus debug console (loopback TCP; off by default) ──
    if cfg.debug.enable {
        let listen = cfg.debug.listen.clone();
        let b = bridge_tx.clone();
        tokio::spawn(async move { rod::debug::run(&listen, b).await });
    }

    info!("rod running; press Ctrl-C to stop");
    tokio::signal::ctrl_c().await.ok();
    info!("shutting down");
    // On shutdown, decel-stop any active motion.
    dispatcher.stop_everything().await;
    Ok(())
}

/// Resolve the BLE UID: configured value wins; otherwise read a persisted
/// `device-uid` file next to the binary, generating + persisting a fresh
/// 12-hex-char id on first boot.
fn resolve_uid(configured: &str) -> anyhow::Result<String> {
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let path = PathBuf::from("device-uid");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let bytes: [u8; 6] = rand::random();
    let uid = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    if let Err(e) = std::fs::write(&path, &uid) {
        error!(error = %e, "failed to persist generated UID; continuing with in-memory value");
    } else {
        info!(uid = %uid, "generated and persisted new device UID");
    }
    Ok(uid)
}
