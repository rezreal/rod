//! The Modbus RTU driver: **sole owner** of the serial port (SPEC §8).
//!
//! All bus traffic is serialised here. The driver runs one task that:
//!  * executes [`ActuatorCommand`]s coming from the modes/dispatcher, and
//!  * polls the status block on a timer, updating [`AppState`] and emitting
//!    `NotificationHdspChanged` / `NotificationError` as things change.
//!
//! Movement commands are prioritised over the status poll (`biased` select).
//!
//! The low-level bus is abstracted behind [`ModbusBus`] so the driver logic can
//! be unit-tested with an in-memory fake (no serial port / no hardware).

use std::io;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::{sleep, Instant};
use tracing::{debug, error, info, warn};

use std::sync::Arc;

use crate::config::{Config, MotionProfile};
use crate::modbus::protocol::{self, Dss1, Dsse, MoveCommand, StatusBlock};
use crate::rpc::{self, HdspPlayState, NotificationHdspChanged, RpcMessage};
use crate::state::{ActuatorCommand, AppMode, AppState, BridgeCommand, GameKind};

/// Inter-frame "silent interval" for Modbus RTU at 19200 baud (~2 ms).
const SILENT_INTERVAL: Duration = Duration::from_millis(2);
/// Re-engage settle after a peck-probe servo-off cycle.
const PECK_SETTLE: Duration = Duration::from_millis(50);
/// Longer settle used between alarm-reset edges (knock-rod uses 20 ms).
const ALARM_RESET_GAP: Duration = Duration::from_millis(20);
/// Max retries for a single transaction on CRC/timeout (SPEC §10).
const MAX_RETRIES: usize = 3;
/// Controller speed clamp (0.01 mm/s): knock-rod uses 1..=50000 (≤500 mm/s).
const VCMD_MIN: u32 = 1;
const VCMD_MAX: u32 = 50000;
/// Consecutive failed status polls before declaring the serial link lost and
/// driving the reconnect loop. Each failed poll already burns its own retries,
/// so a small count here is several seconds of genuine silence.
const RECONNECT_AFTER_POLL_FAILURES: u32 = 3;
/// Mechanical clearance kept off the far physical hard stop for every
/// mode-driven move (`depth_scaled`), regardless of zone or depth-ceiling
/// config. Ramming the hard stop trips a controller alarm and can leave
/// homing unable to recover (see `home`'s doc comment). Calibration/
/// peck-probe bypass this — they work in real physical coordinates and need
/// to reach the actual end to detect contact.
const HARD_STOP_MARGIN_MM: f32 = 3.0;

/// Retry a single bus transaction up to `MAX_RETRIES` times on I/O error
/// (SPEC §10). Expands to `self.bus.<call>.await` re-evaluated per attempt —
/// a macro sidesteps the borrowed-future lifetime issues of a closure form.
macro_rules! retry {
    ($self:ident, $what:literal, $method:ident ( $($arg:expr),* $(,)? )) => {{
        let mut result: io::Result<_> = Err(io::Error::other("modbus retry exhausted"));
        for attempt in 1..=MAX_RETRIES {
            match $self.bus.$method($($arg),*).await {
                Ok(v) => { result = Ok(v); break; }
                Err(e) => {
                    warn!(what = $what, attempt, error = %e, "modbus transaction failed");
                    result = Err(e);
                    sleep(SILENT_INTERVAL).await;
                }
            }
        }
        result
    }};
}

/// Low-level Modbus PDU operations. `tokio-modbus` owns the slave address and
/// CRC; this trait only deals in register/coil values.
pub trait ModbusBus: Send {
    fn read_holding_registers(
        &mut self,
        addr: u16,
        cnt: u16,
    ) -> impl std::future::Future<Output = io::Result<Vec<u16>>> + Send;

    fn write_single_coil(
        &mut self,
        addr: u16,
        on: bool,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send;

    fn write_multiple_registers(
        &mut self,
        addr: u16,
        data: &[u16],
    ) -> impl std::future::Future<Output = io::Result<()>> + Send;

    /// Re-establish the underlying link after a disconnect (adapter unplugged
    /// or renumbered, e.g. `ttyUSB0` → `ttyUSB1`). Implementations that don't
    /// back a physical port (test fakes) leave this unsupported.
    fn reconnect(&mut self) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "reconnect not supported by this bus",
            ))
        }
    }

    /// Whether the link is currently open. Test fakes are always "connected".
    fn is_connected(&self) -> bool {
        true
    }
}

/// Resolve a configured serial-device string to a path that currently exists.
///
/// An exact, present path is used as-is. Otherwise — the configured path is gone
/// (adapter renumbered to a different `ttyUSB*`) or it's a glob like
/// `/dev/ttyUSB*` — scan `/dev` and pick the first USB serial adapter, preferring
/// `ttyUSB*` over `ttyACM*`. This lets the bridge follow the adapter across
/// kernel re-enumeration (`ttyUSB0` → `ttyUSB1`) without a config edit.
fn resolve_serial_device(configured: &str) -> anyhow::Result<String> {
    let is_glob = configured.contains('*');
    if !is_glob && std::path::Path::new(configured).exists() {
        return Ok(configured.to_string());
    }
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("ttyUSB") || name.starts_with("ttyACM") {
                    candidates.push(format!("/dev/{name}"));
                }
            }
        }
    }
    candidates.sort();
    // Stable key keeps lexical order within each family; ttyUSB before ttyACM.
    candidates.sort_by_key(|p| u8::from(p.contains("ttyACM")));
    candidates.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no serial device found (configured {configured}; scanned /dev/ttyUSB*, /dev/ttyACM*) \
             — is the actuator connected?"
        )
    })
}

/// Real serial bus backed by `tokio-modbus` RTU over `tokio-serial`. Holds the
/// open parameters so it can re-resolve and re-open the port after a disconnect.
pub struct SerialBus {
    /// `None` while the port is closed (between disconnect and a successful
    /// [`reconnect`](SerialBus::reconnect)).
    ctx: Option<tokio_modbus::client::Context>,
    /// Configured device string (exact path or glob) — re-resolved on reconnect.
    device: String,
    baud: u32,
    slave: u8,
}

impl SerialBus {
    /// Open the serial port and attach a Modbus RTU client for `slave`.
    pub fn open(serial_device: &str, baud_rate: u32, slave: u8) -> anyhow::Result<Self> {
        let path = resolve_serial_device(serial_device)?;
        if path != serial_device {
            info!(configured = serial_device, resolved = %path, "serial device resolved by scan");
        }
        let ctx = Self::open_ctx(&path, baud_rate, slave)?;
        Ok(SerialBus {
            ctx: Some(ctx),
            device: serial_device.to_string(),
            baud: baud_rate,
            slave,
        })
    }

    /// Construct without opening the port. The driver establishes the link in
    /// its run loop (and re-establishes it on hot-plug), so the bridge can boot
    /// without the actuator attached.
    pub fn disconnected(serial_device: &str, baud_rate: u32, slave: u8) -> Self {
        SerialBus {
            ctx: None,
            device: serial_device.to_string(),
            baud: baud_rate,
            slave,
        }
    }

    /// Open a fresh Modbus RTU context on a concrete `path`.
    fn open_ctx(path: &str, baud: u32, slave: u8) -> anyhow::Result<tokio_modbus::client::Context> {
        use tokio_serial::SerialPortBuilderExt;
        let builder = tokio_serial::new(path, baud)
            .data_bits(tokio_serial::DataBits::Eight)
            .stop_bits(tokio_serial::StopBits::One)
            .parity(tokio_serial::Parity::None)
            .timeout(Duration::from_millis(200));
        let port = builder
            .open_native_async()
            .map_err(|e| anyhow::anyhow!("opening serial port {path} @ {baud} 8N1: {e}"))?;
        Ok(tokio_modbus::client::rtu::attach_slave(
            port,
            tokio_modbus::Slave(slave),
        ))
    }

    /// Borrow the live context, or a `NotConnected` error if the port is closed.
    fn ctx(&mut self) -> io::Result<&mut tokio_modbus::client::Context> {
        self.ctx
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "serial port not open"))
    }
}

impl ModbusBus for SerialBus {
    async fn read_holding_registers(&mut self, addr: u16, cnt: u16) -> io::Result<Vec<u16>> {
        use tokio_modbus::client::Reader;
        self.ctx()?.read_holding_registers(addr, cnt).await
    }
    async fn write_single_coil(&mut self, addr: u16, on: bool) -> io::Result<()> {
        use tokio_modbus::client::Writer;
        self.ctx()?.write_single_coil(addr, on).await
    }
    async fn write_multiple_registers(&mut self, addr: u16, data: &[u16]) -> io::Result<()> {
        use tokio_modbus::client::Writer;
        self.ctx()?.write_multiple_registers(addr, data).await
    }

    fn is_connected(&self) -> bool {
        self.ctx.is_some()
    }

    async fn reconnect(&mut self) -> io::Result<()> {
        // Drop the old context *first* so re-opening the same node can't fail
        // with EBUSY ("Device or resource busy") on our own lingering fd.
        self.ctx = None;
        let path = resolve_serial_device(&self.device).map_err(io_other)?;
        let ctx = Self::open_ctx(&path, self.baud, self.slave).map_err(io_other)?;
        self.ctx = Some(ctx);
        info!(path = %path, "serial port reopened");
        Ok(())
    }
}

/// The driver owns the bus, shared state, and the outbound notification bus.
pub struct ModbusDriver<B: ModbusBus> {
    bus: B,
    state: Arc<RwLock<AppState>>,
    notif: broadcast::Sender<RpcMessage>,
    /// Effective stroke length (mm), used to clamp absolute targets.
    stroke_mm: f32,
    /// Default acceleration (0.01 G) and motion profile.
    default_accel_001g: u16,
    /// Max velocity (mm/s), used to normalise commanded speed → motion intensity.
    max_velocity_mm_s: f32,
    /// Force-release the holding brake when the servo is off (BKRL).
    release_brake: bool,
    /// Status poll cadence.
    poll_interval: Duration,
    /// Last emitted HDSP play-state, to debounce `NotificationHdspChanged`.
    last_hdsp_moving: bool,
    /// Last alarm code emitted, to debounce `NotificationError`.
    last_alarm: u16,
    /// Push-to-contact calibration parameters (from `[actuator.calibration]`).
    cal_touch_vel: f32,
    cal_push_current: u16,
    cal_accel: f32,
    /// Forward search distance for calibration (mm); 0 → full stroke.
    cal_max_travel: f32,
    /// Peck-probe parameters (from `[actuator.peck_probe]`).
    peck_coarse_step: f32,
    peck_fine_step: f32,
    peck_fine_back: f32,
    peck_move_vel: f32,
    peck_release: Duration,
    peck_threshold: f32,
    peck_max_travel: f32,
    peck_return_vel: f32,
}

impl<B: ModbusBus> ModbusDriver<B> {
    pub fn new(
        bus: B,
        state: Arc<RwLock<AppState>>,
        notif: broadcast::Sender<RpcMessage>,
        cfg: &Config,
    ) -> Self {
        ModbusDriver {
            bus,
            state,
            notif,
            stroke_mm: cfg.stroke_mm(),
            default_accel_001g: (cfg.actuator.limits.default_accel_g * 100.0) as u16,
            max_velocity_mm_s: cfg.actuator.limits.max_velocity_mm_s.max(1.0),
            release_brake: cfg.actuator.release_brake_on_servo_off,
            poll_interval: Duration::from_millis(80),
            last_hdsp_moving: false,
            last_alarm: 0,
            cal_touch_vel: cfg.actuator.calibration.touch_velocity_mm_s,
            cal_push_current: cfg.actuator.calibration.push_current_pct,
            cal_accel: cfg.actuator.calibration.touch_accel_g,
            cal_max_travel: cfg.actuator.calibration.max_travel_mm,
            peck_coarse_step: cfg.actuator.peck_probe.coarse_step_mm,
            peck_fine_step: cfg.actuator.peck_probe.fine_step_mm,
            peck_fine_back: cfg.actuator.peck_probe.fine_back_mm,
            peck_move_vel: cfg.actuator.peck_probe.move_velocity_mm_s,
            peck_release: Duration::from_millis(cfg.actuator.peck_probe.release_ms),
            peck_threshold: cfg.actuator.peck_probe.springback_threshold_mm,
            peck_max_travel: cfg.actuator.peck_probe.max_travel_mm,
            peck_return_vel: cfg.actuator.peck_probe.return_velocity_mm_s,
        }
    }

    // ───────────────────────── startup primitives ─────────────────────────

    /// ALRS alarm-reset edge: FF00 → (gap) → 0000.
    pub async fn reset_alarm(&mut self) -> io::Result<()> {
        retry!(
            self,
            "alarm_reset_on",
            write_single_coil(protocol::COIL_ALARM_RESET, true)
        )?;
        sleep(ALARM_RESET_GAP).await;
        retry!(
            self,
            "alarm_reset_off",
            write_single_coil(protocol::COIL_ALARM_RESET, false)
        )
    }

    /// Enable Modbus commands (PMSS / PIO-Modbus switch = ON).
    pub async fn pio_modbus_on(&mut self) -> io::Result<()> {
        retry!(
            self,
            "pio_modbus_on",
            write_single_coil(protocol::COIL_PIO_MODBUS, true)
        )
    }

    /// SON servo enable/disable.
    pub async fn set_servo(&mut self, on: bool) -> io::Result<()> {
        retry!(self, "servo", write_single_coil(protocol::COIL_SERVO, on))?;
        // Free the rod when the servo is off: force-release the holding brake
        // (BKRL); re-engage it (normal control) when the servo comes back on.
        if self.release_brake {
            let release = !on;
            retry!(
                self,
                "brake_release",
                write_single_coil(protocol::COIL_BRAKE_RELEASE, release)
            )?;
            self.state.write().await.brake_released = release;
        }
        self.state.write().await.servo_on = on;
        Ok(())
    }

    /// Park: servo off while keeping the rod clamped in place. The holding
    /// brake is fail-safe — it engages on its own when the servo de-energises —
    /// so parking just means *not* force-releasing it. We override the global
    /// `release_brake_on_servo_off` default here: a scene that parks wants the
    /// rod held, never freed. (On a vertical mount `release_brake` is already
    /// false, so the brake holds without us touching the coil.)
    pub async fn park(&mut self) -> io::Result<()> {
        retry!(
            self,
            "servo",
            write_single_coil(protocol::COIL_SERVO, false)
        )?;
        if self.release_brake {
            retry!(
                self,
                "brake_release",
                write_single_coil(protocol::COIL_BRAKE_RELEASE, false)
            )?;
            self.state.write().await.brake_released = false;
        }
        self.state.write().await.servo_on = false;
        Ok(())
    }

    /// Deceleration stop (STOP edge = FF00). Keeps the servo on.
    pub async fn decel_stop(&mut self) -> io::Result<()> {
        self.state.write().await.motion_intensity = 0.0;
        retry!(
            self,
            "decel_stop",
            write_single_coil(protocol::COIL_DECEL_STOP, true)
        )
    }

    /// Trigger an IAI home-return edge. We follow knock-rod's order: drive the
    /// HOME coil low then high to guarantee a clean rising edge, which also makes
    /// the operation re-callable for `RequestSliderCalibrate`.
    async fn home_edge(&mut self) -> io::Result<()> {
        retry!(
            self,
            "home_low",
            write_single_coil(protocol::COIL_HOME, false)
        )?;
        sleep(SILENT_INTERVAL).await;
        retry!(
            self,
            "home_high",
            write_single_coil(protocol::COIL_HOME, true)
        )
    }

    /// Full homing: issue the edge, settle, then poll DSS1.HEND for up to ~12 s.
    pub async fn home(&mut self) -> anyhow::Result<()> {
        self.state.write().await.homing_complete = false;
        self.home_edge().await?;
        sleep(Duration::from_millis(200)).await;

        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let status = self.read_status().await?;
            if status.home_complete() {
                let mut st = self.state.write().await;
                st.homing_complete = true;
                st.position_mm = status.position_mm();
                info!("homing complete");
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("homing did not complete within 12 s");
            }
            sleep(Duration::from_millis(200)).await;
        }
    }

    /// Full startup / calibration sequence (SPEC §9). Mirrors `KnockRod.init()`:
    /// reset alarm → enable Modbus → servo on → seed status → home if needed.
    /// Leaves the device in Idle with `homing_complete` set.
    pub async fn startup(&mut self) -> anyhow::Result<()> {
        info!("modbus startup: resetting alarm");
        self.reset_alarm().await?;
        info!("modbus startup: enabling Modbus commands (PMSS)");
        self.pio_modbus_on().await?;
        info!("modbus startup: servo on");
        self.set_servo(true).await?;

        let status = self.read_status().await?;
        {
            let mut st = self.state.write().await;
            st.position_mm = status.position_mm();
            st.servo_on = status.servo_on();
            st.homing_complete = status.home_complete();
            st.alarm_code = status.almc;
            st.actuator_connected = true; // link is up — status read succeeded
        }

        // Always home on startup. The controller's HEND bit can persist as "home
        // complete" across a fault even though the slider is jammed against a
        // mechanical stop with a stale position reference — in that state every
        // absolute move drives the blocked motor and instantly trips a deviation
        // alarm (0xA3). A fresh home-return pulls the slider off the stop and
        // re-establishes the reference. If the slider is physically stuck so hard
        // that even homing times out, log it and continue so the bridge still
        // comes up and advertises (the user can free it and reconnect).
        info!(
            home_complete = status.home_complete(),
            "modbus startup: homing to re-establish reference"
        );
        self.state
            .write()
            .await
            .set_mode(crate::state::AppMode::Homing);
        if let Err(e) = self.home().await {
            warn!(error = %e, "startup home-return failed; continuing (free the slider and reconnect)");
        }
        self.state
            .write()
            .await
            .set_mode(crate::state::AppMode::Idle);
        Ok(())
    }

    /// Read and parse the status block.
    pub async fn read_status(&mut self) -> io::Result<StatusBlock> {
        let regs = retry!(
            self,
            "status_block",
            read_holding_registers(protocol::REG_STATUS_BLOCK, protocol::STATUS_BLOCK_LEN)
        )?;
        protocol::parse_status_block(&regs)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Map a program's target into its active depth ceiling. Programs address
    /// the full stroke; the ceiling rescales `[0, stroke] → [0, ceiling]` so the
    /// mode runs between the entrance (0) and the ceiling, scaled — not
    /// hard-clamped. `0.0` (unset) means full stroke (no scaling). Calibration/
    /// peck-probe call `move_to`/`move_push` directly and are intentionally
    /// *not* scaled (they work in real physical coordinates).
    ///
    /// Which ceiling applies depends on the active mode: modes that oscillate
    /// in a fixed zone (HAMP, cycle, pulse, plumb, surge, tide, trace, tempo,
    /// and the fixed-zone games — edge & recover, gauntlet, deadman's climb,
    /// stillness) use `comfortable_depth_mm`. Modes that press toward or hold a
    /// single far point (ramp, HDSP, HSP, learn, drill, impale, echo, and the
    /// Hold the Line game) use `max_depth_mm`.
    ///
    /// Whatever the ceiling scaling produces is then hard-clamped away from
    /// the far physical stop by `HARD_STOP_MARGIN_MM` — this is the single
    /// choke point every mode's moves pass through (they all queue
    /// `ActuatorCommand::MoveTo`/`MovePush`, handled in `execute`), so it's
    /// where a stop margin protects all of them uniformly instead of relying
    /// on each mode's own defaults staying sane.
    async fn depth_scaled(&mut self, pos_mm: f32) -> f32 {
        let st = self.state.read().await;
        let ceiling = match st.mode {
            AppMode::Ramp
            | AppMode::Hdsp
            | AppMode::Hsp
            | AppMode::Learn
            | AppMode::Drill
            | AppMode::Impale
            | AppMode::Echo => st.max_depth_mm,
            AppMode::Game if st.game.kind == Some(GameKind::HoldTheLine) => st.max_depth_mm,
            _ => st.comfortable_depth_mm,
        };
        let scaled = if ceiling > 0.0 && ceiling < self.stroke_mm {
            pos_mm * (ceiling / self.stroke_mm)
        } else {
            pos_mm
        };
        scaled.min(self.stroke_mm - HARD_STOP_MARGIN_MM)
    }

    /// Execute a single move (FC 0x10 @ 0x9900). Converts engineering units to
    /// the controller's native 0.01-mm / 0.01-mm·s⁻¹ / 0.01-G scaling and clamps
    /// to the hard controller limits, mirroring knock-rod's `moveTo`.
    pub async fn move_to(
        &mut self,
        pos_mm: f32,
        vel_mm_s: f32,
        accel_g: f32,
        profile: MotionProfile,
    ) -> io::Result<()> {
        let pos_001 = (pos_mm * 100.0).round() as i32;
        let pos_001 = pos_001.clamp(0, (self.stroke_mm * 100.0) as i32);
        let vel_001 =
            ((vel_mm_s * 100.0).round() as i64).clamp(VCMD_MIN as i64, VCMD_MAX as i64) as u32;
        let accel_001 = if accel_g > 0.0 {
            (accel_g * 100.0).round() as u16
        } else {
            self.default_accel_001g
        };
        let cmd = MoveCommand::new(pos_001, vel_001, accel_001, profile);
        let regs = cmd.to_registers();
        debug!(pos_mm, vel_mm_s, ?profile, ?regs, "move_to");
        retry!(
            self,
            "move",
            write_multiple_registers(protocol::REG_MOVE_BLOCK, &regs)
        )?;
        let mut st = self.state.write().await;
        st.target_mm = pos_001 as f32 / 100.0;
        st.is_moving = true;
        // Normalised commanded speed (0..1) — a universal "how hard the rod is
        // moving" signal that external devices (e.g. the Coyote) can follow.
        st.motion_intensity = (vel_mm_s / self.max_velocity_mm_s).clamp(0.0, 1.0);
        Ok(())
    }

    /// Execute a **push-motion** move (FC 0x10 @ 0x9900 with CTLF.PUSH set).
    /// Like [`move_to`] but thrust is capped at `push_current_pct` so the rod
    /// presses gently into whatever it meets instead of faulting. The contact
    /// is then observable via `StatusBlock::pushing()`.
    pub async fn move_push(
        &mut self,
        pos_mm: f32,
        vel_mm_s: f32,
        accel_g: f32,
        push_current_pct: u16,
        profile: MotionProfile,
    ) -> io::Result<()> {
        let pos_001 = (pos_mm * 100.0).round() as i32;
        let pos_001 = pos_001.clamp(0, (self.stroke_mm * 100.0) as i32);
        let vel_001 =
            ((vel_mm_s * 100.0).round() as i64).clamp(VCMD_MIN as i64, VCMD_MAX as i64) as u32;
        let accel_001 = if accel_g > 0.0 {
            (accel_g * 100.0).round() as u16
        } else {
            self.default_accel_001g
        };
        let cmd = MoveCommand::new_push(pos_001, vel_001, accel_001, push_current_pct, profile);
        let regs = cmd.to_registers();
        debug!(pos_mm, vel_mm_s, push_current_pct, ?regs, "move_push");
        retry!(
            self,
            "move_push",
            write_multiple_registers(protocol::REG_MOVE_BLOCK, &regs)
        )?;
        let mut st = self.state.write().await;
        st.target_mm = pos_001 as f32 / 100.0;
        st.is_moving = true;
        Ok(())
    }

    /// Soft-touch calibration: home, then crawl forward at minimal thrust until
    /// the rod contacts the work-piece, and return the contact position (mm).
    ///
    /// This locates the *start of the work-piece* so later moves can be made
    /// relative to it. Sequence:
    ///  1. Home (re-establish the absolute zero / DSS1.HEND).
    ///  2. Issue a slow push-motion move toward the search limit.
    ///  3. Poll status: stop on `DSSE.PUSH` (contact) — recording the position —
    ///     or bail if the rod reaches the limit (nothing there), an alarm is
    ///     raised, or we time out.
    ///
    /// On success the contact position is stored in `AppState::work_origin_mm`.
    pub async fn calibrate_to_contact(&mut self) -> anyhow::Result<f32> {
        info!("calibration: homing before contact search");
        self.state.write().await.set_mode(AppMode::Homing);
        self.home().await?;

        let target_mm = if self.cal_max_travel > 0.0 {
            self.cal_max_travel.min(self.stroke_mm)
        } else {
            self.stroke_mm
        };
        self.state.write().await.calibrating = true;
        info!(
            target_mm,
            vel = self.cal_touch_vel,
            push = self.cal_push_current,
            "calibration: slow push-to-contact"
        );
        self.move_push(
            target_mm,
            self.cal_touch_vel,
            self.cal_accel,
            self.cal_push_current,
            MotionProfile::Trapezoid,
        )
        .await?;

        // Crawl can be slow (mm/s over a full stroke); allow a generous window.
        let deadline = Instant::now() + Duration::from_secs(120);
        let band_mm = 0.1;
        let outcome: anyhow::Result<f32> = loop {
            let status = self.read_status().await?;

            // An alarm during the press (e.g. blocked past threshold) → reset
            // and report failure so the caller can retry softer.
            if status.almc != 0 {
                warn!(
                    almc = status.almc,
                    "calibration: alarm during push; resetting"
                );
                let _ = self.reset_alarm().await;
                break Err(anyhow::anyhow!(
                    "controller alarm 0x{:04X} during calibration",
                    status.almc
                ));
            }

            let pos = status.position_mm();
            let reached_limit = pos >= target_mm - band_mm;

            // Contact: the rod is pressing, or positioning completed short of
            // the search limit (it stopped early because it hit something).
            if status.pushing() || (status.positioning_complete() && !reached_limit) {
                info!(pos, "calibration: contact detected");
                break Ok(pos);
            }
            if status.positioning_complete() && reached_limit {
                break Err(anyhow::anyhow!(
                    "reached search limit ({target_mm:.1} mm) without contact"
                ));
            }
            if Instant::now() >= deadline {
                break Err(anyhow::anyhow!("calibration timed out before contact"));
            }
            sleep(Duration::from_millis(50)).await;
        };

        // Always settle: decel-stop (keeps servo holding) and clear the flag.
        let _ = self.decel_stop().await;
        {
            let mut st = self.state.write().await;
            st.calibrating = false;
            st.is_moving = false;
            if let Ok(pos) = outcome {
                st.work_origin_mm = Some(pos);
            }
            st.set_mode(AppMode::Idle);
        }
        outcome
    }

    /// Issue a move and block until `DSS1.PEND` is set (positioning complete).
    /// Returns the actual position in mm. Times out after 10 s per step.
    async fn move_and_wait_pend(
        &mut self,
        pos_mm: f32,
        vel_mm_s: f32,
        accel_g: f32,
    ) -> anyhow::Result<f32> {
        self.move_to(pos_mm, vel_mm_s, accel_g, MotionProfile::Trapezoid)
            .await?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = self.read_status().await?;
            if status.almc != 0 {
                anyhow::bail!("alarm 0x{:04X} during move to {pos_mm:.1}mm", status.almc);
            }
            if status.positioning_complete() {
                return Ok(status.position_mm());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("positioning timed out moving to {pos_mm:.1}mm");
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    /// Scan from `start_mm` to `max_mm` in `step_mm` increments. At each step:
    /// release the servo, wait for spring-back, re-engage. Returns the first
    /// position where spring-back ≥ `peck_threshold`, or `None` if clear.
    async fn peck_scan(
        &mut self,
        step_mm: f32,
        start_mm: f32,
        max_mm: f32,
    ) -> anyhow::Result<Option<f32>> {
        let mut target = start_mm;
        while target <= max_mm + step_mm * 0.5 {
            let clamped = target.min(self.stroke_mm);
            let actual = self
                .move_and_wait_pend(clamped, self.peck_move_vel, self.cal_accel)
                .await?;

            retry!(
                self,
                "peck_servo_off",
                write_single_coil(protocol::COIL_SERVO, false)
            )?;
            // Release the holding brake too — otherwise it pins the rod and masks
            // the spring-back we're trying to measure.
            if self.release_brake {
                retry!(
                    self,
                    "peck_brake_off",
                    write_single_coil(protocol::COIL_BRAKE_RELEASE, true)
                )?;
            }
            sleep(self.peck_release).await;
            let free_mm = self.read_status().await?.position_mm();
            if self.release_brake {
                retry!(
                    self,
                    "peck_brake_on",
                    write_single_coil(protocol::COIL_BRAKE_RELEASE, false)
                )?;
            }
            retry!(
                self,
                "peck_servo_on",
                write_single_coil(protocol::COIL_SERVO, true)
            )?;
            {
                self.state.write().await.servo_on = true;
            }
            sleep(PECK_SETTLE).await;

            let springback = actual - free_mm;
            debug!(target, actual, free_mm, springback, "peck");

            if springback >= self.peck_threshold {
                return Ok(Some(actual));
            }
            target += step_mm;
        }
        Ok(None)
    }

    /// Two-phase peck-probe: home, coarse scan (5 mm steps by default) to
    /// bracket the contact zone, then fine scan (1 mm steps) to pin it down.
    /// Stores the result in `AppState::work_origin_mm` for relative moves.
    pub async fn peck_probe(&mut self) -> anyhow::Result<f32> {
        info!("peck-probe: homing before scan");
        self.state.write().await.set_mode(AppMode::Homing);

        // Run the fallible sequence in its own block so a `?` anywhere in it
        // (home, either scan phase, or no-contact-found) short-circuits only
        // this block, not the whole function — the mode reset below always
        // runs afterward, on both success and failure.
        let outcome: anyhow::Result<f32> = async {
            self.home().await?;

            let max_mm = if self.peck_max_travel > 0.0 {
                self.peck_max_travel.min(self.stroke_mm)
            } else {
                self.stroke_mm
            };

            // Phase 1 — coarse
            info!(
                max_mm,
                step = self.peck_coarse_step,
                "peck-probe: coarse scan"
            );
            let coarse_mm = self
                .peck_scan(self.peck_coarse_step, self.peck_coarse_step, max_mm)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("peck-probe: no contact found up to {max_mm:.0}mm")
                })?;
            info!(coarse_mm, "peck-probe: coarse contact");

            // Phase 2 — fine
            let fine_start = (coarse_mm - self.peck_fine_back).max(0.0);
            info!(
                fine_start,
                step = self.peck_fine_step,
                "peck-probe: fine scan"
            );
            let contact_mm = self
                .peck_scan(self.peck_fine_step, fine_start, coarse_mm)
                .await?
                .unwrap_or(coarse_mm);
            info!(contact_mm, "peck-probe: contact located");

            Ok(contact_mm)
        }
        .await;

        // Always settle: a failed probe must not leave the device stuck
        // reporting Homing forever — land back on Idle either way.
        {
            let mut st = self.state.write().await;
            if let Ok(pos) = &outcome {
                st.work_origin_mm = Some(*pos);
            }
            st.set_mode(AppMode::Idle);
        }
        let contact_mm = outcome?;

        info!("peck-probe: returning home");
        self.move_and_wait_pend(0.0, self.peck_return_vel, self.cal_accel)
            .await
            .map_err(|e| anyhow::anyhow!("peck-probe return failed: {e}"))?;

        Ok(contact_mm)
    }

    // ───────────────────────── command execution ─────────────────────────

    async fn execute(&mut self, cmd: ActuatorCommand) {
        // Silently drop movement commands while the controller is in alarm state.
        // The HAMP task keeps firing moves on its timer; suppressing them here
        // prevents writing to a servo that has de-energised.  ResetAlarm and Stop
        // always pass through so recovery can proceed.
        let alarm = self.state.read().await.alarm_code;
        if alarm != 0 {
            match &cmd {
                ActuatorCommand::MoveTo { .. }
                | ActuatorCommand::MovePush { .. }
                | ActuatorCommand::Home => {
                    debug!(alarm, "suppressing move while controller is in alarm");
                    return;
                }
                _ => {} // ResetAlarm / Stop / ServoOn / Park always allowed
            }
        }
        let result = match cmd {
            ActuatorCommand::MoveTo {
                pos_mm,
                vel_mm_s,
                accel_g,
                profile,
                // The shaper sits ahead of the driver and has already expanded
                // any softened move into plain sub-moves; ignore the marker here.
                soften: _,
            } => {
                let pos_mm = self.depth_scaled(pos_mm).await;
                self.move_to(pos_mm, vel_mm_s, accel_g, profile).await
            }
            ActuatorCommand::MovePush {
                pos_mm,
                vel_mm_s,
                accel_g,
                push_current_pct,
            } => {
                let pos_mm = self.depth_scaled(pos_mm).await;
                self.move_push(
                    pos_mm,
                    vel_mm_s,
                    accel_g,
                    push_current_pct,
                    MotionProfile::Trapezoid,
                )
                .await
            }
            ActuatorCommand::Home => self.home().await.map_err(io_other),
            ActuatorCommand::Stop => self.decel_stop().await,
            ActuatorCommand::ServoOn(on) => self.set_servo(on).await,
            ActuatorCommand::Park => self.park().await,
            ActuatorCommand::ResetAlarm => self.reset_alarm().await,
        };
        if let Err(e) = result {
            error!(error = %e, "actuator command failed");
        }
    }

    // ───────────────────────── status poll ─────────────────────────

    /// Poll once: refresh `AppState` and emit derived notifications. Returns the
    /// I/O error on a failed bus read so the run loop can decide when a run of
    /// failures means the link is gone and a reconnect is warranted.
    async fn poll_once(&mut self) -> io::Result<()> {
        let status = match self.read_status().await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "status poll failed");
                return Err(e);
            }
        };

        {
            let mut st = self.state.write().await;
            st.position_mm = status.position_mm();
            st.homing_complete = status.home_complete();
            st.servo_on = status.servo_on();
            st.alarm_code = status.almc;
            st.is_moving = status.moving();
            // Granular DSS1/DSSE bits for SSCP telemetry.
            st.controller_ready = status.dss1.contains(Dss1::PWR);
            st.positioning_done = status.dss1.contains(Dss1::PEND);
            st.brake_released = status.dss1.contains(Dss1::BKRL);
            st.push_active = status.dsse.contains(Dsse::PUSH);
            st.emergency_stop =
                status.dss1.contains(Dss1::EMGS) || status.dsse.contains(Dsse::EMGP);
            st.motor_voltage_low = status.dsse.contains(Dsse::MPUV);
            st.safety_speed = status.dss1.contains(Dss1::SFTY);
            st.alarm_minor = status.dss1.contains(Dss1::ALML);
            st.alarm_major = status.dss1.contains(Dss1::ALMH);
            st.hand_switch = status.hand_switch();
        }

        // HDSP play-state notification on a moving → reached transition.
        let moving = status.moving();
        if moving != self.last_hdsp_moving {
            let play_state = if moving {
                HdspPlayState::HdspStateMoving
            } else {
                HdspPlayState::HdspStateReached
            };
            self.emit(rpc::Notification {
                id: 0,
                notification: Some(rpc::notification::Notification::NotificationHdspChanged(
                    NotificationHdspChanged {
                        state: play_state as i32,
                    },
                )),
            });
            self.last_hdsp_moving = moving;
        }

        // Alarm notification on a newly-raised alarm code.
        let alarm_newly_raised = status.almc != 0 && status.almc != self.last_alarm;
        if alarm_newly_raised {
            warn!(almc = status.almc, "controller alarm — will auto-reset");
            self.emit(rpc::Notification {
                id: 0,
                notification: Some(rpc::notification::Notification::NotificationError(
                    rpc::NotificationError {
                        code: status.almc as i32,
                        message: format!("controller alarm 0x{:04X}", status.almc),
                    },
                )),
            });
        }
        self.last_alarm = status.almc;

        // Auto-recover: when the alarm just appeared, wait briefly (so the BLE
        // notification reaches the client), then reset + re-enable the servo.
        // Moves are suppressed in execute() while alarm_code != 0, so HAMP keeps
        // running but its move commands are silently dropped — it will resume as
        // soon as the alarm clears.
        if alarm_newly_raised {
            sleep(Duration::from_millis(500)).await;
            if let Err(e) = self.reset_alarm().await {
                warn!(error = %e, "alarm auto-reset failed");
            } else {
                // Re-enable servo: it de-energises on major alarm.
                if let Err(e) = self.set_servo(true).await {
                    warn!(error = %e, "servo re-enable after alarm-reset failed");
                } else {
                    info!(almc = status.almc, "alarm auto-reset; servo re-enabled");
                    self.last_alarm = 0;
                }
            }
        }
        Ok(())
    }

    /// Recover the serial link after the adapter was unplugged or renumbered.
    /// Loops with capped backoff until the port re-opens and the §9 startup
    /// (alarm reset → servo on → home) completes, so a replug auto-recovers
    /// without restarting the bridge. While down, reflect "not live" in state.
    /// Establish (or re-establish) the serial link: resolve+open the port
    /// (port-independent scan), run startup, retrying with backoff until it
    /// succeeds. Blocks until the actuator is present and ready.
    async fn connect_loop(&mut self) {
        let mut backoff = Duration::from_millis(500);
        loop {
            match self.bus.reconnect().await {
                Ok(()) => match self.startup().await {
                    Ok(()) => {
                        info!("actuator ready");
                        return;
                    }
                    Err(e) => warn!(error = %e, "startup failed; retrying"),
                },
                Err(e) => debug!(error = %e, "serial connect attempt failed; retrying"),
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }

    async fn recover(&mut self) {
        warn!("actuator link lost; attempting to reconnect serial port");
        {
            let mut st = self.state.write().await;
            st.servo_on = false;
            st.is_moving = false;
            st.actuator_connected = false;
        }
        self.connect_loop().await;
        info!("actuator link recovered");
    }

    fn emit(&self, note: rpc::Notification) {
        // Best-effort fan-out; ignore "no subscribers".
        let _ = self.notif.send(RpcMessage::notification(note));
    }

    /// Handle a reply-bearing bridge command, sending the outcome back over its
    /// oneshot. These are vendor extensions (not Handy FW4 RPC).
    async fn execute_bridge(&mut self, cmd: BridgeCommand) {
        match cmd {
            BridgeCommand::ResetAlarm { reply } => {
                let r = self.reset_alarm().await.map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            BridgeCommand::Calibrate { reply } => {
                let r = self.calibrate_to_contact().await.map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            BridgeCommand::PeckProbe { reply } => {
                let r = self.peck_probe().await.map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            BridgeCommand::RawReadRegs { addr, count, reply } => {
                let r = self
                    .bus
                    .read_holding_registers(addr, count)
                    .await
                    .map_err(|e| e.to_string());
                debug!(addr, count, ok = r.is_ok(), "raw read_holding_registers");
                let _ = reply.send(r);
            }
            BridgeCommand::RawWriteRegs { addr, data, reply } => {
                let r = self
                    .bus
                    .write_multiple_registers(addr, &data)
                    .await
                    .map_err(|e| e.to_string());
                debug!(addr, ?data, ok = r.is_ok(), "raw write_multiple_registers");
                let _ = reply.send(r);
            }
            BridgeCommand::RawWriteCoil { addr, on, reply } => {
                let r = self
                    .bus
                    .write_single_coil(addr, on)
                    .await
                    .map_err(|e| e.to_string());
                debug!(addr, on, ok = r.is_ok(), "raw write_single_coil");
                let _ = reply.send(r);
            }
            BridgeCommand::RawTestMove {
                regs,
                settle_ms,
                reply,
            } => {
                let r = async {
                    self.bus
                        .write_multiple_registers(protocol::REG_MOVE_BLOCK, &regs)
                        .await
                        .map_err(|e| format!("write: {e}"))?;
                    sleep(Duration::from_millis(settle_ms)).await;
                    self.bus
                        .read_holding_registers(
                            protocol::REG_STATUS_BLOCK,
                            protocol::STATUS_BLOCK_LEN,
                        )
                        .await
                        .map_err(|e| format!("read: {e}"))
                }
                .await;
                debug!(?regs, settle_ms, ok = r.is_ok(), "raw test_move");
                let _ = reply.send(r);
            }
            BridgeCommand::SetComfortableDepth { mm } => {
                let mut st = self.state.write().await;
                let mm = mm.clamp(0.0, st.max_depth_mm);
                st.comfortable_depth_mm = mm;
                drop(st);
                persist_comfortable_depth(mm);
                info!(
                    comfortable_depth_mm = mm,
                    "comfortable-depth set (oscillating-mode range rescaled)"
                );
            }
            BridgeCommand::SetMaxDepth { mm } => {
                let mut st = self.state.write().await;
                if st.program_running() {
                    warn!(
                        requested_mm = mm,
                        mode = ?st.mode,
                        "ignoring max-depth change while a program is running"
                    );
                } else {
                    let mm = mm.clamp(0.0, self.stroke_mm);
                    st.max_depth_mm = mm;
                    // Max depth is authoritative: pull comfortable depth down
                    // with it if it now exceeds the new ceiling (comfortable
                    // may equal max depth, just never exceed it), rather than
                    // blocking the max-depth change.
                    let comfortable_pulled_down = st.comfortable_depth_mm > mm;
                    if comfortable_pulled_down {
                        st.comfortable_depth_mm = mm;
                    }
                    let comfortable_mm = st.comfortable_depth_mm;
                    drop(st);
                    persist_max_depth(mm);
                    if comfortable_pulled_down {
                        persist_comfortable_depth(comfortable_mm);
                    }
                    info!(
                        max_depth_mm = mm,
                        comfortable_depth_mm = comfortable_mm,
                        comfortable_pulled_down,
                        "max-depth set (program range rescaled)"
                    );
                }
            }
        }
    }

    /// Run one status poll and track consecutive failures, reconnecting when a
    /// run of them indicates the link is gone.
    async fn poll_and_track(&mut self, poll_failures: &mut u32) {
        if self.poll_once().await.is_ok() {
            *poll_failures = 0;
        } else {
            *poll_failures += 1;
            if *poll_failures >= RECONNECT_AFTER_POLL_FAILURES {
                self.recover().await;
                *poll_failures = 0;
            }
        }
    }

    /// Driver task entry point. Owns the bus for its lifetime. Services both the
    /// actuator-command stream (modes/dispatcher) and the bridge-control stream
    /// (vendor commands with replies), preferring commands over the status poll.
    pub async fn run(
        mut self,
        mut cmd_rx: mpsc::Receiver<ActuatorCommand>,
        mut bridge_rx: mpsc::Receiver<BridgeCommand>,
    ) {
        // Disabled once the bridge channel closes, so its `recv()` (which then
        // returns `None` immediately) can't spin the select loop.
        let mut bridge_open = true;
        // Consecutive failed status polls; a run of these means the link is gone
        // (adapter unplugged / renumbered) and we should reconnect.
        let mut poll_failures: u32 = 0;
        // Last time the status poll actually ran. The select below is `biased`,
        // so a sustained command flood (rapid drill/game deadman pulses) would
        // otherwise starve the poll entirely — freezing AppState, including
        // `hand_switch`, so the hand-switch watcher could never observe a
        // release and motion would never stop. We force a poll when overdue.
        let mut last_poll = Instant::now();
        info!(
            poll_ms = self.poll_interval.as_millis() as u64,
            "modbus driver running"
        );
        // If the actuator isn't attached yet, wait for it (port-independent)
        // while the rest of the bridge — BLE, modes, sensors — is already up.
        if !self.bus.is_connected() {
            warn!("actuator not connected; bridge is up and will connect when it is attached");
            self.connect_loop().await;
        }
        loop {
            // Liveness guard: if the poll is overdue (commands have been
            // preempting it), run it now before servicing more commands. This
            // bounds status staleness to ~one poll interval even under a flood.
            if last_poll.elapsed() >= self.poll_interval {
                self.poll_and_track(&mut poll_failures).await;
                last_poll = Instant::now();
                continue;
            }
            tokio::select! {
                biased; // movement commands preempt the status poll
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(c) => self.execute(c).await,
                        None => {
                            info!("command channel closed; modbus driver stopping");
                            break;
                        }
                    }
                }
                bridge = bridge_rx.recv(), if bridge_open => {
                    match bridge {
                        Some(c) => self.execute_bridge(c).await,
                        // Bridge channel closing is not fatal; just stop polling it.
                        None => bridge_open = false,
                    }
                }
                _ = sleep(self.poll_interval.saturating_sub(last_poll.elapsed())) => {
                    self.poll_and_track(&mut poll_failures).await;
                    last_poll = Instant::now();
                }
            }
        }
    }
}

fn io_other(e: anyhow::Error) -> io::Error {
    io::Error::other(e.to_string())
}

/// Sidecar file holding the persisted max-depth ceiling (mm), next to the
/// binary — same convention as `device-uid` (see `main::resolve_uid`).
const MAX_DEPTH_FILE: &str = "max-depth-mm";

/// Read the persisted max-depth ceiling (mm), if any. Returns `None` when the
/// file is absent or unparseable (→ caller falls back to full stroke).
pub fn load_max_depth() -> Option<f32> {
    std::fs::read_to_string(MAX_DEPTH_FILE)
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// Persist the max-depth ceiling (mm) so it survives a reboot. Best-effort.
fn persist_max_depth(mm: f32) {
    if let Err(e) = std::fs::write(MAX_DEPTH_FILE, format!("{mm}")) {
        warn!(error = %e, "failed to persist max-depth; value is in-memory only");
    }
}

/// Sidecar file holding the persisted comfortable-depth ceiling (mm).
const COMFORTABLE_DEPTH_FILE: &str = "comfortable-depth-mm";

/// Read the persisted comfortable-depth ceiling (mm), if any. Returns `None`
/// when the file is absent or unparseable.
pub fn load_comfortable_depth() -> Option<f32> {
    std::fs::read_to_string(COMFORTABLE_DEPTH_FILE)
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// Persist the comfortable-depth ceiling (mm) so it survives a reboot. Best-effort.
fn persist_comfortable_depth(mm: f32) {
    if let Err(e) = std::fs::write(COMFORTABLE_DEPTH_FILE, format!("{mm}")) {
        warn!(error = %e, "failed to persist comfortable-depth; value is in-memory only");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// In-memory fake bus: records writes, replays canned status reads.
    #[derive(Default)]
    struct FakeBus {
        coil_writes: Vec<(u16, bool)>,
        reg_writes: Vec<(u16, Vec<u16>)>,
        status_responses: VecDeque<Vec<u16>>,
    }

    impl ModbusBus for FakeBus {
        async fn read_holding_registers(&mut self, _addr: u16, cnt: u16) -> io::Result<Vec<u16>> {
            Ok(self
                .status_responses
                .pop_front()
                .unwrap_or_else(|| vec![0u16; cnt as usize]))
        }
        async fn write_single_coil(&mut self, addr: u16, on: bool) -> io::Result<()> {
            self.coil_writes.push((addr, on));
            Ok(())
        }
        async fn write_multiple_registers(&mut self, addr: u16, data: &[u16]) -> io::Result<()> {
            self.reg_writes.push((addr, data.to_vec()));
            Ok(())
        }
    }

    fn test_cfg() -> Config {
        toml::from_str(
            r#"
            [actuator]
            variant = "12inch"
            [actuator.limits]
            default_accel_g = 0.3
        "#,
        )
        .unwrap()
    }

    fn driver(bus: FakeBus) -> (ModbusDriver<FakeBus>, Arc<RwLock<AppState>>) {
        let state = Arc::new(RwLock::new(AppState::new("uid".into(), 1)));
        let (tx, _rx) = broadcast::channel(16);
        let d = ModbusDriver::new(bus, state.clone(), tx, &test_cfg());
        (d, state)
    }

    #[tokio::test]
    async fn move_to_writes_move_block_and_clamps_position() {
        let (mut d, state) = driver(FakeBus::default());
        // Request 500 mm on a 300 mm stroke -> clamps to 300 mm (30000 in 0.01mm).
        d.move_to(500.0, 200.0, 0.3, MotionProfile::SCurve)
            .await
            .unwrap();
        let (addr, regs) = &d.bus.reg_writes[0];
        assert_eq!(*addr, protocol::REG_MOVE_BLOCK);
        let pcmd = ((regs[0] as u32) << 16 | regs[1] as u32) as i32;
        assert_eq!(pcmd, 30000); // clamped to stroke
        assert_eq!(state.try_read().unwrap().target_mm, 300.0);
        assert!(state.try_read().unwrap().is_moving);
    }

    #[tokio::test]
    async fn queued_move_to_is_kept_off_the_hard_stop_by_default() {
        let (mut d, _s) = driver(FakeBus::default());
        // No comfortable_depth_mm configured (default 0 = unset): a mode asking
        // for the literal far end of a 300 mm stroke must still be kept
        // HARD_STOP_MARGIN_MM off it, not driven straight into the hard stop.
        d.execute(ActuatorCommand::MoveTo {
            pos_mm: 300.0,
            vel_mm_s: 200.0,
            accel_g: 0.3,
            profile: MotionProfile::Trapezoid,
            soften: false,
        })
        .await;
        let (_addr, regs) = &d.bus.reg_writes[0];
        let pcmd = ((regs[0] as u32) << 16 | regs[1] as u32) as i32;
        assert_eq!(pcmd, ((300.0 - HARD_STOP_MARGIN_MM) * 100.0) as i32);
    }

    #[tokio::test]
    async fn calibration_move_push_bypasses_the_hard_stop_margin() {
        let (mut d, _s) = driver(FakeBus::default());
        // calibrate_to_contact/peck_probe call move_push directly (not through
        // execute/depth_scaled) and must be able to reach the literal end to
        // detect contact.
        d.move_push(300.0, 10.0, 0.3, 20, MotionProfile::Trapezoid)
            .await
            .unwrap();
        let (_addr, regs) = &d.bus.reg_writes[0];
        let pcmd = ((regs[0] as u32) << 16 | regs[1] as u32) as i32;
        assert_eq!(pcmd, 30000); // full 300 mm, no margin applied
    }

    #[tokio::test]
    async fn move_to_clamps_velocity_to_controller_max() {
        let (mut d, _s) = driver(FakeBus::default());
        d.move_to(100.0, 9999.0, 0.3, MotionProfile::Trapezoid)
            .await
            .unwrap();
        let (_addr, regs) = &d.bus.reg_writes[0];
        let vcmd = (regs[4] as u32) << 16 | regs[5] as u32;
        assert_eq!(vcmd, VCMD_MAX); // 9999 mm/s -> 999900 clamped to 50000
    }

    #[tokio::test]
    async fn reset_alarm_sends_ff00_then_0000() {
        let (mut d, _s) = driver(FakeBus::default());
        d.reset_alarm().await.unwrap();
        assert_eq!(
            d.bus.coil_writes,
            vec![
                (protocol::COIL_ALARM_RESET, true),
                (protocol::COIL_ALARM_RESET, false)
            ]
        );
    }

    #[tokio::test]
    async fn reset_alarm_command_triggers_alrs_edge() {
        let (mut d, _s) = driver(FakeBus::default());
        d.execute(ActuatorCommand::ResetAlarm).await;
        assert_eq!(
            d.bus.coil_writes,
            vec![
                (protocol::COIL_ALARM_RESET, true),
                (protocol::COIL_ALARM_RESET, false)
            ]
        );
    }

    #[tokio::test]
    async fn calibrate_to_contact_detects_push_and_records_origin() {
        let mut bus = FakeBus::default();
        // 1st status read (home loop): HEND set so homing completes immediately.
        let mut homed = vec![0u16; 10];
        homed[5] = protocol::Dss1::HEND.bits();
        bus.status_responses.push_back(homed);
        // 2nd status read (push poll): DSSE.PUSH set at PNOW = 5000 (= 50.00 mm).
        let mut contact = vec![0u16; 10];
        contact[1] = 5000; // PNOW lo
        contact[7] = protocol::Dsse::PUSH.bits(); // DSSE.PUSH
        bus.status_responses.push_back(contact);

        let (mut d, state) = driver(bus);
        let pos = d.calibrate_to_contact().await.unwrap();
        assert_eq!(pos, 50.0);

        let st = state.try_read().unwrap();
        assert_eq!(st.work_origin_mm, Some(50.0));
        assert!(!st.calibrating);
        // A push-motion move block was written (CTLF.PUSH set on the last move).
        let (_addr, regs) = d.bus.reg_writes.last().unwrap();
        assert_eq!(regs[8] & 1, 1);
    }

    #[tokio::test]
    async fn calibrate_to_contact_errors_when_limit_reached_without_contact() {
        let mut bus = FakeBus::default();
        let mut homed = vec![0u16; 10];
        homed[5] = protocol::Dss1::HEND.bits();
        bus.status_responses.push_back(homed);
        // Reached the search limit (PNOW = full 300 mm stroke = 30000) with
        // positioning complete (DSS1.PEND) but no push → no work-piece found.
        let mut at_limit = vec![0u16; 10];
        at_limit[1] = 30000; // PNOW lo = 30000 -> 300.00 mm
        at_limit[5] = protocol::Dss1::PEND.bits();
        bus.status_responses.push_back(at_limit);

        let (mut d, state) = driver(bus);
        let err = d.calibrate_to_contact().await.unwrap_err();
        assert!(err.to_string().contains("without contact"));
        assert_eq!(state.try_read().unwrap().work_origin_mm, None);
        assert!(!state.try_read().unwrap().calibrating);
    }

    #[tokio::test]
    async fn home_polls_until_hend_set() {
        let mut bus = FakeBus::default();
        // First status read: not homed. Second: HEND set.
        bus.status_responses.push_back(vec![0; 10]);
        let mut homed = vec![0u16; 10];
        homed[5] = protocol::Dss1::HEND.bits(); // DSS1.HEND
        bus.status_responses.push_back(homed);
        let (mut d, state) = driver(bus);
        // Speed up: home() sleeps 200ms between polls; pause/auto-advance time.
        tokio::time::pause();
        let h = tokio::spawn(async move { d.home().await });
        tokio::time::advance(Duration::from_millis(600)).await;
        // allow tasks to progress
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(600)).await;
        h.await.unwrap().unwrap();
        assert!(state.try_read().unwrap().homing_complete);
    }
}
