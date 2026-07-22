//! BLE central — connect to biosensors while the bridge stays a BLE peripheral.
//!
//! Bluetooth LE is dual-role: the same adapter that serves our GATT server and
//! advertises to the web app (peripheral, see `transport/ble.rs`) can also scan
//! for and subscribe to other devices (central). This module runs that central
//! side on its own `bluer` session — currently a standard **Heart Rate Service**
//! (`0x180D`) sensor, whose BPM is written into [`AppState::sensors`] for the
//! Pulse program and the web UI to read.
//!
//! Like the BLE transport it only compiles on Linux (BlueZ/D-Bus); elsewhere a
//! no-op stub keeps the rest of the bridge building and testing.

use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::state::AppState;

/// Runtime control for the heart-rate sensor task. The UI triggers pairing on
/// demand (mirrors the [`CoyoteControl`](crate::devices::CoyoteControl) pattern)
/// instead of the sensor only connecting when enabled at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorControl {
    /// Start scanning for and maintaining a heart-rate sensor connection.
    Connect,
    /// Drop any current connection/scan and go idle.
    Disconnect,
}

/// Parse a GATT Heart Rate Measurement (characteristic `0x2A37`) into BPM.
/// Byte 0 is the flags field; bit 0 selects the value width (0 = u8, 1 = u16 LE).
pub fn parse_hr_measurement(data: &[u8]) -> Option<u16> {
    let flags = *data.first()?;
    if flags & 0x01 == 0 {
        Some(*data.get(1)? as u16)
    } else {
        let lo = *data.get(1)? as u16;
        let hi = *data.get(2)? as u16;
        Some(lo | (hi << 8))
    }
}

#[cfg(target_os = "linux")]
pub use imp::run;

#[cfg(not(target_os = "linux"))]
pub async fn run(
    _state: Arc<RwLock<AppState>>,
    mut ctrl_rx: mpsc::Receiver<SensorControl>,
    _adapter: String,
    _name_filter: String,
) {
    tracing::warn!(
        "heart-rate sensor (BLE central) requires Linux/BlueZ — skipping on this platform."
    );
    // Drain control messages so senders never block on this platform.
    while ctrl_rx.recv().await.is_some() {}
}

#[cfg(target_os = "linux")]
mod imp {
    use std::time::Duration;

    use bluer::{AdapterEvent, Device, Session, Uuid};
    use futures::StreamExt;
    use tokio::time::sleep;
    use tracing::{info, warn};

    use super::*;

    /// 16-bit assigned number → full 128-bit Bluetooth base UUID.
    const fn bt_uuid(short: u16) -> Uuid {
        Uuid::from_u128(0x0000_0000_0000_1000_8000_00805f9b34fb | ((short as u128) << 96))
    }
    const HR_SERVICE: Uuid = bt_uuid(0x180D);
    const HR_MEASUREMENT: Uuid = bt_uuid(0x2A37);

    /// Control-driven supervisor. Idle until a [`SensorControl::Connect`] arrives,
    /// then keep a heart-rate sensor connected (retrying on any error/disconnect)
    /// until [`SensorControl::Disconnect`]. A new command cancels in-flight work.
    pub async fn run(
        state: Arc<RwLock<AppState>>,
        mut ctrl_rx: mpsc::Receiver<SensorControl>,
        adapter: String,
        name_filter: String,
    ) {
        info!("heart-rate sensor task running (idle)");
        loop {
            // ── Idle: wait for a Connect. ──
            match ctrl_rx.recv().await {
                None => return, // channel closed → shutting down
                Some(SensorControl::Connect) => {}
                Some(SensorControl::Disconnect) => continue, // already idle
            }
            set_scanning(&state, true).await;
            info!("heart-rate: connect requested; scanning");

            // ── Active: maintain the connection, retrying, until told to stop. ──
            loop {
                // One attempt + a backoff, as a cancellable unit. If a control
                // message arrives first, the attempt future is dropped (cancelled).
                let attempt = async {
                    if let Err(e) = session_once(&state, &adapter, &name_filter).await {
                        warn!(error = %e, "heart-rate sensor session ended; retrying in 5s");
                    }
                    {
                        let mut st = state.write().await;
                        st.sensors.hr_connected = false;
                        st.sensors.hr_bpm = None;
                    }
                    sleep(Duration::from_secs(5)).await;
                };
                tokio::select! {
                    ctrl = ctrl_rx.recv() => match ctrl {
                        None => return,
                        Some(SensorControl::Disconnect) => break,
                        Some(SensorControl::Connect) => {} // re-scan from the top
                    },
                    _ = attempt => {} // attempt finished/failed → loop and retry
                }
            }

            // ── Back to idle: clear all sensor state. ──
            let mut st = state.write().await;
            st.sensors = Default::default();
            info!("heart-rate: disconnected (idle)");
        }
    }

    /// Set the "scanning/searching" flag without disturbing other sensor fields.
    async fn set_scanning(state: &Arc<RwLock<AppState>>, scanning: bool) {
        state.write().await.sensors.hr_scanning = scanning;
    }

    async fn session_once(
        state: &Arc<RwLock<AppState>>,
        adapter_name: &str,
        name_filter: &str,
    ) -> bluer::Result<()> {
        let session = Session::new().await?;
        let adapter = match session.adapter(adapter_name) {
            Ok(a) => a,
            Err(_) => session.default_adapter().await?,
        };
        adapter.set_powered(true).await?;

        info!("scanning for a heart-rate sensor…");
        let device = find_sensor(&adapter, name_filter).await?;
        let name = device.name().await?.unwrap_or_default();
        info!(%name, "heart-rate sensor found; connecting");
        if !device.is_connected().await? {
            device.connect().await?;
        }

        let ch = hr_characteristic(&device)
            .await?
            .ok_or_else(|| bluer::Error {
                kind: bluer::ErrorKind::NotFound,
                message: "no heart-rate measurement characteristic".into(),
            })?;
        // bluer's notify stream isn't Unpin; pin it so `.next()` works.
        let mut notify = Box::pin(ch.notify().await?);
        {
            let mut st = state.write().await;
            st.sensors.hr_connected = true;
        }
        info!("subscribed to heart-rate notifications");

        while let Some(data) = notify.next().await {
            if let Some(bpm) = parse_hr_measurement(&data) {
                let mut st = state.write().await;
                st.sensors.hr_bpm = Some(bpm);
                st.sensors.hr_connected = true;
            }
        }
        Ok(()) // notifications ended → treat as disconnect, supervisor retries
    }

    /// Discover the first device advertising the Heart Rate Service (optionally
    /// filtered by a name substring).
    async fn find_sensor(adapter: &bluer::Adapter, name_filter: &str) -> bluer::Result<Device> {
        let mut events = adapter.discover_devices().await?;
        while let Some(ev) = events.next().await {
            if let AdapterEvent::DeviceAdded(addr) = ev {
                let device = adapter.device(addr)?;
                let advertises_hr = device
                    .uuids()
                    .await?
                    .map(|u| u.contains(&HR_SERVICE))
                    .unwrap_or(false);
                let name_ok = name_filter.is_empty()
                    || device
                        .name()
                        .await?
                        .map(|n| n.to_lowercase().contains(&name_filter.to_lowercase()))
                        .unwrap_or(false);
                if advertises_hr && name_ok {
                    return Ok(device);
                }
            }
        }
        Err(bluer::Error {
            kind: bluer::ErrorKind::NotFound,
            message: "device discovery stream ended".into(),
        })
    }

    async fn hr_characteristic(
        device: &Device,
    ) -> bluer::Result<Option<bluer::gatt::remote::Characteristic>> {
        for service in device.services().await? {
            if service.uuid().await? == HR_SERVICE {
                for ch in service.characteristics().await? {
                    if ch.uuid().await? == HR_MEASUREMENT {
                        return Ok(Some(ch));
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_hr_measurement;

    #[test]
    fn parses_uint8_and_uint16_formats() {
        // flags=0x00 → u8 BPM in byte 1
        assert_eq!(parse_hr_measurement(&[0x00, 72]), Some(72));
        // flags=0x01 → u16 LE BPM in bytes 1..3 (300 = 0x012C)
        assert_eq!(parse_hr_measurement(&[0x01, 0x2C, 0x01]), Some(300));
        // too short → None
        assert_eq!(parse_hr_measurement(&[0x00]), None);
        assert_eq!(parse_hr_measurement(&[]), None);
    }
}
