//! Shared internal state and the actuator command type. See SPEC §6.
//!
//! `AppState` is the single source of truth for mode and motion, guarded by an
//! `Arc<RwLock<_>>`. The dispatcher and mode tasks mutate it; transports and the
//! status poll read it. `ActuatorCommand` is the transport-agnostic instruction
//! the modes produce and the Modbus driver consumes.

use tokio::sync::oneshot;

use crate::config::MotionProfile;
use crate::rpc::{HampPlayState, HspPlayState, Point};

/// Coarse operating mode tag. Mirrors the on-device `Mode` enum for the subset
/// the bridge implements; per-mode runtime lives in dedicated fields below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Uninitialized,
    Homing,
    Idle,
    Hamp,
    Hdsp,
    Hsp,
    /// Interactive drill: servo off by default; deadman button enables servo
    /// and pushes the rod outward at a configured feed rate.
    Drill,
    /// Auto-ramp: oscillates on its own, building speed/depth over time along a
    /// curve; nudges steer intensity and an idle timeout auto-stops it.
    Ramp,
    /// Endurance games: a family of button-gated programs that measure how long
    /// the user stays in control. The active game is in `GameRuntime::kind`.
    Game,
    /// Cycle: a pattern playlist over a fixed zone; the button cycles patterns
    /// (short press) and pauses (long press).
    Cycle,
    /// Learn: teach-and-repeat — record a hand-moved motion (servo off), then
    /// loop an imitation built from simplified support points.
    Learn,
    /// Pulse: oscillation whose speed tracks a connected heart-rate sensor
    /// (speed = bpm × factor).
    Pulse,
    /// Impale: button-hold extends the rod slowly; releasing brakes and arms a
    /// retract timer (default 10 min) that drives the rod back to home.
    Impale,
    /// Plumb: fixed-speed oscillation between the work origin and a hand-set
    /// upper bound (the switch drops the servo to reposition).
    Plumb,
    /// Surge: arousal-driven oscillation — hold the switch to build intensity
    /// (faster, deeper), release to let it ebb.
    Surge,
    /// Tide: oscillation whose speed eases up while the switch is held and eases
    /// back down when released.
    Tide,
    /// Echo: each tap fires one outward-and-back stroke, stepping depth deeper;
    /// a long hold resets the depth.
    Echo,
    /// Trace: oscillation from a fixed ceiling; the switch sets a new lower
    /// (return) bound by hand.
    Trace,
    /// Tempo: tap out a rhythm to set the stroke period; a long hold stops it.
    Tempo,
}

/// DG-LAB Coyote e-stim device state (BLE central; see `src/devices/coyote.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CoyoteState {
    pub connected: bool,
    pub battery: Option<u8>,
    /// Device-reported current strength per channel (0..200).
    pub strength_a: u8,
    pub strength_b: u8,
    /// Following the rod's motion intensity (vs. manual strength).
    pub following: bool,
}

/// Hismith PiuPiu lube launcher state (BLE central; see `src/devices/piupiu.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PiuPiuState {
    pub connected: bool,
    /// Squirt trigger currently held (repeating the command every 100 ms).
    pub active: bool,
}

/// Readings from connected biosensors (BLE central; see `src/sensors/`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SensorState {
    /// Whether a heart-rate sensor is currently connected.
    pub hr_connected: bool,
    /// Whether we're actively scanning/searching for a sensor (Connect requested
    /// but not yet subscribed, or reconnecting). Cleared on Disconnect.
    pub hr_scanning: bool,
    /// Latest heart rate in BPM, if a sensor is reporting.
    pub hr_bpm: Option<u16>,
}

/// Pulse (heart-rate-reactive oscillation) runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PulseRuntime {
    pub active: bool,
    /// Speed factor: mm/s of stroke velocity per BPM.
    pub factor: f32,
    /// BPM used for the current stroke (live sensor value, or the base fallback).
    pub bpm: u16,
    /// Current stroke velocity derived from bpm × factor (clamped).
    pub velocity_mm_s: f32,
}

/// Phase of the learn (teach-and-repeat) program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LearnPhase {
    /// Servo off, rod free; waiting to start recording.
    #[default]
    Armed,
    /// Sampling the hand-moved position.
    Recording,
    /// Recording captured and simplified to support points; ready to play.
    Ready,
    /// Looping the imitation.
    Playing,
}

impl LearnPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            LearnPhase::Armed => "armed",
            LearnPhase::Recording => "recording",
            LearnPhase::Ready => "ready",
            LearnPhase::Playing => "playing",
        }
    }
}

/// Which endurance game is running. See `src/modes/games.rs` and the web manual
/// for the rules of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameKind {
    /// Climb intensity while the button is held; release to back off ("edge").
    EdgeRecover,
    /// Alternating work/rest intervals, each gated by a button press.
    Gauntlet,
    /// Banked climb: lapses drop you one checkpoint, not back to zero.
    DeadmansClimb,
    /// Hold the freely-moving rod still against random servo nudges.
    Stillness,
}

impl GameKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GameKind::EdgeRecover => "edge_recover",
            GameKind::Gauntlet => "gauntlet",
            GameKind::DeadmansClimb => "deadmans_climb",
            GameKind::Stillness => "stillness",
        }
    }

    pub fn parse(s: &str) -> Option<GameKind> {
        Some(match s {
            "edge_recover" => GameKind::EdgeRecover,
            "gauntlet" => GameKind::Gauntlet,
            "deadmans_climb" => GameKind::DeadmansClimb,
            "stillness" => GameKind::Stillness,
            _ => return None,
        })
    }
}

/// Coarse phase a game is in, surfaced to the UI for status display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    Idle,
    /// Waiting for the hardware ready signal (triple-tap the hand switch)
    /// before motion starts. `GameRuntime::level` counts taps so far.
    Armed,
    /// Actively building / working / climbing.
    Active,
    /// Backing off after an edge (Edge & Recover) or between climbs.
    Recover,
    /// Rest interval (Gauntlet) — servo free.
    Rest,
    /// Holding still (Stillness).
    Hold,
    /// Lost ground / moved too much this round.
    Slip,
    /// Reached the top of the climb (Deadman's Climb) — round over, won.
    Win,
}

impl GamePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            GamePhase::Idle => "idle",
            GamePhase::Armed => "armed",
            GamePhase::Active => "active",
            GamePhase::Recover => "recover",
            GamePhase::Rest => "rest",
            GamePhase::Hold => "hold",
            GamePhase::Slip => "slip",
            GamePhase::Win => "win",
        }
    }
}

/// Live runtime for the endurance-games subsystem.
#[derive(Debug, Clone)]
pub struct GameRuntime {
    pub active: bool,
    pub kind: Option<GameKind>,
    pub phase: GamePhase,
    /// Generic 0..1 drive level (speed/thrust intensity), for the UI meter.
    pub intensity: f32,
    /// Level / interval / checkpoint / lines-lost, interpreted per game.
    pub level: u32,
    /// Elapsed duration, in seconds, survived / held this session.
    pub duration_s: f32,
    /// Whether the deadman button is currently held.
    pub holding: bool,
}

/// Cycle (pattern-playlist) runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CycleRuntime {
    pub active: bool,
    /// Current pattern index (0..PATTERN_COUNT).
    pub pattern: u32,
    pub paused: bool,
}

/// Learn (teach-and-repeat) runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LearnRuntime {
    pub active: bool,
    pub phase: LearnPhase,
    /// Raw samples captured so far (while recording).
    pub points: u32,
    /// Support points after simplification (once Ready).
    pub waypoints: u32,
}

impl Default for GameRuntime {
    fn default() -> Self {
        GameRuntime {
            active: false,
            kind: None,
            phase: GamePhase::Idle,
            intensity: 0.0,
            level: 0,
            duration_s: 0.0,
            holding: false,
        }
    }
}

impl AppMode {
    /// Short attribute label for telemetry (`rod.mode`).
    pub fn label(self) -> &'static str {
        match self {
            AppMode::Uninitialized => "uninitialized",
            AppMode::Homing => "homing",
            AppMode::Idle => "idle",
            AppMode::Hamp => "hamp",
            AppMode::Hdsp => "hdsp",
            AppMode::Hsp => "hsp",
            AppMode::Drill => "drill",
            AppMode::Ramp => "ramp",
            AppMode::Game => "game",
            AppMode::Cycle => "cycle",
            AppMode::Learn => "learn",
            AppMode::Pulse => "pulse",
            AppMode::Impale => "impale",
            AppMode::Plumb => "plumb",
            AppMode::Surge => "surge",
            AppMode::Tide => "tide",
            AppMode::Echo => "echo",
            AppMode::Trace => "trace",
            AppMode::Tempo => "tempo",
        }
    }
}

/// Drill interactive-program runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillRuntime {
    /// Whether drill mode is active (servo may still be off between pushes).
    pub active: bool,
    /// Whether the deadman is currently held (servo on, rod moving outward).
    pub pushing: bool,
    /// Current outward feed rate in mm/s.
    pub feed_rate_mm_s: f32,
}

impl Default for DrillRuntime {
    fn default() -> Self {
        DrillRuntime {
            active: false,
            pushing: false,
            feed_rate_mm_s: 5.0,
        }
    }
}

/// Impale interactive-program runtime. The button extends the rod outward; on
/// release the rod brakes and an idle timer (`retract_after_s`) later drives it
/// back to home.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpaleRuntime {
    /// Whether impale mode is active.
    pub active: bool,
    /// Whether the rod is currently extending (button held, servo on).
    pub extending: bool,
    /// Whether the retract timer is armed (button released, servo braked).
    pub waiting: bool,
    /// Whether the rod is currently retracting back to home.
    pub retracting: bool,
    /// Current outward feed rate in mm/s.
    pub feed_rate_mm_s: f32,
    /// Hold duration (seconds) after release before the rod auto-retracts.
    pub retract_after_s: f32,
}

impl Default for ImpaleRuntime {
    fn default() -> Self {
        ImpaleRuntime {
            active: false,
            extending: false,
            waiting: false,
            retracting: false,
            feed_rate_mm_s: 5.0,
            retract_after_s: 600.0,
        }
    }
}

/// Auto-ramp interactive-program runtime. The task publishes the live derived
/// values here each stroke so the UI can show the current intensity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RampRuntime {
    /// Whether the auto-ramp program is running.
    pub active: bool,
    /// Current intensity in 0..1 (time-curve + accumulated nudges).
    pub intensity: f32,
    /// Current stroke velocity derived from `intensity` (mm/s).
    pub velocity_mm_s: f32,
    /// Current stroke zone (relative 0..1), widening with intensity.
    pub zone_min: f32,
    pub zone_max: f32,
}

/// HAMP (oscillation) runtime, mirrors the proto `HampState`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HampRuntime {
    pub running: bool,
    pub velocity: f32,
    pub direction: bool,
    /// Relative zone bounds (0..1), mapped onto `[work_origin_mm, stroke]` —
    /// 0.0 is the calibrated origin, 1.0 the far end (itself capped by the
    /// comfortable-depth ceiling; see `HampTask::stroke` and
    /// `driver::depth_scaled`).
    pub min: f32,
    pub max: f32,
    /// Reversal softness: 0 = hard/snappy (full configured accel),
    /// 1 = very soft (10 % of configured accel).
    /// Handy-originated HAMP commands carry no softness → default 0.5 (medium).
    pub softness: f32,
}

impl Default for HampRuntime {
    fn default() -> Self {
        HampRuntime {
            running: false,
            velocity: 0.0,
            direction: false,
            // Full range: the calibrated origin and the comfortable-depth
            // ceiling already bound the stroke, so no extra margin is needed.
            min: 0.0,
            max: 1.0,
            softness: 0.5,
        }
    }
}

impl HampRuntime {
    pub fn play_state(&self) -> HampPlayState {
        if self.running {
            HampPlayState::HampStateRunning
        } else {
            HampPlayState::HampStateStopped
        }
    }
}

/// HSP (script playback) runtime, mirrors the proto `HspState`.
#[derive(Debug, Clone, PartialEq)]
pub struct HspRuntime {
    pub play_state: HspPlayState,
    pub max_points: u32,
    pub current_point: i32,
    pub current_time: i32,
    pub looped: bool,
    pub playback_rate: f32,
    pub first_point_time: u32,
    pub last_point_time: u32,
    pub stream_id: u32,
    pub tail_point_stream_index: i32,
    pub tail_point_threshold: u32,
    pub pause_on_starving: bool,
}

impl Default for HspRuntime {
    fn default() -> Self {
        HspRuntime {
            play_state: HspPlayState::HspStateNotInitialized,
            max_points: 4000,
            current_point: -1,
            current_time: 0,
            looped: false,
            playback_rate: 1.0,
            first_point_time: 0,
            last_point_time: 0,
            stream_id: 0,
            tail_point_stream_index: -1,
            tail_point_threshold: 0,
            pause_on_starving: false,
        }
    }
}

/// Plumb interactive-program runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlumbRuntime {
    pub active: bool,
    /// Current upper bound for oscillation (mm); updated when the user
    /// repositions the rod with the hand switch.
    pub target_mm: f32,
    /// True while the servo is off and the user is hand-positioning the rod.
    pub handing_off: bool,
}

/// Surge interactive-program runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SurgeRuntime {
    pub active: bool,
    /// Current arousal level [0.0, 1.0]: drives both speed and stroke depth.
    pub arousal: f32,
}

/// Tide interactive-program runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TideRuntime {
    pub active: bool,
    pub speed_mm_s: f32,
    pub target_mm: f32,
}

/// Echo interactive-program runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EchoRuntime {
    pub active: bool,
    /// Absolute target depth of the next stroke (mm).
    pub current_depth_mm: f32,
    /// Number of depth steps taken since start.
    pub steps_taken: u32,
}

/// Trace interactive-program runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TraceRuntime {
    pub active: bool,
    /// Current lower (return) bound in mm; user-set via hand positioning.
    pub lower_mm: f32,
    /// True while the servo is off and the user is repositioning the rod.
    pub handing_off: bool,
}

/// Tempo interactive-program runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TempoRuntime {
    pub active: bool,
    /// Established stroke cycle period in ms; 0 = no tempo set yet.
    pub period_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub mode: AppMode,
    /// Last known / interpolated actuator position in mm.
    pub position_mm: f32,
    /// Last commanded target in mm.
    pub target_mm: f32,
    /// Active stroke zone (0..1), remaps all relative positions.
    pub slide_min: f32,
    pub slide_max: f32,

    pub homing_complete: bool,
    pub servo_on: bool,
    pub alarm_code: u16,
    pub is_moving: bool,
    pub motor_temp_c: f32,
    pub speed_mm_s: f32,
    /// Whether the Modbus serial link to the actuator is currently up. The
    /// bridge boots without it and connects when the actuator is attached.
    pub actuator_connected: bool,
    /// Normalised commanded speed (0..1): "how hard the rod is moving", set by
    /// the driver per move. External devices (e.g. the Coyote) can follow it.
    pub motion_intensity: f32,

    // ── Granular DSS1/DSSE status bits (populated by the Modbus driver) ──
    pub controller_ready: bool,  // DSS1.PWR
    pub positioning_done: bool,  // DSS1.PEND
    pub brake_released: bool,    // DSS1.BKRL
    pub push_active: bool,       // DSSE.PUSH
    pub emergency_stop: bool,    // DSS1.EMGS | DSSE.EMGP
    pub motor_voltage_low: bool, // DSSE.MPUV
    pub safety_speed: bool,      // DSS1.SFTY
    pub alarm_minor: bool,       // DSS1.ALML
    pub alarm_major: bool,       // DSS1.ALMH
    /// Hand/palm switch wired to the controller's PIO input (DIPM bit 0).
    pub hand_switch: bool,

    /// A push-to-contact calibration is currently running.
    pub calibrating: bool,
    /// Absolute position (mm) where the last contact calibration sensed the
    /// work-piece. `None` until a calibration has succeeded.
    pub work_origin_mm: Option<f32>,
    /// Depth ceiling for oscillating modes (mm): HAMP, cycle, pulse, plumb,
    /// surge, tide, trace, tempo, and the fixed-zone games (edge & recover,
    /// gauntlet, deadman's climb, stillness) all rescale `[0, stroke] → [0,
    /// comfortable_depth]` so they run between the entrance (0) and this depth
    /// — scaled, not hard-clamped. Never exceeds `max_depth_mm` (may equal it).
    /// 0.0 means "unset → full stroke". Set at boot and via
    /// `BridgeCommand::SetComfortableDepth`.
    pub comfortable_depth_mm: f32,
    /// Depth ceiling for modes that press toward or hold a single far point
    /// rather than oscillating in a fixed zone: ramp, HDSP, HSP, learn, drill,
    /// impale, echo rescale into `[0, max_depth]`
    /// the same way. 0.0 means "unset → full stroke". Set at boot (from the
    /// persisted value or stroke) and via `BridgeCommand::SetMaxDepth` — which
    /// is refused while a program is running (see `AppState::program_running`)
    /// so it can't shift a program's own range out from under it mid-run.
    /// Calibration is not scaled by either ceiling.
    pub max_depth_mm: f32,

    pub uid: String,
    pub connection_key: Option<String>,

    pub hamp: HampRuntime,
    pub hsp: HspRuntime,
    pub hsp_buffer: Vec<Point>,
    pub drill: DrillRuntime,
    pub ramp: RampRuntime,
    pub game: GameRuntime,
    pub cycle: CycleRuntime,
    pub learn: LearnRuntime,
    pub pulse: PulseRuntime,
    pub impale: ImpaleRuntime,
    pub plumb: PlumbRuntime,
    pub surge: SurgeRuntime,
    pub tide: TideRuntime,
    pub echo: EchoRuntime,
    pub trace: TraceRuntime,
    pub tempo: TempoRuntime,
    pub sensors: SensorState,
    pub coyote: CoyoteState,
    pub piupiu: PiuPiuState,

    /// Persisted "autoconnect at boot" setting for the Coyote/PiuPiu BLE
    /// devices (see `devices::load_autoconnect`/`persist_autoconnect`). Lives
    /// here — not in `CoyoteState`/`PiuPiuState` — because those get reset to
    /// `Default` on every disconnect, while this is a standing user
    /// preference independent of the current connection.
    pub coyote_autoconnect: bool,
    pub piupiu_autoconnect: bool,

    /// Server↔device clock offset in ms (for HSP `server_time` sync).
    pub clock_offset_ms: i64,

    /// Increments on every mode change (mirrors device `mode_session_id`).
    pub mode_session_id: u32,
    /// Random per-boot id (mirrors device `boot_session_id`).
    pub boot_session_id: u32,
    /// Increments on each (re)connect (mirrors device `socket_session_id`).
    pub socket_session_id: u32,
}

impl AppState {
    pub fn new(uid: String, boot_session_id: u32) -> Self {
        AppState {
            mode: AppMode::Uninitialized,
            position_mm: 0.0,
            target_mm: 0.0,
            slide_min: 0.0,
            slide_max: 1.0,
            homing_complete: false,
            servo_on: false,
            alarm_code: 0,
            is_moving: false,
            motor_temp_c: 0.0,
            speed_mm_s: 0.0,
            actuator_connected: false,
            motion_intensity: 0.0,
            controller_ready: false,
            positioning_done: false,
            brake_released: false,
            push_active: false,
            emergency_stop: false,
            motor_voltage_low: false,
            safety_speed: false,
            alarm_minor: false,
            alarm_major: false,
            hand_switch: false,
            calibrating: false,
            work_origin_mm: None,
            comfortable_depth_mm: 0.0,
            max_depth_mm: 0.0,
            uid,
            connection_key: None,
            hamp: HampRuntime::default(),
            hsp: HspRuntime::default(),
            hsp_buffer: Vec::new(),
            drill: DrillRuntime::default(),
            ramp: RampRuntime::default(),
            game: GameRuntime::default(),
            cycle: CycleRuntime::default(),
            learn: LearnRuntime::default(),
            pulse: PulseRuntime::default(),
            impale: ImpaleRuntime::default(),
            plumb: PlumbRuntime::default(),
            surge: SurgeRuntime::default(),
            tide: TideRuntime::default(),
            echo: EchoRuntime::default(),
            trace: TraceRuntime::default(),
            tempo: TempoRuntime::default(),
            sensors: SensorState::default(),
            coyote: CoyoteState::default(),
            piupiu: PiuPiuState::default(),
            coyote_autoconnect: false,
            piupiu_autoconnect: false,
            clock_offset_ms: 0,
            mode_session_id: 0,
            boot_session_id,
            socket_session_id: 0,
        }
    }

    /// Replace the mode and bump the session id when it actually changes.
    /// Returns the (possibly unchanged) session id.
    pub fn set_mode(&mut self, mode: AppMode) -> u32 {
        if self.mode != mode {
            self.mode_session_id = self.mode_session_id.wrapping_add(1);
            self.mode = mode;
        }
        self.mode_session_id
    }

    /// True once an interactive/automatic program is active. `max_depth_mm`
    /// changes are refused while this holds, so a running program's rescale
    /// target can't shift out from under it mid-run.
    pub fn program_running(&self) -> bool {
        !matches!(self.mode, AppMode::Idle | AppMode::Homing | AppMode::Uninitialized)
    }
}

/// A transport-agnostic instruction for the Modbus driver. Modes translate RPC
/// requests into these; the driver is the sole serial owner that executes them.
#[derive(Debug, Clone, PartialEq)]
pub enum ActuatorCommand {
    /// Absolute move with explicit kinematics.
    MoveTo {
        pos_mm: f32,
        vel_mm_s: f32,
        accel_g: f32,
        profile: MotionProfile,
        /// Request software jerk-limiting: the motion shaper expands this move
        /// into a ramped sub-move sequence so the launch is eased instead of a
        /// hard trapezoid corner (the controller can't do S-curve in hardware —
        /// see docs/knock-rod-protocol-notes.md §1). Only the oscillation modes
        /// (HAMP, ramp) set this; streaming/realtime moves (HSP, HDSP) and the
        /// shaper's own sub-moves leave it `false`.
        soften: bool,
    },
    /// Push-motion move: advance toward `pos_mm` but cap thrust at
    /// `push_current_pct` so the rod presses with a bounded force instead of
    /// faulting on resistance. The user's resistance shows up as `push_active`
    /// (DSSE.PUSH) with `position_mm` held short of the target. Used by the
    /// bridge's push-to-contact calibration.
    MovePush {
        pos_mm: f32,
        vel_mm_s: f32,
        accel_g: f32,
        push_current_pct: u16,
    },
    /// Run the IAI homing sequence.
    Home,
    /// Deceleration stop (does not drop the servo).
    Stop,
    /// Servo enable / disable.
    ///
    /// Disabling follows the global `release_brake_on_servo_off` policy: on a
    /// horizontal mount the holding brake is force-released so the rod can be
    /// moved by hand (drill, learn). Use [`ActuatorCommand::Park`] instead when
    /// a program comes to rest and the rod should stay put.
    ServoOn(bool),
    /// Bring the rod to rest with the servo off but the holding brake *engaged*,
    /// so it stays clamped at its current position and the motor bears no static
    /// load. Unlike `ServoOn(false)`, this never force-releases the brake,
    /// regardless of `release_brake_on_servo_off` — a vertical mount holds
    /// either way, and a horizontal mount holds only when a scene asks for it.
    /// Used by the automatic programs (ramp, pulse, cycle) at their rest points;
    /// the hand-held modes (drill, learn) keep using `ServoOn(false)`.
    Park,
    /// Clear a latched controller alarm (ALRS edge). Use after the motor has
    /// faulted into ERR — e.g. the rod was blocked past the thrust threshold.
    ResetAlarm,
}

/// Reply-bearing vendor commands carried on the dedicated bridge-control
/// channel. These are kept separate from [`ActuatorCommand`] because each owns
/// a `oneshot` reply sender (so it can't be `Clone`/`PartialEq`) and because
/// they are bridge extensions, not part of the Handy FW4 protocol.
#[derive(Debug)]
pub enum BridgeCommand {
    /// Clear a latched controller alarm; replies once the ALRS edge is sent.
    ResetAlarm {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Home, then slowly push forward at minimal thrust until contact is
    /// sensed. Replies with the contact position in mm (the work-piece origin)
    /// or an error string.
    Calibrate {
        reply: oneshot::Sender<Result<f32, String>>,
    },
    /// Raw debug: read `count` holding registers starting at `addr`.
    /// Replies with the register values (or an error string).
    RawReadRegs {
        addr: u16,
        count: u16,
        reply: oneshot::Sender<Result<Vec<u16>, String>>,
    },
    /// Raw debug: write holding registers `data` starting at `addr` (FC 0x10).
    RawWriteRegs {
        addr: u16,
        data: Vec<u16>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Raw debug: write a single coil (FC 0x05).
    RawWriteCoil {
        addr: u16,
        on: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Two-phase peck-probe: home, then step forward releasing the servo at each
    /// step to measure elastic spring-back. Locates the work-piece without any
    /// motor-thrust force. Stores the contact position in
    /// `AppState::work_origin_mm` so other commands can move relative to it.
    /// Replies with the contact position in mm or an error string.
    PeckProbe {
        reply: oneshot::Sender<Result<f32, String>>,
    },
    /// Raw debug: write the 9-register move block at 0x9900, wait `settle_ms`,
    /// then read back the 10-word status block — all atomically within the
    /// driver task so the status poll / auto-reset can't clear the alarm before
    /// we observe it. Used to sweep CTLF flag values. Replies with the status
    /// words (PNOW_hi, PNOW_lo, ALMC, …).
    RawTestMove {
        regs: [u16; 9],
        settle_ms: u64,
        reply: oneshot::Sender<Result<Vec<u16>, String>>,
    },
    /// Set the comfortable depth (mm) for oscillating modes. Clamped to
    /// `[0, max_depth]` (may equal max depth) and persisted across reboots.
    /// Fire-and-forget; always succeeds (clamped rather than rejected).
    SetComfortableDepth { mm: f32 },
    /// Set the max depth (mm) for modes that press toward or hold a single far
    /// point. Clamped to `[0, stroke]` and persisted across reboots. Max depth
    /// is authoritative: if this pulls the ceiling below the current
    /// comfortable depth, comfortable depth is lowered to fit (and persisted)
    /// rather than the max-depth change being blocked. Fire-and-forget, but
    /// silently ignored (with a log warning) while `AppState::program_running`
    /// is true.
    SetMaxDepth { mm: f32 },
}
