//! Hismith PiuPiu lube launcher — BLE-central actuator driver (spike).
//!
//! Protocol per the Buttplug protocol-docs issue documenting this device
//! (<https://github.com/buttplugio/docs.buttplug.io/issues/34>): BLE name
//! "Hismith Piupiu", a single GATT service, one write characteristic, and one
//! momentary "squirt" command — there's no intensity/duration parameter and no
//! notify/battery characteristic. "Holding" a shot is purely a matter of the
//! host repeating the write, so this driver resends the command every 100 ms
//! for as long as [`PiuPiuControl::Squirt`] is active — same cadence and
//! same "device-side no fail-safe, so the host must stop sending" shape as
//! the Coyote driver (`src/devices/coyote.rs`).
//!
//! This is a SPIKE: the `bluer` connect/write path (Linux-only) has NOT been
//! validated on hardware.

use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::config::PiuPiu;
use crate::state::AppState;

/// Control messages to the PiuPiu driver. The task is always running but idle
/// until [`Connect`](PiuPiuControl::Connect); the UI (or configured
/// autoconnect) triggers pairing on demand, mirroring
/// [`CoyoteControl`](crate::devices::CoyoteControl) /
/// [`SensorControl`](crate::sensors::SensorControl).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiuPiuControl {
    /// Start scanning for and maintaining a PiuPiu connection.
    Connect,
    /// Drop any current connection/scan and go idle.
    Disconnect,
    /// Hold (`true`) or release (`false`) the squirt trigger. While held, the
    /// squirt command is resent every 100 ms.
    Squirt { active: bool },
}

// ───────────────────────────── protocol (pure) ──────────────────────────────

/// Momentary "squirt" command — the device has no other commands.
pub const SQUIRT_CMD: [u8; 4] = [0xCC, 0x0B, 0x01, 0x0C];

// ───────────────────────────── driver (Linux) ───────────────────────────────

#[cfg(target_os = "linux")]
pub use imp::run;

#[cfg(not(target_os = "linux"))]
pub async fn run(
    _state: Arc<RwLock<AppState>>,
    mut ctrl_rx: mpsc::Receiver<PiuPiuControl>,
    _cfg: PiuPiu,
    _adapter: String,
) {
    tracing::warn!("PiuPiu driver requires Linux/BlueZ — skipping on this platform.");
    // Drain control messages so senders never block on this platform.
    while ctrl_rx.recv().await.is_some() {}
}

#[cfg(target_os = "linux")]
mod imp {
    use std::time::Duration;

    use bluer::{AdapterEvent, Device, Session, Uuid};
    use futures::StreamExt;
    use tokio::time::{interval, sleep, MissedTickBehavior};
    use tracing::{info, warn};

    use super::*;

    const fn bt(short: u16) -> Uuid {
        Uuid::from_u128(0x0000_0000_0000_1000_8000_0080_5f9b_34fb | ((short as u128) << 96))
    }
    const SVC_LUBE: Uuid = bt(0xffe5);
    const CH_TX: Uuid = bt(0xffe9);
    /// Advertised name substring for the Hismith PiuPiu.
    const DEFAULT_NAME: &str = "Piupiu";
    /// Resend cadence while the squirt trigger is held.
    const FRAME: Duration = Duration::from_millis(100);

    /// Control-driven supervisor. Idle until a [`PiuPiuControl::Connect`]
    /// arrives, then keep a PiuPiu connected (retrying on any error/drop) until
    /// [`PiuPiuControl::Disconnect`]. Mirrors `coyote::run`/`sensors::run`.
    pub async fn run(
        state: Arc<RwLock<AppState>>,
        mut ctrl_rx: mpsc::Receiver<PiuPiuControl>,
        cfg: PiuPiu,
        adapter: String,
    ) {
        info!("piupiu driver running (idle)");
        loop {
            // ── Idle: wait for a Connect. ──
            match ctrl_rx.recv().await {
                None => return, // channel closed → shutting down
                Some(PiuPiuControl::Connect) => {}
                Some(_) => continue, // already idle: nothing to disconnect/squirt
            }
            info!("piupiu: connect requested; scanning");

            // ── Active: maintain the connection, retrying, until disconnected. ──
            loop {
                let disconnect_requested = match session(&state, &mut ctrl_rx, &cfg, &adapter).await
                {
                    Ok(disconnect_requested) => disconnect_requested,
                    Err(e) => {
                        warn!(error = %e, "piupiu session ended; retrying in 5s");
                        false
                    }
                };
                {
                    let mut st = state.write().await;
                    st.piupiu.connected = false;
                    st.piupiu.active = false;
                }
                if disconnect_requested {
                    break;
                }
                sleep(Duration::from_secs(5)).await;
            }

            // ── Back to idle: clear all piupiu state. ──
            state.write().await.piupiu = Default::default();
            info!("piupiu: disconnected (idle)");
        }
    }

    /// Runs one connection attempt. Returns `Ok(true)` if it ended because a
    /// [`PiuPiuControl::Disconnect`] arrived (caller should go idle, no
    /// retry), `Ok(false)`/`Err` if it ended for any other reason (caller
    /// retries after a backoff).
    async fn session(
        state: &Arc<RwLock<AppState>>,
        ctrl_rx: &mut mpsc::Receiver<PiuPiuControl>,
        cfg: &PiuPiu,
        adapter_name: &str,
    ) -> bluer::Result<bool> {
        let session = Session::new().await?;
        let adapter = match session.adapter(adapter_name) {
            Ok(a) => a,
            Err(_) => session.default_adapter().await?,
        };
        adapter.set_powered(true).await?;

        let name = if cfg.name.is_empty() {
            DEFAULT_NAME
        } else {
            &cfg.name
        };
        info!(%name, "scanning for PiuPiu…");
        let device = find_device(&adapter, name).await?;
        if !device.is_connected().await? {
            device.connect().await?;
        }

        let write = tx_characteristic(&device).await?;
        state.write().await.piupiu.connected = true;
        info!("piupiu connected");

        let mut tick = interval(FRAME);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut active = false;

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => return Ok(false),
                    Some(PiuPiuControl::Connect) => {} // already connected; no-op
                    Some(PiuPiuControl::Disconnect) => {
                        state.write().await.piupiu.active = false;
                        return Ok(true);
                    }
                    Some(PiuPiuControl::Squirt { active: a }) => {
                        active = a;
                        state.write().await.piupiu.active = a;
                    }
                },
                _ = tick.tick() => {
                    if active {
                        if let Err(e) = write.write(&SQUIRT_CMD).await {
                            warn!(error = %e, "piupiu squirt write failed");
                            return Ok(false); // drop session → reconnect
                        }
                    }
                }
            }
        }
    }

    async fn find_device(adapter: &bluer::Adapter, name: &str) -> bluer::Result<Device> {
        let mut events = adapter.discover_devices().await?;
        while let Some(ev) = events.next().await {
            if let AdapterEvent::DeviceAdded(addr) = ev {
                let device = adapter.device(addr)?;
                let matches = device
                    .name()
                    .await?
                    .map(|n| n.contains(name))
                    .unwrap_or(false)
                    || device
                        .uuids()
                        .await?
                        .map(|u| u.contains(&SVC_LUBE))
                        .unwrap_or(false);
                if matches {
                    return Ok(device);
                }
            }
        }
        Err(bluer::Error {
            kind: bluer::ErrorKind::NotFound,
            message: "discovery ended".into(),
        })
    }

    type Chr = bluer::gatt::remote::Characteristic;
    async fn tx_characteristic(device: &Device) -> bluer::Result<Chr> {
        for service in device.services().await? {
            for ch in service.characteristics().await? {
                if ch.uuid().await? == CH_TX {
                    return Ok(ch);
                }
            }
        }
        Err(bluer::Error {
            kind: bluer::ErrorKind::NotFound,
            message: "PiuPiu command characteristic not found".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squirt_command_is_fixed_bytes() {
        assert_eq!(SQUIRT_CMD, [0xCC, 0x0B, 0x01, 0x0C]);
    }
}
