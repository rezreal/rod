//! Configuration model (`config.toml`). See SPEC §5.
//!
//! Everything the bridge needs at runtime is expressed here and loaded from a
//! TOML file via `serde`. Defaults match the known-working knock-rod hardware
//! (19200 8N1, etc.) so a minimal config still boots.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub actuator: Actuator,
    #[serde(default)]
    pub transports: Transports,
    #[serde(default)]
    pub ble: Ble,
    #[serde(default)]
    pub cloud: Cloud,
    #[serde(default)]
    pub debug: Debug,
    #[serde(default)]
    pub sensors: Sensors,
    #[serde(default)]
    pub devices: Devices,
}

/// Biosensor (BLE central) configuration. See `src/sensors/`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Sensors {
    #[serde(default)]
    pub heart_rate: HeartRateSensor,
}

/// External BLE actuator devices. See `src/devices/`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Devices {
    #[serde(default)]
    pub coyote: Coyote,
}

/// DG-LAB Coyote 3.0 e-stim driver config. SAFETY: `max_strength` caps output
/// well below the device max (200); start low. See `src/devices/coyote.rs`.
#[derive(Debug, Clone, Deserialize)]
pub struct Coyote {
    #[serde(default)]
    pub enable: bool,
    /// Name substring to match; empty = the default Coyote 3.0 name.
    #[serde(default)]
    pub name: String,
    /// Hard cap on per-channel strength (0..200). Keep conservative.
    #[serde(default = "default_coyote_max_strength")]
    pub max_strength: u8,
    /// Default waveform frequency (user units 10..1000).
    #[serde(default = "default_coyote_freq")]
    pub waveform_freq: u16,
    /// Default waveform intensity (0..100).
    #[serde(default = "default_coyote_intensity")]
    pub waveform_intensity: u8,
    /// Default follow scale (0..1): how much of the cap full rod motion reaches
    /// when following the program. Applied when follow mode is enabled.
    #[serde(default = "default_coyote_follow_scale")]
    pub follow_scale: f32,
}

impl Default for Coyote {
    fn default() -> Self {
        Coyote {
            enable: false,
            name: String::new(),
            max_strength: default_coyote_max_strength(),
            waveform_freq: default_coyote_freq(),
            waveform_intensity: default_coyote_intensity(),
            follow_scale: default_coyote_follow_scale(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HeartRateSensor {
    /// Connect to a heart-rate sensor over BLE (central role).
    #[serde(default)]
    pub enable: bool,
    /// Optional case-insensitive name substring to match; empty = first device
    /// advertising the Heart Rate Service.
    #[serde(default)]
    pub name: String,
}

/// Local raw-Modbus debug console (SPEC §dev). When enabled, the bridge listens
/// on a TCP socket and accepts line-based text commands (`rreg`/`wreg`/`wcoil`/
/// `testmove`/`reset-alarm`/`calibrate`/`status`) so the controller can be poked
/// directly over SSH (`nc 127.0.0.1 <port>`) without going through BLE. Bind to
/// loopback only; it grants unauthenticated raw bus access.
#[derive(Debug, Clone, Deserialize)]
pub struct Debug {
    #[serde(default)]
    pub enable: bool,
    #[serde(default = "default_debug_listen")]
    pub listen: String,
}

impl Default for Debug {
    fn default() -> Self {
        Debug {
            enable: false,
            listen: default_debug_listen(),
        }
    }
}

fn default_debug_listen() -> String {
    "127.0.0.1:7878".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Actuator {
    #[serde(default = "default_serial_device")]
    pub serial_device: String,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_slave")]
    pub modbus_slave: u8,
    /// "4inch".."12inch" — selects stroke length. See [`Variant`].
    #[serde(default = "default_variant")]
    pub variant: String,
    /// Physical homing direction; informational / for the calibration sign.
    #[serde(default = "default_home_direction")]
    pub home_direction: String,
    /// Force-release the holding brake (BKRL) whenever the servo is disabled, so
    /// the rod can be moved freely by hand (drill, learn) and spring-back isn't
    /// masked during the peck-probe. Harmless no-op on units without a brake.
    /// Disable for a vertical mount where the rod could drop under gravity.
    #[serde(default = "default_true")]
    pub release_brake_on_servo_off: bool,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub calibration: Calibration,
    #[serde(default)]
    pub peck_probe: PeckProbe,
    #[serde(default)]
    pub drill: Drill,
    #[serde(default)]
    pub ramp: Ramp,
    #[serde(default)]
    pub softening: Softening,
    #[serde(default)]
    pub games: Games,
    #[serde(default)]
    pub cycle: Cycle,
    #[serde(default)]
    pub learn: Learn,
    #[serde(default)]
    pub pulse: Pulse,
    #[serde(default)]
    pub impale: Impale,
    #[serde(default)]
    pub plumb: Plumb,
    #[serde(default)]
    pub surge: Surge,
    #[serde(default)]
    pub tide: Tide,
    #[serde(default)]
    pub echo: Echo,
    #[serde(default)]
    pub trace: Trace,
    #[serde(default)]
    pub tempo: Tempo,
}

/// "Impale" interactive-program parameters. Holding the button extends the rod
/// outward at `feed_rate_mm_s`; releasing brakes it (servo off). After
/// `retract_after_s` with no further press the rod retracts to home.
#[derive(Debug, Clone, Deserialize)]
pub struct Impale {
    /// Outward feed rate (mm/s) while the button is held. Overridable per-command.
    #[serde(default = "default_impale_feed_rate")]
    pub feed_rate_mm_s: f32,
    /// Velocity for the automatic retract back to home (mm/s).
    #[serde(default = "default_impale_retract_speed")]
    pub retract_speed_mm_s: f32,
    /// Idle time after button release before the rod auto-retracts (seconds).
    #[serde(default = "default_impale_retract_after")]
    pub retract_after_s: u64,
    /// Acceleration used during impale moves (G units). Gentle by default.
    #[serde(default = "default_impale_accel_g")]
    pub accel_g: f32,
    /// Deadman window (ms): if no button heartbeat arrives within this, the rod
    /// brakes and arms the retract timer — so a dropped release (or a connection
    /// loss mid-extension) can never leave the rod advancing unattended. The
    /// client resends the held state every ~50 ms.
    #[serde(default = "default_impale_deadman_ms")]
    pub deadman_timeout_ms: u64,
}

impl Default for Impale {
    fn default() -> Self {
        Impale {
            feed_rate_mm_s: default_impale_feed_rate(),
            retract_speed_mm_s: default_impale_retract_speed(),
            retract_after_s: default_impale_retract_after(),
            accel_g: default_impale_accel_g(),
            deadman_timeout_ms: default_impale_deadman_ms(),
        }
    }
}

/// Plumb — fixed-speed oscillation between work origin and a hand-set target.
#[derive(Debug, Clone, Deserialize)]
pub struct Plumb {
    /// Oscillation speed (mm/s). Fixed — does not vary with hand-switch state.
    #[serde(default = "default_plumb_speed")]
    pub speed_mm_s: f32,
    /// Initial upper bound above work_origin_mm (mm). Clamped to max_position_mm.
    #[serde(default = "default_plumb_depth")]
    pub default_depth_mm: f32,
    /// Acceleration used during oscillation (G).
    #[serde(default = "default_plumb_accel_g")]
    pub accel_g: f32,
}

impl Default for Plumb {
    fn default() -> Self {
        Plumb {
            speed_mm_s: default_plumb_speed(),
            default_depth_mm: default_plumb_depth(),
            accel_g: default_plumb_accel_g(),
        }
    }
}

/// Surge — arousal-driven oscillation: hold the switch to build intensity.
#[derive(Debug, Clone, Deserialize)]
pub struct Surge {
    /// Arousal rise rate (1/s) while the button is held.
    #[serde(default = "default_surge_rise_rate")]
    pub rise_rate: f32,
    /// Arousal fall rate (1/s) while the button is released.
    #[serde(default = "default_surge_fall_rate")]
    pub fall_rate: f32,
    /// Stroke speed at zero arousal (mm/s).
    #[serde(default = "default_surge_min_speed")]
    pub min_speed_mm_s: f32,
    /// Stroke speed at full arousal (mm/s).
    #[serde(default = "default_surge_max_speed")]
    pub max_speed_mm_s: f32,
    /// How far the lower (return) bound drifts outward at full arousal (mm).
    #[serde(default = "default_surge_lower_drift")]
    pub lower_drift_mm: f32,
    /// Fraction of available stroke above origin used as the ceiling at full
    /// arousal. 1.0 = full stroke.
    #[serde(default = "default_surge_max_depth_pct")]
    pub max_depth_pct: f32,
    /// Acceleration used for all surge strokes (G).
    #[serde(default = "default_surge_accel_g")]
    pub accel_g: f32,
}

impl Default for Surge {
    fn default() -> Self {
        Surge {
            rise_rate: default_surge_rise_rate(),
            fall_rate: default_surge_fall_rate(),
            min_speed_mm_s: default_surge_min_speed(),
            max_speed_mm_s: default_surge_max_speed(),
            lower_drift_mm: default_surge_lower_drift(),
            max_depth_pct: default_surge_max_depth_pct(),
            accel_g: default_surge_accel_g(),
        }
    }
}

/// Tide — oscillation whose speed eases up/down while the switch is held/released.
#[derive(Debug, Clone, Deserialize)]
pub struct Tide {
    #[serde(default = "default_tide_min_speed")]
    pub min_speed_mm_s: f32,
    #[serde(default = "default_tide_max_speed")]
    pub max_speed_mm_s: f32,
    #[serde(default = "default_tide_depth")]
    pub default_depth_mm: f32,
    /// How fast speed changes (mm/s per second).
    #[serde(default = "default_tide_adjust_rate")]
    pub speed_adjust_rate: f32,
    #[serde(default = "default_tide_accel_g")]
    pub accel_g: f32,
}

impl Default for Tide {
    fn default() -> Self {
        Tide {
            min_speed_mm_s: default_tide_min_speed(),
            max_speed_mm_s: default_tide_max_speed(),
            default_depth_mm: default_tide_depth(),
            speed_adjust_rate: default_tide_adjust_rate(),
            accel_g: default_tide_accel_g(),
        }
    }
}

/// Echo — tap-driven depth-stepping oscillation; long hold resets depth.
#[derive(Debug, Clone, Deserialize)]
pub struct Echo {
    /// Depth of the first stroke above work_origin_mm (mm).
    #[serde(default = "default_echo_start_depth")]
    pub start_depth_mm: f32,
    /// Depth increment per tap (mm).
    #[serde(default = "default_echo_step")]
    pub step_mm: f32,
    /// Extra depth limit above work_origin_mm; 0 = use max_position_mm.
    #[serde(default)]
    pub max_extra_depth_mm: f32,
    #[serde(default = "default_echo_speed")]
    pub speed_mm_s: f32,
    #[serde(default = "default_echo_accel_g")]
    pub accel_g: f32,
    /// How long to hold the button to trigger a depth reset (ms).
    #[serde(default = "default_echo_reset_hold_ms")]
    pub reset_hold_ms: u64,
}

impl Default for Echo {
    fn default() -> Self {
        Echo {
            start_depth_mm: default_echo_start_depth(),
            step_mm: default_echo_step(),
            max_extra_depth_mm: 0.0,
            speed_mm_s: default_echo_speed(),
            accel_g: default_echo_accel_g(),
            reset_hold_ms: default_echo_reset_hold_ms(),
        }
    }
}

/// Trace — oscillation from a fixed ceiling; the hand switch sets a new lower bound.
#[derive(Debug, Clone, Deserialize)]
pub struct Trace {
    /// Depth above work_origin_mm for the fixed ceiling; 0 = use max_position_mm.
    #[serde(default)]
    pub ceiling_depth_mm: f32,
    /// Initial stroke length from ceiling (sets the starting lower bound).
    #[serde(default = "default_trace_depth")]
    pub default_depth_mm: f32,
    #[serde(default = "default_trace_speed")]
    pub speed_mm_s: f32,
    #[serde(default = "default_trace_accel_g")]
    pub accel_g: f32,
}

impl Default for Trace {
    fn default() -> Self {
        Trace {
            ceiling_depth_mm: 0.0,
            default_depth_mm: default_trace_depth(),
            speed_mm_s: default_trace_speed(),
            accel_g: default_trace_accel_g(),
        }
    }
}

/// Tempo — rhythm-tapped oscillation: tap out a beat, hold to stop.
#[derive(Debug, Clone, Deserialize)]
pub struct Tempo {
    /// Minimum stroke cycle period (ms) — caps the fastest tapped tempo.
    #[serde(default = "default_tempo_min_period")]
    pub min_period_ms: u64,
    /// Maximum stroke cycle period (ms) — caps the slowest tapped tempo.
    #[serde(default = "default_tempo_max_period")]
    pub max_period_ms: u64,
    #[serde(default = "default_tempo_depth")]
    pub depth_mm: f32,
    #[serde(default = "default_tempo_accel_g")]
    pub accel_g: f32,
    /// Hold duration to stop oscillation (ms).
    #[serde(default = "default_tempo_stop_hold_ms")]
    pub stop_hold_ms: u64,
    /// Auto-stop after this many periods without a new tap.
    #[serde(default = "default_tempo_timeout_periods")]
    pub timeout_periods: f32,
}

impl Default for Tempo {
    fn default() -> Self {
        Tempo {
            min_period_ms: default_tempo_min_period(),
            max_period_ms: default_tempo_max_period(),
            depth_mm: default_tempo_depth(),
            accel_g: default_tempo_accel_g(),
            stop_hold_ms: default_tempo_stop_hold_ms(),
            timeout_periods: default_tempo_timeout_periods(),
        }
    }
}

/// Pulse (heart-rate-reactive oscillation) parameters. Stroke velocity is
/// `bpm × factor`, clamped to [min, max]. See `src/modes/pulse.rs`.
#[derive(Debug, Clone, Deserialize)]
pub struct Pulse {
    /// Default speed factor: mm/s of stroke velocity per BPM.
    #[serde(default = "default_pulse_factor")]
    pub default_factor: f32,
    #[serde(default = "default_pulse_min_velocity")]
    pub min_velocity_mm_s: f32,
    #[serde(default = "default_pulse_max_velocity")]
    pub max_velocity_mm_s: f32,
    #[serde(default = "default_pulse_zone_min")]
    pub zone_min: f32,
    #[serde(default = "default_pulse_zone_max")]
    pub zone_max: f32,
    #[serde(default = "default_pulse_accel_g")]
    pub accel_g: f32,
    /// BPM used when no sensor is connected, so the program still runs.
    #[serde(default = "default_pulse_base_bpm")]
    pub base_bpm: u16,
}

impl Default for Pulse {
    fn default() -> Self {
        Pulse {
            default_factor: default_pulse_factor(),
            min_velocity_mm_s: default_pulse_min_velocity(),
            max_velocity_mm_s: default_pulse_max_velocity(),
            zone_min: default_pulse_zone_min(),
            zone_max: default_pulse_zone_max(),
            accel_g: default_pulse_accel_g(),
            base_bpm: default_pulse_base_bpm(),
        }
    }
}

/// Learn (teach-and-repeat) parameters. See `src/modes/learn.rs`.
#[derive(Debug, Clone, Deserialize)]
pub struct Learn {
    /// Position sampling interval while recording (ms).
    #[serde(default = "default_learn_sample_ms")]
    pub sample_ms: u64,
    /// Maximum recording length (s); samples past this are dropped.
    #[serde(default = "default_learn_max_record_s")]
    pub max_record_s: f32,
    /// Simplification tolerance (mm): the max deviation a dropped sample may
    /// have from the line between kept support points.
    #[serde(default = "default_learn_epsilon_mm")]
    pub simplify_epsilon_mm: f32,
    /// Cap on the number of support points.
    #[serde(default = "default_learn_max_waypoints")]
    pub max_waypoints: usize,
    /// Velocity cap during playback (mm/s).
    #[serde(default = "default_learn_max_velocity")]
    pub max_velocity_mm_s: f32,
    /// Acceleration for playback moves (G).
    #[serde(default = "default_learn_accel_g")]
    pub accel_g: f32,
    /// Time for the loop-closing segment (last support point → first), ms.
    #[serde(default = "default_learn_loop_gap_ms")]
    pub loop_gap_ms: u64,
}

impl Default for Learn {
    fn default() -> Self {
        Learn {
            sample_ms: default_learn_sample_ms(),
            max_record_s: default_learn_max_record_s(),
            simplify_epsilon_mm: default_learn_epsilon_mm(),
            max_waypoints: default_learn_max_waypoints(),
            max_velocity_mm_s: default_learn_max_velocity(),
            accel_g: default_learn_accel_g(),
            loop_gap_ms: default_learn_loop_gap_ms(),
        }
    }
}

/// Cycle (pattern-playlist) parameters. All patterns play over the same zone;
/// they differ in speed and shape (see `src/modes/cycle.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct Cycle {
    /// Relative stroke zone (0..1) shared by every pattern.
    #[serde(default = "default_cycle_zone_min")]
    pub zone_min: f32,
    #[serde(default = "default_cycle_zone_max")]
    pub zone_max: f32,
    /// Velocity cap for the sampled point-to-point moves (mm/s).
    #[serde(default = "default_cycle_max_velocity")]
    pub max_velocity_mm_s: f32,
    /// Acceleration for pattern moves (G).
    #[serde(default = "default_cycle_accel_g")]
    pub accel_g: f32,
    /// Waveform sampling interval (ms). The bus floor is ~80 ms.
    #[serde(default = "default_cycle_tick_ms")]
    pub tick_ms: u64,
    /// Hold this long for a press to count as a long press (pause toggle), ms.
    #[serde(default = "default_cycle_long_press_ms")]
    pub long_press_ms: u64,
}

impl Default for Cycle {
    fn default() -> Self {
        Cycle {
            zone_min: default_cycle_zone_min(),
            zone_max: default_cycle_zone_max(),
            max_velocity_mm_s: default_cycle_max_velocity(),
            accel_g: default_cycle_accel_g(),
            tick_ms: default_cycle_tick_ms(),
            long_press_ms: default_cycle_long_press_ms(),
        }
    }
}

/// Endurance-games parameters (see `src/modes/games.rs` and the web manual).
/// One flat block of knobs shared across the five games; defaults are gentle.
#[derive(Debug, Clone, Deserialize)]
pub struct Games {
    /// Relative stroke zone (0..1) used by the oscillation games.
    #[serde(default = "default_game_zone_min")]
    pub zone_min: f32,
    #[serde(default = "default_game_zone_max")]
    pub zone_max: f32,
    /// Velocity range mapped from intensity 0..1 (mm/s).
    #[serde(default = "default_game_min_velocity")]
    pub min_velocity_mm_s: f32,
    #[serde(default = "default_game_max_velocity")]
    pub max_velocity_mm_s: f32,
    /// Acceleration for game strokes (G).
    #[serde(default = "default_game_accel_g")]
    pub accel_g: f32,
    /// Deadman window (ms): no button heartbeat within this → treated as released.
    #[serde(default = "default_game_deadman_ms")]
    pub deadman_timeout_ms: u64,
    /// Internal update tick (ms) for scoring / intensity integration.
    #[serde(default = "default_game_tick_ms")]
    pub tick_ms: u64,

    // ── Edge & Recover ──
    /// Time to climb from 0 to full intensity while held (s).
    #[serde(default = "default_game_climb_s")]
    pub edge_climb_s: f32,
    /// How fast intensity falls during recovery (fraction/s).
    #[serde(default = "default_game_backoff_rate")]
    pub edge_backoff_rate: f32,

    // ── Hold the Line ──
    /// Push thrust at the start and end of the ramp (percent of rated thrust).
    #[serde(default = "default_game_push_start_pct")]
    pub hold_push_start_pct: u16,
    #[serde(default = "default_game_push_max_pct")]
    pub hold_push_max_pct: u16,
    /// Time for thrust to climb from start to max (s).
    #[serde(default = "default_game_push_ramp_s")]
    pub hold_push_ramp_s: f32,
    /// How far the rod may advance past the line before a "ground lost" (mm).
    #[serde(default = "default_game_line_advance_mm")]
    pub hold_line_advance_mm: f32,
    /// Push approach velocity (mm/s) — gentle.
    #[serde(default = "default_game_push_velocity")]
    pub hold_push_velocity_mm_s: f32,

    // ── Gauntlet ──
    /// First work interval length (s); each completed round adds `work_growth_s`.
    #[serde(default = "default_game_work_s")]
    pub gauntlet_work_s: f32,
    #[serde(default = "default_game_work_growth_s")]
    pub gauntlet_work_growth_s: f32,
    /// Rest interval length (s) and how long you have to signal ready.
    #[serde(default = "default_game_rest_s")]
    pub gauntlet_rest_s: f32,

    // ── Deadman's Climb ──
    /// Number of banked checkpoints between 0 and full intensity.
    #[serde(default = "default_game_checkpoints")]
    pub climb_checkpoints: u32,
    /// Time to climb across the whole range while held (s).
    #[serde(default = "default_game_climb_total_s")]
    pub climb_total_s: f32,

    // ── Stillness ──
    /// Allowed deviation from the round's center before it counts as a move (mm).
    #[serde(default = "default_game_still_tol_mm")]
    pub stillness_tolerance_mm: f32,
    /// Lives to start a round with; a round ends when the last one is spent.
    #[serde(default = "default_game_still_lives")]
    pub stillness_lives: u32,
    /// Size of the micro-vibration feedback pulse (mm).
    #[serde(default = "default_game_still_vibration_mm")]
    pub stillness_vibration_mm: f32,
    /// Minimum gap between vibration/life-loss events, even while held past
    /// tolerance (ms) — stops one sustained drift from draining every life at once.
    #[serde(default = "default_game_still_debounce_ms")]
    pub stillness_debounce_ms: u64,
}

impl Default for Games {
    fn default() -> Self {
        Games {
            zone_min: default_game_zone_min(),
            zone_max: default_game_zone_max(),
            min_velocity_mm_s: default_game_min_velocity(),
            max_velocity_mm_s: default_game_max_velocity(),
            accel_g: default_game_accel_g(),
            deadman_timeout_ms: default_game_deadman_ms(),
            tick_ms: default_game_tick_ms(),
            edge_climb_s: default_game_climb_s(),
            edge_backoff_rate: default_game_backoff_rate(),
            hold_push_start_pct: default_game_push_start_pct(),
            hold_push_max_pct: default_game_push_max_pct(),
            hold_push_ramp_s: default_game_push_ramp_s(),
            hold_line_advance_mm: default_game_line_advance_mm(),
            hold_push_velocity_mm_s: default_game_push_velocity(),
            gauntlet_work_s: default_game_work_s(),
            gauntlet_work_growth_s: default_game_work_growth_s(),
            gauntlet_rest_s: default_game_rest_s(),
            climb_checkpoints: default_game_checkpoints(),
            climb_total_s: default_game_climb_total_s(),
            stillness_tolerance_mm: default_game_still_tol_mm(),
            stillness_lives: default_game_still_lives(),
            stillness_vibration_mm: default_game_still_vibration_mm(),
            stillness_debounce_ms: default_game_still_debounce_ms(),
        }
    }
}

/// Software motion-softening (jerk-limiting) parameters. The IAI controller only
/// does trapezoidal moves; when `enable` is set, the oscillation modes (HAMP,
/// ramp) flag their strokes for software shaping and the motion shaper expands
/// each into a short velocity-ramped sub-move sequence, easing the launch (see
/// `src/shaper.rs`). Off by default — leaving it off preserves the plain
/// single-move behaviour.
#[derive(Debug, Clone, Deserialize)]
pub struct Softening {
    /// Master switch. When false, strokes are issued as a single move.
    #[serde(default)]
    pub enable: bool,
    /// Interval between sub-moves (ms). Bounded below by the Modbus cadence;
    /// going much under ~30 ms just floods the bus.
    #[serde(default = "default_soften_step_ms")]
    pub step_ms: u64,
    /// Total launch-ramp duration (ms). Velocity climbs from
    /// `start_velocity_frac`·v to full v over this window.
    #[serde(default = "default_soften_ramp_ms")]
    pub ramp_ms: u64,
    /// Initial fraction of the target velocity for the first sub-move (0..1).
    #[serde(default = "default_soften_start_frac")]
    pub start_velocity_frac: f32,
    /// Ease-in exponent for the velocity ramp (1 = linear, >1 = gentler start).
    #[serde(default = "default_soften_curve_exp")]
    pub curve_exp: f32,
}

impl Default for Softening {
    fn default() -> Self {
        Softening {
            enable: false,
            step_ms: default_soften_step_ms(),
            ramp_ms: default_soften_ramp_ms(),
            start_velocity_frac: default_soften_start_frac(),
            curve_exp: default_soften_curve_exp(),
        }
    }
}

/// "Drill" interactive-program parameters. The servo is normally off so the
/// rod can be moved freely; holding the deadman button enables the servo and
/// advances the rod outward at a steady feed rate.
#[derive(Debug, Clone, Deserialize)]
pub struct Drill {
    /// Outward feed rate (mm/s). Overridable per-command.
    #[serde(default = "default_drill_feed_rate")]
    pub default_feed_rate_mm_s: f32,
    /// Deadman window (ms). If no push arrives within this window the servo is
    /// released. Must be clearly longer than the push interval (10–20 ms).
    #[serde(default = "default_drill_deadman_ms")]
    pub deadman_timeout_ms: u64,
    /// Acceleration used during a drill push (G units). Gentle default to
    /// avoid slamming the work-piece on the first press.
    #[serde(default = "default_drill_accel_g")]
    pub accel_g: f32,
}

impl Default for Drill {
    fn default() -> Self {
        Drill {
            default_feed_rate_mm_s: default_drill_feed_rate(),
            deadman_timeout_ms: default_drill_deadman_ms(),
            accel_g: default_drill_accel_g(),
        }
    }
}

/// "Ramp" (auto-ramp) interactive-program parameters. The rod oscillates on its
/// own, building both speed and stroke length from a gentle start to a peak over
/// `ramp_duration_s`, then plateaus. Nudges steer the intensity; with no input
/// for `idle_timeout_s` the program auto-stops.
#[derive(Debug, Clone, Deserialize)]
pub struct Ramp {
    /// Stroke velocity at intensity 0 (mm/s).
    #[serde(default = "default_ramp_min_velocity")]
    pub min_velocity_mm_s: f32,
    /// Stroke velocity at intensity 1 (mm/s). Clamped to the actuator limit.
    #[serde(default = "default_ramp_max_velocity")]
    pub max_velocity_mm_s: f32,
    /// Time to climb from intensity 0 to 1 before any nudges (seconds).
    #[serde(default = "default_ramp_duration")]
    pub ramp_duration_s: f32,
    /// Auto-stop after this long with no Start/Nudge input (seconds).
    #[serde(default = "default_ramp_idle_timeout")]
    pub idle_timeout_s: f32,
    /// Relative lower bound (0..1) of the full-intensity stroke zone.
    #[serde(default = "default_ramp_zone_min")]
    pub zone_min: f32,
    /// Relative upper bound (0..1) of the full-intensity stroke zone.
    #[serde(default = "default_ramp_zone_max")]
    pub zone_max: f32,
    /// Fraction of the full span used at intensity 0 (short but non-zero early
    /// strokes).
    #[serde(default = "default_ramp_min_span_frac")]
    pub min_span_frac: f32,
    /// Ease-in exponent for the time→intensity curve (1 = linear, >1 = slow
    /// start).
    #[serde(default = "default_ramp_curve_exp")]
    pub curve_exp: f32,
    /// Acceleration used for ramp strokes (G units).
    #[serde(default = "default_ramp_accel_g")]
    pub accel_g: f32,
}

impl Default for Ramp {
    fn default() -> Self {
        Ramp {
            min_velocity_mm_s: default_ramp_min_velocity(),
            max_velocity_mm_s: default_ramp_max_velocity(),
            ramp_duration_s: default_ramp_duration(),
            idle_timeout_s: default_ramp_idle_timeout(),
            zone_min: default_ramp_zone_min(),
            zone_max: default_ramp_zone_max(),
            min_span_frac: default_ramp_min_span_frac(),
            curve_exp: default_ramp_curve_exp(),
            accel_g: default_ramp_accel_g(),
        }
    }
}

/// Peck-probe parameters: step forward, momentarily release the servo, and
/// measure elastic spring-back to locate the work-piece without motor force.
/// Two-phase: coarse scan brackets the contact zone, fine scan pins it down.
#[derive(Debug, Clone, Deserialize)]
pub struct PeckProbe {
    /// Step size for the initial coarse scan (mm).
    #[serde(default = "default_peck_coarse_step")]
    pub coarse_step_mm: f32,
    /// Step size for the fine refinement scan (mm).
    #[serde(default = "default_peck_fine_step")]
    pub fine_step_mm: f32,
    /// How far before the coarse hit to start the fine scan (mm).
    #[serde(default = "default_peck_fine_back")]
    pub fine_back_mm: f32,
    /// Stepping velocity for both phases (mm/s).
    #[serde(default = "default_peck_move_vel")]
    pub move_velocity_mm_s: f32,
    /// How long to hold the servo off at each step (ms).
    #[serde(default = "default_peck_release_ms")]
    pub release_ms: u64,
    /// Spring-back distance that declares contact (mm).
    #[serde(default = "default_peck_threshold")]
    pub springback_threshold_mm: f32,
    /// Forward search limit (mm). 0 → full stroke.
    #[serde(default)]
    pub max_travel_mm: f32,
    /// Return-to-home velocity after probing (mm/s).
    #[serde(default = "default_peck_return_vel")]
    pub return_velocity_mm_s: f32,
}

impl Default for PeckProbe {
    fn default() -> Self {
        PeckProbe {
            coarse_step_mm: default_peck_coarse_step(),
            fine_step_mm: default_peck_fine_step(),
            fine_back_mm: default_peck_fine_back(),
            move_velocity_mm_s: default_peck_move_vel(),
            release_ms: default_peck_release_ms(),
            springback_threshold_mm: default_peck_threshold(),
            max_travel_mm: 0.0,
            return_velocity_mm_s: default_peck_return_vel(),
        }
    }
}

/// Push-to-contact ("touch") calibration parameters. Defaults are intentionally
/// gentle: crawl forward and press at minimal thrust so contact is sensed
/// softly without driving the rod into a fault.
#[derive(Debug, Clone, Deserialize)]
pub struct Calibration {
    /// Crawl speed while searching for contact (mm/s).
    #[serde(default = "default_touch_velocity")]
    pub touch_velocity_mm_s: f32,
    /// Push-current limit while pressing (percent of rated thrust). Lower =
    /// softer contact. The controller enforces its own minimum.
    #[serde(default = "default_push_current")]
    pub push_current_pct: u16,
    /// Acceleration for the crawl (0.01 G units come from ×100 of this).
    #[serde(default = "default_touch_accel")]
    pub touch_accel_g: f32,
    /// How far forward of home to search before giving up (mm). 0 → full stroke.
    #[serde(default)]
    pub max_travel_mm: f32,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            touch_velocity_mm_s: default_touch_velocity(),
            push_current_pct: default_push_current(),
            touch_accel_g: default_touch_accel(),
            max_travel_mm: 0.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub min_position_mm: f32,
    /// Hard upper soft-limit in mm. Defaults to the selected variant's stroke
    /// (resolved in [`Config::stroke_mm`]) when left at 0.
    #[serde(default)]
    pub max_position_mm: f32,
    #[serde(default = "default_max_velocity")]
    pub max_velocity_mm_s: f32,
    #[serde(default = "default_accel_g")]
    pub default_accel_g: f32,
    #[serde(default = "default_profile")]
    pub default_motion_profile: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Transports {
    #[serde(default = "default_true")]
    pub enable_ble: bool,
    #[serde(default)]
    pub enable_cloud: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ble {
    #[serde(default = "default_hw_model")]
    pub hw_model: u8,
    /// 12-hex-char device UID. Empty means "generate + persist on first boot".
    #[serde(default)]
    pub uid: String,
    #[serde(default = "default_adapter")]
    pub adapter: String,
    /// Handy cloud "connection key" returned by `RequestConnectionKeyGet`.
    /// Empty → reported as blank (device looks un-provisioned). A real Handy
    /// gets this from Handyverse; set a placeholder to satisfy clients that
    /// expect a non-empty key.
    #[serde(default)]
    pub connection_key: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Cloud {
    #[serde(default = "default_server_env")]
    pub server_env: String,
    #[serde(default)]
    pub custom_url: String,
}

/// Stroke length per ShockRod variant (mm). Matches knock-rod's `ShockRodSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    FourInch = 100,
    SixInch = 150,
    EightInch = 200,
    TenInch = 250,
    TwelveInch = 300,
}

impl Variant {
    pub fn stroke_mm(self) -> f32 {
        self as i32 as f32
    }
}

/// Motion profile -> IAI CTLF MOD bits (see SPEC §7.1 / §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionProfile {
    Trapezoid,
    SCurve,
    Filter,
}

impl Config {
    /// Load and validate config from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.as_ref().display()))?;
        let cfg: Config = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> anyhow::Result<()> {
        self.variant()?;
        let profile = self.motion_profile()?;
        // The IAI numerical-value movement command (0x9900 block) only supports
        // trapezoid: CTLF defines bits 0–3 (push / direction / incremental) and
        // has no S-motion/filter bit, so s_curve/filter silently fall back to
        // trapezoid (see docs/knock-rod-protocol-notes.md §1). Warn so the
        // config isn't misread as actually shaping the acceleration.
        if !matches!(profile, MotionProfile::Trapezoid) {
            tracing::warn!(
                profile = %self.actuator.limits.default_motion_profile,
                "default_motion_profile is not supported by this controller and \
                 falls back to trapezoid; for smoother motion enable \
                 [actuator.softening] (software jerk-limiting) instead"
            );
        }
        if self.actuator.baud_rate == 0 {
            anyhow::bail!("baud_rate must be > 0");
        }
        Ok(())
    }

    pub fn variant(&self) -> anyhow::Result<Variant> {
        Ok(match self.actuator.variant.as_str() {
            "4inch" => Variant::FourInch,
            "6inch" => Variant::SixInch,
            "8inch" => Variant::EightInch,
            "10inch" => Variant::TenInch,
            "12inch" => Variant::TwelveInch,
            other => anyhow::bail!("unknown actuator.variant {other:?} (expected 4/6/8/10/12inch)"),
        })
    }

    pub fn motion_profile(&self) -> anyhow::Result<MotionProfile> {
        Ok(match self.actuator.limits.default_motion_profile.as_str() {
            "trapezoid" => MotionProfile::Trapezoid,
            "s_curve" => MotionProfile::SCurve,
            "filter" => MotionProfile::Filter,
            other => anyhow::bail!("unknown default_motion_profile {other:?}"),
        })
    }

    /// Effective stroke length in mm (from the variant).
    pub fn stroke_mm(&self) -> f32 {
        self.variant().map(Variant::stroke_mm).unwrap_or(300.0)
    }

    /// Effective max position soft-limit in mm: the configured value, or the
    /// variant stroke when unset (0).
    pub fn max_position_mm(&self) -> f32 {
        let m = self.actuator.limits.max_position_mm;
        if m > 0.0 {
            m
        } else {
            self.stroke_mm()
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            min_position_mm: 0.0,
            max_position_mm: 0.0,
            max_velocity_mm_s: default_max_velocity(),
            default_accel_g: default_accel_g(),
            default_motion_profile: default_profile(),
        }
    }
}

impl Default for Ble {
    fn default() -> Self {
        Ble {
            hw_model: default_hw_model(),
            uid: String::new(),
            adapter: default_adapter(),
            connection_key: String::new(),
        }
    }
}

fn default_serial_device() -> String {
    "/dev/ttyUSB0".into()
}
fn default_baud_rate() -> u32 {
    19200
}
fn default_slave() -> u8 {
    1
}
fn default_variant() -> String {
    "12inch".into()
}
fn default_home_direction() -> String {
    "negative".into()
}
fn default_drill_feed_rate() -> f32 {
    5.0
}
fn default_drill_deadman_ms() -> u64 {
    50
}
fn default_drill_accel_g() -> f32 {
    0.05
}

fn default_impale_feed_rate() -> f32 {
    3.0
}
fn default_impale_retract_speed() -> f32 {
    20.0
}
fn default_impale_retract_after() -> u64 {
    600
}
fn default_impale_accel_g() -> f32 {
    0.05
}
fn default_impale_deadman_ms() -> u64 {
    150
}

fn default_plumb_speed() -> f32 {
    40.0
}
fn default_plumb_depth() -> f32 {
    80.0
}
fn default_plumb_accel_g() -> f32 {
    0.15
}
fn default_surge_rise_rate() -> f32 {
    0.15
}
fn default_surge_fall_rate() -> f32 {
    0.08
}
fn default_surge_min_speed() -> f32 {
    5.0
}
fn default_surge_max_speed() -> f32 {
    80.0
}
fn default_surge_lower_drift() -> f32 {
    5.0
}
fn default_surge_max_depth_pct() -> f32 {
    1.0
}
fn default_surge_accel_g() -> f32 {
    0.1
}
fn default_tide_min_speed() -> f32 {
    10.0
}
fn default_tide_max_speed() -> f32 {
    80.0
}
fn default_tide_depth() -> f32 {
    80.0
}
fn default_tide_adjust_rate() -> f32 {
    20.0
}
fn default_tide_accel_g() -> f32 {
    0.15
}
fn default_echo_start_depth() -> f32 {
    20.0
}
fn default_echo_step() -> f32 {
    10.0
}
fn default_echo_speed() -> f32 {
    50.0
}
fn default_echo_accel_g() -> f32 {
    0.2
}
fn default_echo_reset_hold_ms() -> u64 {
    2000
}
fn default_trace_depth() -> f32 {
    80.0
}
fn default_trace_speed() -> f32 {
    40.0
}
fn default_trace_accel_g() -> f32 {
    0.15
}
fn default_tempo_min_period() -> u64 {
    400
}
fn default_tempo_max_period() -> u64 {
    4000
}
fn default_tempo_depth() -> f32 {
    80.0
}
fn default_tempo_accel_g() -> f32 {
    0.2
}
fn default_tempo_stop_hold_ms() -> u64 {
    1000
}
fn default_tempo_timeout_periods() -> f32 {
    2.0
}

fn default_ramp_min_velocity() -> f32 {
    40.0
}
fn default_ramp_max_velocity() -> f32 {
    250.0
}
fn default_ramp_duration() -> f32 {
    120.0
}
fn default_ramp_idle_timeout() -> f32 {
    180.0
}
fn default_ramp_zone_min() -> f32 {
    0.1
}
fn default_ramp_zone_max() -> f32 {
    0.9
}
fn default_ramp_min_span_frac() -> f32 {
    0.2
}
fn default_ramp_curve_exp() -> f32 {
    2.0
}
fn default_ramp_accel_g() -> f32 {
    0.2
}

fn default_soften_step_ms() -> u64 {
    40
}
fn default_soften_ramp_ms() -> u64 {
    120
}
fn default_soften_start_frac() -> f32 {
    0.3
}
fn default_soften_curve_exp() -> f32 {
    2.0
}

fn default_game_zone_min() -> f32 {
    0.1
}
fn default_game_zone_max() -> f32 {
    0.9
}
fn default_game_min_velocity() -> f32 {
    40.0
}
fn default_game_max_velocity() -> f32 {
    250.0
}
fn default_game_accel_g() -> f32 {
    0.2
}
fn default_game_deadman_ms() -> u64 {
    150
}
fn default_game_tick_ms() -> u64 {
    100
}
fn default_game_climb_s() -> f32 {
    45.0
}
fn default_game_backoff_rate() -> f32 {
    0.6
}
fn default_game_push_start_pct() -> u16 {
    10
}
fn default_game_push_max_pct() -> u16 {
    40
}
fn default_game_push_ramp_s() -> f32 {
    90.0
}
fn default_game_line_advance_mm() -> f32 {
    15.0
}
fn default_game_push_velocity() -> f32 {
    8.0
}
fn default_game_work_s() -> f32 {
    15.0
}
fn default_game_work_growth_s() -> f32 {
    5.0
}
fn default_game_rest_s() -> f32 {
    12.0
}
fn default_game_checkpoints() -> u32 {
    5
}
fn default_game_climb_total_s() -> f32 {
    90.0
}
fn default_game_still_tol_mm() -> f32 {
    12.0
}
fn default_game_still_lives() -> u32 {
    5
}
fn default_game_still_vibration_mm() -> f32 {
    3.0
}
fn default_game_still_debounce_ms() -> u64 {
    2000
}

fn default_cycle_zone_min() -> f32 {
    0.1
}
fn default_cycle_zone_max() -> f32 {
    0.9
}
fn default_cycle_max_velocity() -> f32 {
    300.0
}
fn default_cycle_accel_g() -> f32 {
    0.3
}
fn default_cycle_tick_ms() -> u64 {
    80
}
fn default_cycle_long_press_ms() -> u64 {
    2000
}

fn default_learn_sample_ms() -> u64 {
    80
}
fn default_learn_max_record_s() -> f32 {
    60.0
}
fn default_learn_epsilon_mm() -> f32 {
    2.0
}
fn default_learn_max_waypoints() -> usize {
    200
}
fn default_learn_max_velocity() -> f32 {
    300.0
}
fn default_learn_accel_g() -> f32 {
    0.3
}
fn default_learn_loop_gap_ms() -> u64 {
    600
}

fn default_pulse_factor() -> f32 {
    2.0
}
fn default_pulse_min_velocity() -> f32 {
    30.0
}
fn default_pulse_max_velocity() -> f32 {
    300.0
}
fn default_pulse_zone_min() -> f32 {
    0.1
}
fn default_pulse_zone_max() -> f32 {
    0.9
}
fn default_pulse_accel_g() -> f32 {
    0.2
}
fn default_pulse_base_bpm() -> u16 {
    70
}

fn default_coyote_max_strength() -> u8 {
    20
}
fn default_coyote_freq() -> u16 {
    100
}
fn default_coyote_intensity() -> u8 {
    30
}
fn default_coyote_follow_scale() -> f32 {
    1.0
}
fn default_peck_coarse_step() -> f32 {
    5.0
}
fn default_peck_fine_step() -> f32 {
    1.0
}
fn default_peck_fine_back() -> f32 {
    8.0
}
fn default_peck_move_vel() -> f32 {
    20.0
}
fn default_peck_release_ms() -> u64 {
    80
}
fn default_peck_threshold() -> f32 {
    0.3
}
fn default_peck_return_vel() -> f32 {
    10.0
}

fn default_touch_velocity() -> f32 {
    2.0
}
fn default_push_current() -> u16 {
    10
}
fn default_touch_accel() -> f32 {
    0.1
}
fn default_max_velocity() -> f32 {
    400.0
}
fn default_accel_g() -> f32 {
    0.3
}
fn default_profile() -> String {
    // Trapezoid is the only profile this controller's numerical-move command
    // honours; s_curve/filter fall back to it anyway (see validate()).
    "trapezoid".into()
}
fn default_true() -> bool {
    true
}
fn default_hw_model() -> u8 {
    3
}
fn default_adapter() -> String {
    "hci0".into()
}
fn default_server_env() -> String {
    "production".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_repo_config() {
        let cfg = Config::load("config.toml").expect("config.toml should parse");
        assert_eq!(cfg.variant().unwrap(), Variant::EightInch);
        assert_eq!(cfg.stroke_mm(), 200.0);
        assert_eq!(cfg.max_position_mm(), 200.0);
        assert_eq!(cfg.motion_profile().unwrap(), MotionProfile::Trapezoid);
        assert_eq!(cfg.actuator.baud_rate, 19200);
    }

    #[test]
    fn max_position_falls_back_to_stroke() {
        let cfg: Config = toml::from_str(
            r#"
            [actuator]
            variant = "6inch"
            [actuator.limits]
        "#,
        )
        .unwrap();
        assert_eq!(cfg.stroke_mm(), 150.0);
        assert_eq!(cfg.max_position_mm(), 150.0);
    }
}
