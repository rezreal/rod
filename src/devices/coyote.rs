//! DG-LAB Coyote 3.0 e-stim power box — BLE-central actuator driver (spike).
//!
//! buttplug doesn't cover the Coyote, but DG-LAB publishes the BLE protocol
//! (<https://github.com/dungeonlab-open/dglab-bluetooth-protocol>, coyote/v3), so
//! we drive it directly as a custom device, same BLE-central role as the
//! heart-rate sensor (`src/sensors/`). It's an e-stim unit with two channels
//! (A/B); each carries an overall **strength** plus a streamed **waveform**
//! (4 frequency + 4 intensity "pulse" values per 100 ms frame). The device stops
//! if the stream stops — a built-in fail-safe.
//!
//! SAFETY: e-stim demands care. Strength is clamped to a configurable
//! `max_strength` well below the device max (200), ramped (never jumped), zeroed
//! on Stop/disconnect, and the stream-stops-it fail-safe backs that up. Start
//! low.
//!
//! This is a SPIKE: the pure protocol encoders below are unit-tested, but the
//! `bluer` connect/stream path (Linux-only) has NOT been validated on hardware.

use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::config::Coyote;
use crate::state::AppState;

/// Control messages to the Coyote driver.
#[derive(Debug, Clone, PartialEq)]
pub enum CoyoteControl {
    /// Set per-channel target strength (clamped to `max_strength`, then ramped).
    /// Switches off follow mode (manual takes over).
    SetStrength { a: u8, b: u8 },
    /// Follow the rod's motion intensity: both channels track
    /// `motion_intensity × scale × max_strength`. `scale` 0..1 dials how much of
    /// the cap full motion reaches.
    Follow { enable: bool, scale: f32 },
    /// Immediately zero both channels (output stops); also leaves follow mode.
    Stop,
}

// ───────────────────────────── protocol (pure) ──────────────────────────────

/// Device max for the strength byte (we cap well below this in config).
pub const DEVICE_MAX_STRENGTH: u8 = 200;

/// One channel's frame: absolute strength + 4 waveform pulses (device units).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFrame {
    pub strength: u8,
    pub freq: [u8; 4],
    pub intensity: [u8; 4],
}

/// Encode a user-facing frequency input (10–1000) to the device value (10–240),
/// per the V3 piecewise mapping.
pub fn encode_frequency(input: u16) -> u8 {
    let i = input.clamp(10, 1000);
    let v = if i <= 100 {
        i
    } else if i <= 600 {
        (i - 100) / 5 + 100
    } else {
        (i - 600) / 10 + 200
    };
    v.clamp(10, 240) as u8
}

/// Build a B0 command (20 bytes): both channels' strength + waveform, absolute
/// strength mode. `seq` is a 4-bit rolling sequence number.
pub fn encode_b0(seq: u8, a: &ChannelFrame, b: &ChannelFrame) -> [u8; 20] {
    let mut p = [0u8; 20];
    p[0] = 0xB0;
    // byte 1: high nibble = seq; low nibble = strength-interpretation, two 2-bit
    // segments (A=bits[3:2], B=bits[1:0]); 0b11 = absolute set → 0b1111.
    p[1] = ((seq & 0x0F) << 4) | 0b1111;
    p[2] = a.strength.min(DEVICE_MAX_STRENGTH);
    p[3] = b.strength.min(DEVICE_MAX_STRENGTH);
    p[4..8].copy_from_slice(&a.freq);
    p[8..12].copy_from_slice(&a.intensity);
    p[12..16].copy_from_slice(&b.freq);
    p[16..20].copy_from_slice(&b.intensity);
    p
}

/// Build a BF command (7 bytes): per-channel soft strength limits + balance
/// params. (Defense-in-depth; we also clamp in software. Not sent by the spike
/// until the balance-param defaults are confirmed on hardware.)
pub fn encode_bf(a_limit: u8, b_limit: u8, freq_bal: u8, intensity_bal: u8) -> [u8; 7] {
    [
        0xBF,
        a_limit.min(DEVICE_MAX_STRENGTH),
        b_limit.min(DEVICE_MAX_STRENGTH),
        freq_bal,
        freq_bal,
        intensity_bal,
        intensity_bal,
    ]
}

/// Parsed B1 notification: the device's *actual* current strengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoyoteStatus {
    pub seq: u8,
    pub strength_a: u8,
    pub strength_b: u8,
}

pub fn parse_b1(data: &[u8]) -> Option<CoyoteStatus> {
    if data.first() != Some(&0xB1) || data.len() < 4 {
        return None;
    }
    Some(CoyoteStatus {
        seq: data[1],
        strength_a: data[2],
        strength_b: data[3],
    })
}

/// Build the steady waveform frame for a channel at the given strength, using a
/// single (freq, intensity) repeated across the 4 pulse slots.
pub fn steady_frame(strength: u8, freq_input: u16, intensity: u8) -> ChannelFrame {
    let f = encode_frequency(freq_input);
    let i = intensity.min(100);
    ChannelFrame {
        strength,
        freq: [f; 4],
        intensity: [i; 4],
    }
}

// ───────────────────────────── driver (Linux) ───────────────────────────────

#[cfg(target_os = "linux")]
pub use imp::run;

#[cfg(not(target_os = "linux"))]
pub async fn run(
    _state: Arc<RwLock<AppState>>,
    _ctrl_rx: mpsc::Receiver<CoyoteControl>,
    _cfg: Coyote,
    _adapter: String,
) {
    tracing::warn!("Coyote driver requires Linux/BlueZ — skipping on this platform.");
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
    const SVC_CMD: Uuid = bt(0x180C);
    const CH_WRITE: Uuid = bt(0x150A);
    const CH_NOTIFY: Uuid = bt(0x150B);
    const CH_BATTERY: Uuid = bt(0x1500);
    /// Coyote 3.0 host advertises this name.
    const DEFAULT_NAME: &str = "47L121000";
    /// Streaming cadence — the device requires a frame ~every 100 ms.
    const FRAME: Duration = Duration::from_millis(100);
    /// Max strength change per frame (ramp), so output never jumps.
    const RAMP_STEP: u8 = 1;

    pub async fn run(
        state: Arc<RwLock<AppState>>,
        mut ctrl_rx: mpsc::Receiver<CoyoteControl>,
        cfg: Coyote,
        adapter: String,
    ) {
        info!("coyote driver running");
        loop {
            if let Err(e) = session(&state, &mut ctrl_rx, &cfg, &adapter).await {
                warn!(error = %e, "coyote session ended; retrying in 5s");
            }
            {
                let mut st = state.write().await;
                st.coyote.connected = false;
                st.coyote.strength_a = 0;
                st.coyote.strength_b = 0;
            }
            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn session(
        state: &Arc<RwLock<AppState>>,
        ctrl_rx: &mut mpsc::Receiver<CoyoteControl>,
        cfg: &Coyote,
        adapter_name: &str,
    ) -> bluer::Result<()> {
        let cap = cfg.max_strength.min(DEVICE_MAX_STRENGTH);
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
        info!(%name, "scanning for Coyote…");
        let device = find_device(&adapter, name).await?;
        if !device.is_connected().await? {
            device.connect().await?;
        }

        let (write, notify, battery) = characteristics(&device).await?;
        // bluer's notify stream isn't Unpin; pin it so `.next()` works in select!.
        let mut notify_stream = Box::pin(notify.notify().await?);
        if let Some(b) = &battery {
            if let Ok(v) = b.read().await {
                if let Some(level) = v.first() {
                    state.write().await.coyote.battery = Some(*level);
                }
            }
        }
        state.write().await.coyote.connected = true;
        info!("coyote connected; streaming (capped at {cap})");

        let mut tick = interval(FRAME);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut target_a = 0u8;
        let mut target_b = 0u8;
        let mut cur_a = 0u8;
        let mut cur_b = 0u8;
        let mut seq = 0u8;
        let mut following = false;
        let mut scale = cfg.follow_scale.clamp(0.0, 1.0);

        loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => match ctrl {
                    None => return Ok(()),
                    Some(CoyoteControl::SetStrength { a, b }) => {
                        following = false;
                        target_a = a.min(cap);
                        target_b = b.min(cap);
                        state.write().await.coyote.following = false;
                    }
                    Some(CoyoteControl::Follow { enable, scale: s }) => {
                        following = enable;
                        scale = s.clamp(0.0, 1.0);
                        if !enable { target_a = 0; target_b = 0; }
                        state.write().await.coyote.following = enable;
                    }
                    Some(CoyoteControl::Stop) => {
                        // Immediate zero (not ramped) on an explicit stop.
                        following = false;
                        target_a = 0; target_b = 0; cur_a = 0; cur_b = 0;
                        state.write().await.coyote.following = false;
                    }
                },
                Some(data) = notify_stream.next() => {
                    if let Some(s) = parse_b1(&data) {
                        let mut st = state.write().await;
                        st.coyote.strength_a = s.strength_a;
                        st.coyote.strength_b = s.strength_b;
                    }
                }
                _ = tick.tick() => {
                    if following {
                        // Track the rod's motion intensity → e-stim strength.
                        let mi = state.read().await.motion_intensity.clamp(0.0, 1.0);
                        let t = (mi * scale * cap as f32).round() as u8;
                        target_a = t.min(cap);
                        target_b = t.min(cap);
                    }
                    cur_a = ramp(cur_a, target_a);
                    cur_b = ramp(cur_b, target_b);
                    let fa = steady_frame(cur_a, cfg.waveform_freq, cfg.waveform_intensity);
                    let fb = steady_frame(cur_b, cfg.waveform_freq, cfg.waveform_intensity);
                    let frame = encode_b0(seq, &fa, &fb);
                    seq = seq.wrapping_add(1) & 0x0F;
                    if let Err(e) = write.write(&frame).await {
                        warn!(error = %e, "coyote frame write failed");
                        return Ok(()); // drop session → reconnect (fail-safe stops output)
                    }
                }
            }
        }
    }

    fn ramp(cur: u8, target: u8) -> u8 {
        if cur < target {
            (cur + RAMP_STEP).min(target)
        } else if cur > target {
            cur.saturating_sub(RAMP_STEP).max(target)
        } else {
            cur
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
                        .map(|u| u.contains(&SVC_CMD))
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
    async fn characteristics(device: &Device) -> bluer::Result<(Chr, Chr, Option<Chr>)> {
        let (mut write, mut notify, mut battery) = (None, None, None);
        for service in device.services().await? {
            for ch in service.characteristics().await? {
                match ch.uuid().await? {
                    u if u == CH_WRITE => write = Some(ch),
                    u if u == CH_NOTIFY => notify = Some(ch),
                    u if u == CH_BATTERY => battery = Some(ch),
                    _ => {}
                }
            }
        }
        match (write, notify) {
            (Some(w), Some(n)) => Ok((w, n, battery)),
            _ => Err(bluer::Error {
                kind: bluer::ErrorKind::NotFound,
                message: "Coyote command/notify characteristics not found".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_encoding_piecewise() {
        assert_eq!(encode_frequency(10), 10);
        assert_eq!(encode_frequency(100), 100);
        assert_eq!(encode_frequency(600), 200); // (600-100)/5+100 = 200
        assert_eq!(encode_frequency(1000), 240); // (1000-600)/10+200 = 240
        assert_eq!(encode_frequency(5), 10); // clamped low
        assert_eq!(encode_frequency(5000), 240); // clamped high
    }

    #[test]
    fn b0_layout_is_correct() {
        let a = ChannelFrame {
            strength: 20,
            freq: [10, 10, 10, 10],
            intensity: [5, 5, 5, 5],
        };
        let b = ChannelFrame {
            strength: 0,
            freq: [50, 50, 50, 50],
            intensity: [0, 0, 0, 0],
        };
        let p = encode_b0(3, &a, &b);
        assert_eq!(p[0], 0xB0);
        assert_eq!(p[1], (3 << 4) | 0x0F); // seq=3, absolute both
        assert_eq!(p[2], 20); // A strength
        assert_eq!(p[3], 0); // B strength
        assert_eq!(&p[4..8], &[10, 10, 10, 10]); // A freq
        assert_eq!(&p[8..12], &[5, 5, 5, 5]); // A intensity
        assert_eq!(&p[12..16], &[50, 50, 50, 50]); // B freq
        assert_eq!(p.len(), 20);
    }

    #[test]
    fn b0_clamps_strength_to_device_max() {
        let c = ChannelFrame {
            strength: 255,
            freq: [10; 4],
            intensity: [0; 4],
        };
        let p = encode_b0(0, &c, &c);
        assert_eq!(p[2], DEVICE_MAX_STRENGTH);
    }

    #[test]
    fn parse_b1_reads_current_strength() {
        assert_eq!(
            parse_b1(&[0xB1, 0x02, 15, 8]),
            Some(CoyoteStatus {
                seq: 2,
                strength_a: 15,
                strength_b: 8
            })
        );
        assert_eq!(parse_b1(&[0x00, 1, 2, 3]), None); // wrong head
        assert_eq!(parse_b1(&[0xB1, 0]), None); // too short
    }
}
