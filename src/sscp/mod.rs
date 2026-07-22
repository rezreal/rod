//! Rod Control Protocol (SSCP) v1.
//!
//! A first-party JSON-over-BLE protocol independent of the Handy FW4 RPC stack.
//! The wire format is plain UTF-8 JSON so it's trivially inspectable and needs
//! no proto codegen on the host side.
//!
//! See `docs/feature-web-ui.md` §Rod Control Protocol for the full spec.

pub mod service;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::state::{AppMode, AppState};

// ── Telemetry (device → app) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrillTelemetry {
    pub pushing: bool,
    pub feed_rate_mm_s: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpaleTelemetry {
    /// Rod is currently extending (button held).
    pub extending: bool,
    /// Button released; the auto-retract timer is armed (servo braked).
    pub waiting: bool,
    /// Rod is currently retracting back to home.
    pub retracting: bool,
    pub feed_rate_mm_s: f32,
    /// Hold duration (seconds) after release before the rod auto-retracts.
    pub retract_after_s: f32,
    /// Live countdown (seconds) to the auto-retract while `waiting`; 0 outside
    /// that phase.
    pub retract_remaining_s: f32,
    /// Set for one hold cycle once the retract countdown reaches zero without
    /// an explicit stop — the win condition.
    pub won: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoyoteTelemetry {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<u8>,
    pub strength_a: u8,
    pub strength_b: u8,
    /// Configured safety cap — the UI clamps sliders to this.
    pub max_strength: u8,
    /// Following the rod's motion intensity (vs. manual strength).
    pub following: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiuPiuTelemetry {
    pub connected: bool,
    /// Squirt trigger currently held (repeating every 100 ms).
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartRateTelemetry {
    pub connected: bool,
    /// Actively scanning/reconnecting (Connect requested, not yet subscribed).
    pub scanning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PulseTelemetry {
    /// Speed factor (mm/s per BPM).
    pub factor: f32,
    /// BPM driving the current speed.
    pub bpm: u16,
    pub velocity_mm_s: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnTelemetry {
    /// "armed" | "recording" | "ready" | "playing".
    pub phase: &'static str,
    /// Raw samples captured so far (while recording).
    pub points: u32,
    /// Support points after simplification (once ready).
    pub waypoints: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleTelemetry {
    /// Current pattern index (0-based).
    pub pattern: u32,
    /// Human-readable name of the current pattern.
    pub pattern_name: &'static str,
    /// Total number of patterns to cycle through.
    pub pattern_count: u32,
    pub paused: bool,
    /// Per-pattern playback parameters, indexed by pattern — always present
    /// (even when idle) so the UI can initialise its sliders.
    pub params: [crate::modes::cycle::CyclePatternParams; crate::modes::cycle::PATTERN_COUNT as usize],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameTelemetry {
    /// Active game kind, e.g. "edge_recover" (snake_case, matches GameStart).
    pub kind: &'static str,
    /// Coarse phase: "active" | "recover" | "rest" | "hold" | "slip" | "win" | "idle".
    pub phase: &'static str,
    /// Generic 0..1 drive/closeness meter.
    pub intensity: f32,
    /// Level / interval / checkpoint / lines-lost, per game.
    pub level: u32,
    /// Elapsed duration, in seconds.
    pub duration_s: f32,
    /// Whether the deadman button is currently held.
    pub holding: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RampTelemetry {
    pub intensity: f32,
    pub velocity_mm_s: f32,
    pub zone_min: f32,
    pub zone_max: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HampTelemetry {
    pub running: bool,
    pub velocity: f32,
    pub zone_min: f32,
    pub zone_max: f32,
    pub softness: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub firmware_version: &'static str,
    pub stroke_mm: f32,
    pub device_name: String,
    pub sscp_version: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlumbTelemetry {
    /// Current upper bound for oscillation (mm).
    pub target_mm: f32,
    /// True while the servo is off and the user is repositioning the rod.
    pub handing_off: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurgeTelemetry {
    /// Current arousal level [0, 1]: drives both speed and depth.
    pub arousal: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TideTelemetry {
    /// Current stroke speed (mm/s), eased by the hand switch.
    pub speed_mm_s: f32,
    /// Current upper bound for oscillation (mm).
    pub target_mm: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EchoTelemetry {
    /// Absolute target depth of the next stroke (mm).
    pub current_depth_mm: f32,
    /// Number of depth steps taken since start.
    pub steps_taken: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceTelemetry {
    /// Current lower (return) bound (mm), user-set via hand positioning.
    pub lower_mm: f32,
    /// True while the servo is off and the user is repositioning the rod.
    pub handing_off: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoTelemetry {
    /// Established stroke cycle period (ms); 0 = no tempo set yet.
    pub period_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub position_mm: f32,
    pub position_pct: f32,
    pub moving: bool,
    pub direction: &'static str,

    /// Whether the Modbus link to the actuator is up (false = unplugged; the
    /// bridge still runs and BLE/sensors work).
    pub actuator_connected: bool,
    pub servo_on: bool,
    pub controller_ready: bool,
    pub homed: bool,
    pub positioning_done: bool,
    pub push_active: bool,
    pub brake_released: bool,

    pub alarm_code: u16,
    pub alarm_minor: bool,
    pub alarm_major: bool,
    pub emergency_stop: bool,
    pub motor_voltage_low: bool,
    pub safety_speed: bool,
    /// Hand/palm switch on the controller's PIO input (DIPM bit 0).
    pub hand_switch: bool,
    /// Comfortable-depth ceiling (mm) for oscillating modes; see
    /// `AppState::comfortable_depth_mm`.
    pub comfortable_depth_mm: f32,
    /// Max-depth ceiling (mm) for modes that press toward or hold a single far
    /// point; see `AppState::max_depth_mm`. Locked while a program is running.
    pub max_depth_mm: f32,
    /// Work-piece origin (mm) from the last calibration, if any — used to offer
    /// a "use calibrated contact" quick-set for comfortable depth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_origin_mm: Option<f32>,

    pub mode: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hamp: Option<HampTelemetry>,

    /// Present only while drill mode is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drill: Option<DrillTelemetry>,

    /// Present only while the auto-ramp program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ramp: Option<RampTelemetry>,

    /// Present only while an endurance game is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<GameTelemetry>,

    /// Present only while the cycle (pattern playlist) is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle: Option<CycleTelemetry>,

    /// Present only while the learn (teach-and-repeat) program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learn: Option<LearnTelemetry>,

    /// Present whenever a heart-rate sensor is connected or reporting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heart_rate: Option<HeartRateTelemetry>,

    /// Present only while the pulse (HR-reactive) program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pulse: Option<PulseTelemetry>,

    /// Present only while the impale program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impale: Option<ImpaleTelemetry>,

    /// Present only while a Coyote e-stim device is connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coyote: Option<CoyoteTelemetry>,

    /// Present only while a PiuPiu lube launcher is connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub piupiu: Option<PiuPiuTelemetry>,

    /// Persisted "autoconnect at boot" setting — always present so the app
    /// can render the Settings toggle even while disconnected.
    pub coyote_autoconnect: bool,
    pub piupiu_autoconnect: bool,

    /// Present only while the plumb program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plumb: Option<PlumbTelemetry>,

    /// Present only while the surge program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surge: Option<SurgeTelemetry>,

    /// Present only while the tide program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tide: Option<TideTelemetry>,

    /// Present only while the echo program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<EchoTelemetry>,

    /// Present only while the trace program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceTelemetry>,

    /// Present only while the tempo program is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempo: Option<TempoTelemetry>,

    /// Included in every frame so the app always has device context.
    pub device_info: DeviceInfo,
}

pub fn build_telemetry(st: &AppState, cfg: &Config) -> Telemetry {
    let stroke_mm = cfg.stroke_mm();
    let zone_span = (st.slide_max - st.slide_min).max(f32::MIN_POSITIVE);
    let position_pct = if stroke_mm > 0.0 {
        ((st.position_mm / stroke_mm) - st.slide_min) / zone_span
    } else {
        0.0
    }
    .clamp(0.0, 1.0);

    let direction = if !st.is_moving {
        "stopped"
    } else if st.hamp.direction {
        "retracting"
    } else {
        "extending"
    };

    let mode = match st.mode {
        AppMode::Uninitialized | AppMode::Idle => "idle",
        AppMode::Homing => "homing",
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
    };

    // Always include HAMP state so the UI can initialise sliders even in idle mode.
    let hamp = Some(HampTelemetry {
        running: st.hamp.running,
        velocity: st.hamp.velocity,
        zone_min: st.hamp.min,
        zone_max: st.hamp.max,
        softness: st.hamp.softness,
    });

    let drill = if st.drill.active {
        Some(DrillTelemetry {
            pushing: st.drill.pushing,
            feed_rate_mm_s: st.drill.feed_rate_mm_s,
        })
    } else {
        None
    };

    let ramp = if st.ramp.active {
        Some(RampTelemetry {
            intensity: st.ramp.intensity,
            velocity_mm_s: st.ramp.velocity_mm_s,
            zone_min: st.ramp.zone_min,
            zone_max: st.ramp.zone_max,
        })
    } else {
        None
    };

    let game = if st.game.active {
        Some(GameTelemetry {
            kind: st.game.kind.map(|k| k.as_str()).unwrap_or("idle"),
            phase: st.game.phase.as_str(),
            intensity: st.game.intensity,
            level: st.game.level,
            duration_s: st.game.duration_s,
            holding: st.game.holding,
        })
    } else {
        None
    };

    // Always include cycle state (like HAMP) so the UI can initialise its
    // per-pattern parameter sliders even while Cycle mode isn't running.
    let cycle = {
        let idx = (st.cycle.pattern as usize)
            .min(crate::modes::cycle::PATTERN_NAMES.len().saturating_sub(1));
        Some(CycleTelemetry {
            pattern: st.cycle.pattern,
            pattern_name: crate::modes::cycle::PATTERN_NAMES[idx],
            pattern_count: crate::modes::cycle::PATTERN_COUNT,
            paused: st.cycle.paused,
            params: st.cycle.params,
        })
    };

    let learn = if st.learn.active {
        Some(LearnTelemetry {
            phase: st.learn.phase.as_str(),
            points: st.learn.points,
            waypoints: st.learn.waypoints,
        })
    } else {
        None
    };

    let heart_rate =
        if st.sensors.hr_connected || st.sensors.hr_scanning || st.sensors.hr_bpm.is_some() {
            Some(HeartRateTelemetry {
                connected: st.sensors.hr_connected,
                scanning: st.sensors.hr_scanning,
                bpm: st.sensors.hr_bpm,
            })
        } else {
            None
        };

    let pulse = if st.pulse.active {
        Some(PulseTelemetry {
            factor: st.pulse.factor,
            bpm: st.pulse.bpm,
            velocity_mm_s: st.pulse.velocity_mm_s,
        })
    } else {
        None
    };

    let impale = if st.impale.active {
        let retract_remaining_s = st
            .impale
            .retract_deadline
            .map(|d| {
                d.saturating_duration_since(std::time::Instant::now())
                    .as_secs_f32()
            })
            .unwrap_or(0.0);
        Some(ImpaleTelemetry {
            extending: st.impale.extending,
            waiting: st.impale.waiting,
            retracting: st.impale.retracting,
            feed_rate_mm_s: st.impale.feed_rate_mm_s,
            retract_after_s: st.impale.retract_after_s,
            retract_remaining_s,
            won: st.impale.won,
        })
    } else {
        None
    };

    let plumb = if st.plumb.active {
        Some(PlumbTelemetry {
            target_mm: st.plumb.target_mm,
            handing_off: st.plumb.handing_off,
        })
    } else {
        None
    };

    let surge = if st.surge.active {
        Some(SurgeTelemetry {
            arousal: st.surge.arousal,
        })
    } else {
        None
    };

    let tide = if st.tide.active {
        Some(TideTelemetry {
            speed_mm_s: st.tide.speed_mm_s,
            target_mm: st.tide.target_mm,
        })
    } else {
        None
    };

    let echo = if st.echo.active {
        Some(EchoTelemetry {
            current_depth_mm: st.echo.current_depth_mm,
            steps_taken: st.echo.steps_taken,
        })
    } else {
        None
    };

    let trace = if st.trace.active {
        Some(TraceTelemetry {
            lower_mm: st.trace.lower_mm,
            handing_off: st.trace.handing_off,
        })
    } else {
        None
    };

    let tempo = if st.tempo.active {
        Some(TempoTelemetry {
            period_ms: st.tempo.period_ms,
        })
    } else {
        None
    };

    let coyote = if st.coyote.connected {
        Some(CoyoteTelemetry {
            connected: st.coyote.connected,
            battery: st.coyote.battery,
            strength_a: st.coyote.strength_a,
            strength_b: st.coyote.strength_b,
            max_strength: cfg.devices.coyote.max_strength,
            following: st.coyote.following,
        })
    } else {
        None
    };

    let piupiu = if st.piupiu.connected {
        Some(PiuPiuTelemetry {
            connected: st.piupiu.connected,
            active: st.piupiu.active,
        })
    } else {
        None
    };

    Telemetry {
        position_mm: st.position_mm,
        position_pct,
        moving: st.is_moving,
        direction,
        actuator_connected: st.actuator_connected,
        servo_on: st.servo_on,
        controller_ready: st.controller_ready,
        homed: st.homing_complete,
        positioning_done: st.positioning_done,
        push_active: st.push_active,
        brake_released: st.brake_released,
        alarm_code: st.alarm_code,
        alarm_minor: st.alarm_minor,
        alarm_major: st.alarm_major,
        emergency_stop: st.emergency_stop,
        motor_voltage_low: st.motor_voltage_low,
        safety_speed: st.safety_speed,
        hand_switch: st.hand_switch,
        comfortable_depth_mm: st.comfortable_depth_mm,
        max_depth_mm: st.max_depth_mm,
        work_origin_mm: st.work_origin_mm,
        mode,
        hamp,
        drill,
        ramp,
        game,
        cycle,
        learn,
        heart_rate,
        pulse,
        impale,
        coyote,
        piupiu,
        coyote_autoconnect: st.coyote_autoconnect,
        piupiu_autoconnect: st.piupiu_autoconnect,
        plumb,
        surge,
        tide,
        echo,
        trace,
        tempo,
        device_info: DeviceInfo {
            firmware_version: env!("CARGO_PKG_VERSION"),
            stroke_mm,
            device_name: format!("Rod-{}", st.uid),
            sscp_version: 1,
        },
    }
}

// ── Commands (app → device) ───────────────────────────────────────────────────

/// A SSCP command received from a connected BLE central. The `type` field is
/// the serde tag; all other fields are optional partial-update parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    StopAll,
    HampStart,
    HampStop,
    HampConfig {
        velocity: Option<f32>,
        // Accept both snake_case (canonical) and camelCase (JS convention).
        #[serde(alias = "zoneMin")]
        zone_min: Option<f32>,
        #[serde(alias = "zoneMax")]
        zone_max: Option<f32>,
        softness: Option<f32>,
    },
    HdspMove {
        #[serde(alias = "positionPct")]
        position_pct: f32,
        #[serde(alias = "velocityPct")]
        velocity_pct: f32,
    },
    HdspStop,
    Calibrate,
    ResetAlarm,
    /// Enter drill mode (servo off; rod moves freely).
    DrillStart {
        /// Override the configured default feed rate (mm/s).
        #[serde(alias = "feedRateMmS")]
        feed_rate_mm_s: Option<f32>,
    },
    /// Deadman pulse: sent repeatedly (~10–20 ms) while the button is held.
    /// Enables the servo on the first pulse and advances the rod outward.
    DrillPush {
        /// Per-pulse feed-rate override (mm/s). Usually omitted.
        #[serde(alias = "feedRateMmS")]
        feed_rate_mm_s: Option<f32>,
    },
    /// Update the outward feed rate without changing the push state.
    DrillConfig {
        #[serde(alias = "feedRateMmS")]
        feed_rate_mm_s: f32,
    },
    /// Leave drill mode: stop motion and release servo.
    DrillStop,
    /// Start the auto-ramp program (servo on; oscillates, building over time).
    RampStart {
        /// Override the configured climb duration (seconds).
        #[serde(alias = "durationS")]
        duration_s: Option<f32>,
    },
    /// Nudge the ramp intensity up/down and reset the idle timeout.
    RampNudge {
        /// Signed fraction to shift intensity by (e.g. ±0.1).
        delta: f32,
    },
    /// Leave ramp mode: stop motion and release servo.
    RampStop,
    /// Start an endurance game by kind (snake_case, e.g. "edge_recover").
    GameStart {
        kind: String,
    },
    /// Deadman button state for the active game. Resend `down: true` every
    /// ~50 ms while held; send `down: false` on release.
    GameButton {
        down: bool,
    },
    /// Leave the game: stop motion and release servo.
    GameStop,
    /// Start the cycle (pattern playlist) at the first pattern.
    CycleStart,
    /// Button edge for cycle mode: `down: true` on press, `false` on release.
    /// The bridge times the hold to tell short press (next pattern) from long
    /// press (pause toggle).
    CycleButton {
        down: bool,
    },
    /// Leave cycle mode: stop motion and release servo.
    CycleStop,
    /// Update per-pattern playback parameters (partial update; unset fields
    /// keep their current value). Applies live within one tick, whether or
    /// not the pattern being edited is the one currently playing.
    CycleConfig {
        pattern: u32,
        speed: Option<f32>,
        intensity: Option<f32>,
        reps: Option<u32>,
        #[serde(alias = "pauseS")]
        pause_s: Option<f32>,
    },
    /// Enter learn (teach-and-repeat) mode.
    LearnStart,
    /// A button tap in learn mode: advance record → stop → play → re-arm.
    LearnButton,
    /// Leave learn mode: stop motion and release servo.
    LearnStop,
    /// Start the pulse (heart-rate-reactive) program; optional factor override.
    PulseStart {
        factor: Option<f32>,
    },
    /// Adjust the pulse speed factor (mm/s per BPM).
    PulseSetFactor {
        factor: f32,
    },
    /// Leave pulse mode: stop motion and release servo.
    PulseStop,
    /// Enter impale mode (servo off; rod free), optional feed-rate and hold-
    /// duration overrides.
    ImpaleStart {
        #[serde(alias = "feedRateMmS")]
        feed_rate_mm_s: Option<f32>,
        #[serde(alias = "retractAfterS")]
        retract_after_s: Option<f32>,
    },
    /// Button edge for impale mode: `down: true` extends the rod; `down: false`
    /// brakes it and arms the auto-retract timer.
    ImpaleButton {
        down: bool,
    },
    /// Update the hold duration (seconds) before the rod auto-retracts.
    ImpaleConfig {
        #[serde(alias = "retractAfterS")]
        retract_after_s: f32,
    },
    /// Leave impale mode: stop motion and release servo.
    ImpaleStop,
    /// Set DG-LAB Coyote per-channel strength (clamped to the configured cap).
    CoyoteSetStrength {
        a: u8,
        b: u8,
    },
    /// Make the Coyote follow the rod's motion intensity (or stop following).
    CoyoteFollow {
        enable: bool,
        scale: f32,
    },
    /// Immediately zero both Coyote channels.
    CoyoteStop,
    /// Start scanning for / connecting the Coyote (BLE central).
    CoyoteConnect,
    /// Disconnect the Coyote and stop scanning.
    CoyoteDisconnect,
    /// Persist the Coyote autoconnect-at-boot setting and connect/disconnect
    /// to match it immediately.
    SetCoyoteAutoconnect {
        enabled: bool,
    },
    /// Hold (`true`) or release (`false`) the PiuPiu squirt trigger. Resend
    /// `active: true` to keep "holding a shot"; the bridge repeats the
    /// underlying command every 100 ms while held.
    ///
    /// `rename`d: serde's `rename_all = "snake_case"` splits "PiuPiu" into two
    /// words ("piu_piu_squirt"), which doesn't match the device name used
    /// everywhere else (state fields, telemetry, sidecar files) — pin the
    /// wire tag explicitly instead.
    #[serde(rename = "piupiu_squirt")]
    PiuPiuSquirt {
        active: bool,
    },
    /// Start scanning for / connecting the PiuPiu lube launcher (BLE central).
    #[serde(rename = "piupiu_connect")]
    PiuPiuConnect,
    /// Disconnect the PiuPiu and stop scanning.
    #[serde(rename = "piupiu_disconnect")]
    PiuPiuDisconnect,
    /// Persist the PiuPiu autoconnect-at-boot setting and connect/disconnect
    /// to match it immediately.
    #[serde(rename = "set_piupiu_autoconnect")]
    SetPiuPiuAutoconnect {
        enabled: bool,
    },
    /// Start scanning for / connecting a heart-rate sensor (BLE central).
    HrConnect,
    /// Disconnect the heart-rate sensor and stop scanning.
    HrDisconnect,
    /// Set the comfortable-depth ceiling (mm) for oscillating modes; clamped
    /// to at most max depth (may equal it) and persisted. Always accepted
    /// (silently clamped).
    SetComfortableDepth {
        mm: f32,
    },
    /// Set the max-depth ceiling (mm) for modes that press toward or hold a
    /// single far point; clamped to stroke and persisted. Authoritative: if
    /// this lowers the ceiling below the current comfortable depth,
    /// comfortable depth is pulled down to fit. Silently ignored while a
    /// program is running.
    SetMaxDepth {
        mm: f32,
    },
    /// Start the plumb (fixed-speed oscillation) program.
    PlumbStart,
    /// Leave plumb mode: stop motion and release servo.
    PlumbStop,
    /// Start the surge (arousal-driven oscillation) program.
    SurgeStart,
    /// Leave surge mode: stop motion and release servo.
    SurgeStop,
    /// Start the tide (speed-easing oscillation) program.
    TideStart,
    /// Leave tide mode: stop motion and release servo.
    TideStop,
    /// Start the echo (tap-stepping) program.
    EchoStart,
    /// Leave echo mode: stop motion and release servo.
    EchoStop,
    /// Start the trace (fixed-ceiling oscillation) program.
    TraceStart,
    /// Leave trace mode: stop motion and release servo.
    TraceStop,
    /// Start the tempo (rhythm-tapped oscillation) program.
    TempoStart,
    /// Leave tempo mode: stop motion and release servo.
    TempoStop,
}

/// Ack sent back on the Ack characteristic after processing a command.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandAck {
    pub seq: u32,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `Command` wire tag the web app actually sends (`web/src/types/sscp.ts`)
    /// must deserialize here. Guards against `rename_all` mis-splitting a
    /// variant name like "PiuPiu" into "piu_piu" — a case serde's default
    /// snake_case conversion gets wrong for repeated-syllable device names,
    /// silently dropping every command from that device.
    #[test]
    fn piupiu_and_coyote_command_tags_match_the_web_app() {
        let cases = [
            r#"{"type":"piupiu_squirt","active":true}"#,
            r#"{"type":"piupiu_connect"}"#,
            r#"{"type":"piupiu_disconnect"}"#,
            r#"{"type":"set_piupiu_autoconnect","enabled":true}"#,
            r#"{"type":"coyote_connect"}"#,
            r#"{"type":"coyote_disconnect"}"#,
            r#"{"type":"set_coyote_autoconnect","enabled":true}"#,
        ];
        for json in cases {
            let result: Result<Command, _> = serde_json::from_str(json);
            assert!(result.is_ok(), "{json} failed to deserialize: {result:?}");
        }
    }

    #[test]
    fn cycle_config_deserializes_partial_updates() {
        let full = r#"{"type":"cycle_config","pattern":3,"speed":1.5,"intensity":0.8,"reps":4,"pauseS":2.5}"#;
        match serde_json::from_str::<Command>(full).expect("full CycleConfig should deserialize") {
            Command::CycleConfig { pattern, speed, intensity, reps, pause_s } => {
                assert_eq!(pattern, 3);
                assert_eq!(speed, Some(1.5));
                assert_eq!(intensity, Some(0.8));
                assert_eq!(reps, Some(4));
                assert_eq!(pause_s, Some(2.5));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let partial = r#"{"type":"cycle_config","pattern":0,"speed":2.0}"#;
        match serde_json::from_str::<Command>(partial).expect("partial CycleConfig should deserialize") {
            Command::CycleConfig { speed, intensity, reps, pause_s, .. } => {
                assert_eq!(speed, Some(2.0));
                assert_eq!(intensity, None);
                assert_eq!(reps, None);
                assert_eq!(pause_s, None);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
