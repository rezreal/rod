//! BLE transport (Handy FW4 GATT, SPEC §4.1).
//!
//! Advertises `ohd_hw<MODEL>_<UID>` with the FW4 service and exposes the TX
//! (notify) / RX (write) characteristics. Client writes carry `RpcMessage`
//! REQUEST/REQUESTS frames; the device answers via TX notify with
//! RESPONSE/NOTIFICATION frames. The decode→dispatch→encode work is shared with
//! the cloud transport via [`crate::transport::serve_frames`].
//!
//! The real implementation uses `bluer` (BlueZ over D-Bus) and therefore only
//! compiles on Linux. On other platforms a no-op stub is built so the rest of
//! the bridge still builds and tests (the actuator/Modbus path is unaffected).

use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use crate::config::Config;
use crate::modes::ModeControls;
use crate::rpc::dispatch::Dispatcher;
use crate::rpc::RpcMessage;
use crate::state::AppState;

/// FW4 service + characteristic UUIDs (SPEC §4.1).
pub const SERVICE_UUID: &str = "77834d26-40f7-11ee-be56-0242ac120002";
// Characteristic roles per buttplug's hardware-tested `thehandy-v3` config +
// protocol impl: the client WRITES commands on `…5032` and SUBSCRIBES for
// notifications on `…5410`. (buttplug's config names these "tx"/"rx" from the
// *host's* perspective — opposite of our device-side TX/RX labels — so the
// UUIDs are assigned the way that looks swapped but is correct.)
pub const TX_UUID: &str = "77835410-40f7-11ee-be56-0242ac120002"; // device→client, Notify
pub const RX_UUID: &str = "77835032-40f7-11ee-be56-0242ac120002"; // client→device, Write

/// Vendor "bridge-control" service: out-of-band commands that are **not** part
/// of the Handy FW4 protocol (alarm reset, contact calibration). Carries a
/// tiny line-based ASCII protocol rather than protobuf. See SPEC §4.3.
pub const BRIDGE_SERVICE_UUID: &str = "6f1d0b00-9a2e-4b8c-9c11-0a1b2c3d4e5f";
pub const BRIDGE_CMD_UUID: &str = "6f1d0b01-9a2e-4b8c-9c11-0a1b2c3d4e5f"; // client→device, Write
pub const BRIDGE_RESP_UUID: &str = "6f1d0b02-9a2e-4b8c-9c11-0a1b2c3d4e5f"; // device→client, Notify

/// Standard Battery Service (0x180F) + Battery Level (0x2A19). The official app
/// expects a battery service to exist; we expose our own **unencrypted**,
/// read-only one (BlueZ's auto battery plugin is disabled on the Pi because its
/// version required authentication, which forced a pairing the app rejects).
pub const BATTERY_SERVICE_UUID: &str = "0000180f-0000-1000-8000-00805f9b34fb";
pub const BATTERY_LEVEL_UUID: &str = "00002a19-0000-1000-8000-00805f9b34fb";
/// Reported battery level (%). The actuator is mains-powered, so this is a
/// constant — it exists only to satisfy clients that read a battery service.
#[cfg(target_os = "linux")]
const BATTERY_LEVEL_PCT: u8 = 100;

/// Advertising name, e.g. `OHD_hw3_a1b2c3d4e5f6`.
///
/// The prefix is **uppercase `OHD_`** — this is what genuine Handy FW4 devices
/// advertise, and the official app validates the name case-sensitively (verified
/// against buttplug's hardware-tested `thehandy-v3` device config). A lowercase
/// `ohd_` is discovered but rejected on connect.
pub fn advertising_name(hw_model: u8, uid: &str) -> String {
    format!("OHD_hw{hw_model}_{uid}")
}

#[cfg(target_os = "linux")]
pub use imp::run;

#[cfg(not(target_os = "linux"))]
pub async fn run(
    _cfg: &Config,
    _state: Arc<RwLock<AppState>>,
    _dispatcher: Dispatcher,
    _notif_tx: broadcast::Sender<RpcMessage>,
    _bridge_tx: tokio::sync::mpsc::Sender<crate::state::BridgeCommand>,
    _modes: ModeControls,
) {
    tracing::warn!(
        "BLE transport requires Linux/BlueZ; not available on this platform — skipping. \
         (The Modbus/actuator path is unaffected.)"
    );
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::str::FromStr;

    use bluer::adv::Advertisement;
    use bluer::gatt::local::{
        characteristic_control, Application, Characteristic, CharacteristicControl,
        CharacteristicControlEvent, CharacteristicNotify, CharacteristicNotifyMethod,
        CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, Service,
    };
    use bluer::gatt::CharacteristicWriter;
    use bluer::Uuid;
    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{mpsc, oneshot};
    use tracing::{error, info, warn};

    use crate::sscp::service::{make_sscp_characteristics, spawn_sscp_tasks};
    use crate::state::BridgeCommand;
    use crate::transport::serve_frames;

    /// Largest BLE payload we read/write per frame (capability `ble_mtu`).
    const MTU: usize = 512;

    pub async fn run(
        cfg: &Config,
        state: Arc<RwLock<AppState>>,
        dispatcher: Dispatcher,
        notif_tx: broadcast::Sender<RpcMessage>,
        bridge_tx: mpsc::Sender<BridgeCommand>,
        modes: ModeControls,
    ) {
        if let Err(e) = run_inner(cfg, state, dispatcher, notif_tx, bridge_tx, modes).await {
            // `{:#}` prints the full anyhow context chain (which operation, then
            // the underlying BlueZ error) instead of just the outermost message.
            let chain = format!("{e:#}");
            error!(error = %chain, "BLE transport failed");
            error!(
                "If this failed at GATT registration or advertising, BlueZ experimental \
                 features are most likely OFF (required by bluer's GATT server + advertising). \
                 Enable them: add `Experimental = true` under `[General]` in \
                 /etc/bluetooth/main.conf, then `sudo systemctl restart bluetooth`. \
                 See README §BLE setup."
            );
        }
    }

    async fn run_inner(
        cfg: &Config,
        state: Arc<RwLock<AppState>>,
        dispatcher: Dispatcher,
        notif_tx: broadcast::Sender<RpcMessage>,
        bridge_tx: mpsc::Sender<BridgeCommand>,
        modes: ModeControls,
    ) -> anyhow::Result<()> {
        use anyhow::Context as _;

        let session = bluer::Session::new()
            .await
            .context("connecting to BlueZ D-Bus session")?;
        let adapter = match session.adapter(&cfg.ble.adapter) {
            Ok(a) => a,
            Err(e) => {
                warn!(adapter = %cfg.ble.adapter, error = %e, "named adapter unavailable; using default");
                session
                    .default_adapter()
                    .await
                    .context("acquiring default Bluetooth adapter")?
            }
        };
        adapter
            .set_powered(true)
            .await
            .context("powering on the Bluetooth adapter")?;
        info!(adapter = %adapter.name(), "BLE adapter powered");

        let service_uuid = Uuid::from_str(SERVICE_UUID)?;
        let tx_uuid = Uuid::from_str(TX_UUID)?;
        let rx_uuid = Uuid::from_str(RX_UUID)?;
        let bridge_service_uuid = Uuid::from_str(BRIDGE_SERVICE_UUID)?;
        let bridge_cmd_uuid = Uuid::from_str(BRIDGE_CMD_UUID)?;
        let bridge_resp_uuid = Uuid::from_str(BRIDGE_RESP_UUID)?;
        let battery_service_uuid = Uuid::from_str(BATTERY_SERVICE_UUID)?;
        let battery_level_uuid = Uuid::from_str(BATTERY_LEVEL_UUID)?;

        let (tx_control, tx_handle) = characteristic_control();
        let (rx_control, rx_handle) = characteristic_control();
        let (bridge_cmd_control, bridge_cmd_handle) = characteristic_control();
        let (bridge_resp_control, bridge_resp_handle) = characteristic_control();

        // Build SSCP characteristics (appended to the Handy FW4 service so no
        // new service registration or advertisement change is needed).
        let (sscp_chars, sscp_controls) = make_sscp_characteristics(cfg, modes)?;

        // Build the Handy FW4 service with SSCP characteristics appended.
        // The Handy app ignores unknown UUIDs; the Rod app discovers them
        // by reading the primary service's full characteristic list.
        let mut handy_chars = vec![
            // TX: device → client, Notify.
            Characteristic {
                uuid: tx_uuid,
                notify: Some(CharacteristicNotify {
                    notify: true,
                    method: CharacteristicNotifyMethod::Io,
                    ..Default::default()
                }),
                control_handle: tx_handle,
                ..Default::default()
            },
            // RX: client → device, Write (with and without response).
            Characteristic {
                uuid: rx_uuid,
                write: Some(CharacteristicWrite {
                    write: true,
                    write_without_response: true,
                    method: CharacteristicWriteMethod::Io,
                    ..Default::default()
                }),
                control_handle: rx_handle,
                ..Default::default()
            },
        ];
        handy_chars.extend(sscp_chars);

        let app = Application {
            services: vec![
                Service {
                    uuid: service_uuid,
                    primary: true,
                    characteristics: handy_chars,
                    ..Default::default()
                },
                // Vendor bridge-control service (ASCII command/response).
                Service {
                    uuid: bridge_service_uuid,
                    primary: true,
                    characteristics: vec![
                        // CMD: client → device, Write.
                        Characteristic {
                            uuid: bridge_cmd_uuid,
                            write: Some(CharacteristicWrite {
                                write: true,
                                write_without_response: true,
                                method: CharacteristicWriteMethod::Io,
                                ..Default::default()
                            }),
                            control_handle: bridge_cmd_handle,
                            ..Default::default()
                        },
                        // RESP: device → client, Notify.
                        Characteristic {
                            uuid: bridge_resp_uuid,
                            notify: Some(CharacteristicNotify {
                                notify: true,
                                method: CharacteristicNotifyMethod::Io,
                                ..Default::default()
                            }),
                            control_handle: bridge_resp_handle,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                // Standard Battery Service (0x180F), unencrypted read-only.
                Service {
                    uuid: battery_service_uuid,
                    primary: true,
                    characteristics: vec![Characteristic {
                        uuid: battery_level_uuid,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(|_req| {
                                Box::pin(async move { Ok(vec![BATTERY_LEVEL_PCT]) })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let _app_handle = adapter
            .serve_gatt_application(app)
            .await
            .context("registering the GATT application (needs BlueZ experimental features)")?;

        let name = advertising_name(cfg.ble.hw_model, &cfg.ble.uid);

        if let Err(e) = adapter.set_alias(name.clone()).await {
            warn!(error = %e, "could not set adapter alias; GAP device name may not match");
        }

        // Advertisement is unchanged from the original: Handy FW4 UUID only.
        // The SSCP characteristics are discoverable by enumerating the service.
        let adv = Advertisement {
            service_uuids: vec![service_uuid].into_iter().collect(),
            discoverable: Some(true),
            local_name: Some(name.clone()),
            ..Default::default()
        };
        let _adv_handle = adapter
            .advertise(adv)
            .await
            .context("starting BLE advertising (needs BlueZ experimental features)")?;
        info!(%name, "BLE advertising started");

        // ── Handy FW4 I/O ──────────────────────────────────────────────────
        let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(64);
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(64);
        tokio::spawn(tx_pump(tx_control, out_rx));
        tokio::spawn(rx_pump(rx_control, in_tx));
        tokio::spawn(bridge_pump(bridge_cmd_control, bridge_resp_control, bridge_tx.clone()));

        // ── SSCP I/O (within Handy service) ───────────────────────────────
        spawn_sscp_tasks(state, dispatcher.clone(), bridge_tx, cfg.clone(), sscp_controls);

        serve_frames("ble", dispatcher, in_rx, out_tx, notif_tx.subscribe()).await;
        Ok(())
    }

    /// Forward outbound frames to whichever central is subscribed for notify.
    async fn tx_pump(mut tx_control: CharacteristicControl, mut out_rx: mpsc::Receiver<Vec<u8>>) {
        let mut writer: Option<CharacteristicWriter> = None;
        loop {
            tokio::select! {
                evt = tx_control.next() => match evt {
                    Some(CharacteristicControlEvent::Notify(w)) => {
                        info!("BLE central subscribed to TX notify");
                        writer = Some(w);
                    }
                    Some(_) => {}
                    None => break,
                },
                frame = out_rx.recv() => {
                    let Some(frame) = frame else { break };
                    if let Some(w) = writer.as_mut() {
                        if frame.len() > MTU {
                            warn!(len = frame.len(), "outbound frame exceeds MTU; dropping");
                            continue;
                        }
                        if let Err(e) = w.write_all(&frame).await {
                            warn!(error = %e, "TX notify write failed; dropping subscriber");
                            writer = None;
                        }
                    }
                }
            }
        }
    }

    /// Read inbound RX writes and forward each frame to serve_frames.
    async fn rx_pump(mut rx_control: CharacteristicControl, in_tx: mpsc::Sender<Vec<u8>>) {
        while let Some(evt) = rx_control.next().await {
            if let CharacteristicControlEvent::Write(req) = evt {
                match req.accept() {
                    Ok(mut reader) => {
                        let in_tx = in_tx.clone();
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; MTU];
                            loop {
                                match reader.read(&mut buf).await {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if in_tx.send(buf[..n].to_vec()).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "RX read failed");
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => warn!(error = %e, "failed to accept RX write"),
                }
            }
        }
    }

    /// Drive the vendor bridge-control service: read an ASCII command from the
    /// CMD characteristic, run it via [`BridgeCommand`], and notify the textual
    /// result on the RESP characteristic.
    ///
    /// A command blocks this pump until it completes (calibration can take a
    /// while); that is intentional — the control channel is strictly serial.
    /// Clients should subscribe to RESP notify *before* issuing a command.
    async fn bridge_pump(
        mut cmd_control: CharacteristicControl,
        mut resp_control: CharacteristicControl,
        bridge_tx: mpsc::Sender<BridgeCommand>,
    ) {
        let mut resp_writer: Option<CharacteristicWriter> = None;
        loop {
            tokio::select! {
                evt = resp_control.next() => match evt {
                    Some(CharacteristicControlEvent::Notify(w)) => {
                        info!("BLE central subscribed to bridge RESP notify");
                        resp_writer = Some(w);
                    }
                    Some(_) => {}
                    None => break,
                },
                evt = cmd_control.next() => match evt {
                    Some(CharacteristicControlEvent::Write(req)) => {
                        let mut reader = match req.accept() {
                            Ok(r) => r,
                            Err(e) => { warn!(error = %e, "failed to accept bridge write"); continue; }
                        };
                        let mut buf = vec![0u8; MTU];
                        let line = match reader.read(&mut buf).await {
                            Ok(0) | Err(_) => continue,
                            Ok(n) => String::from_utf8_lossy(&buf[..n]).trim().to_string(),
                        };
                        info!(%line, "bridge command");
                        let resp = handle_bridge_line(&line, &bridge_tx).await;
                        if let Some(w) = resp_writer.as_mut() {
                            if let Err(e) = w.write_all(resp.as_bytes()).await {
                                warn!(error = %e, "bridge RESP write failed; dropping subscriber");
                                resp_writer = None;
                            }
                        } else {
                            info!(%resp, "bridge response (no notify subscriber)");
                        }
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    /// Map one ASCII command line to a [`BridgeCommand`] and await its reply,
    /// returning the textual response to notify back.
    async fn handle_bridge_line(line: &str, bridge_tx: &mpsc::Sender<BridgeCommand>) -> String {
        match line.trim() {
            "reset-alarm" => {
                let (reply, rx) = oneshot::channel();
                if bridge_tx
                    .send(BridgeCommand::ResetAlarm { reply })
                    .await
                    .is_err()
                {
                    return "err bridge offline".to_string();
                }
                match rx.await {
                    Ok(Ok(())) => "ok reset-alarm".to_string(),
                    Ok(Err(e)) => format!("err {e}"),
                    Err(_) => "err no reply".to_string(),
                }
            }
            "calibrate" => {
                let (reply, rx) = oneshot::channel();
                if bridge_tx
                    .send(BridgeCommand::Calibrate { reply })
                    .await
                    .is_err()
                {
                    return "err bridge offline".to_string();
                }
                match rx.await {
                    Ok(Ok(pos)) => format!("ok contact {pos:.2}"),
                    Ok(Err(e)) => format!("err {e}"),
                    Err(_) => "err no reply".to_string(),
                }
            }
            other => format!("err unknown command {other:?}"),
        }
    }
}
