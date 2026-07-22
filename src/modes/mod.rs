//! Mode controllers: HAMP (oscillation), HDSP (direct streaming, handled inline
//! by the dispatcher) and HSP (script playback). The HAMP and HSP tasks own
//! their own timing loops and read/write [`AppState`](crate::state::AppState);
//! the dispatcher drives them with the control messages defined here.

pub mod cycle;
pub mod drill;
pub mod echo;
pub mod games;
pub mod hamp;
pub mod handswitch;
pub mod hdsp;
pub mod hsp;
pub mod impale;
pub mod learn;
pub mod plumb;
pub mod pulse;
pub mod ramp;
pub mod surge;
pub mod tempo;
pub mod tide;
pub mod trace;

use tokio::sync::mpsc;

/// Bundle of control senders for the SSCP-extension modes (drill, ramp, games,
/// cycle, learn, pulse). Passed together through the transport/dispatch layers
/// so adding a mode doesn't grow every signature by one argument.
#[derive(Clone)]
pub struct ModeControls {
    pub drill: mpsc::Sender<DrillControl>,
    pub ramp: mpsc::Sender<RampControl>,
    pub game: mpsc::Sender<GameControl>,
    pub cycle: mpsc::Sender<CycleControl>,
    pub learn: mpsc::Sender<LearnControl>,
    pub pulse: mpsc::Sender<PulseControl>,
    pub impale: mpsc::Sender<ImpaleControl>,
    pub plumb: mpsc::Sender<PlumbControl>,
    pub surge: mpsc::Sender<SurgeControl>,
    pub tide: mpsc::Sender<TideControl>,
    pub echo: mpsc::Sender<EchoControl>,
    pub trace: mpsc::Sender<TraceControl>,
    pub tempo: mpsc::Sender<TempoControl>,
    /// External e-stim device (DG-LAB Coyote); not a mode, threaded here so it
    /// rides the same control plumbing and `StopAll` reaches it.
    pub coyote: mpsc::Sender<crate::devices::CoyoteControl>,
    /// Heart-rate sensor (BLE central); threaded here so the UI can pair on
    /// demand (Connect/Disconnect). Not a mode.
    pub sensors: mpsc::Sender<crate::sensors::SensorControl>,
}

/// Control messages to the drill task. The SSCP command handler sends these
/// directly; the dispatcher also sends [`DrillControl::Stop`] via
/// [`Dispatcher::stop_everything`].
#[derive(Debug, Clone, PartialEq)]
pub enum DrillControl {
    /// Enter drill mode, optionally overriding the configured feed rate.
    Start { feed_rate_mm_s: Option<f32> },
    /// Deadman pulse: the button is still held. The task enables the servo on
    /// the first pulse and resets the deadman timer on every pulse.
    Push { feed_rate_mm_s: Option<f32> },
    /// Update the feed rate while drill mode is active.
    SetFeedRate { feed_rate_mm_s: f32 },
    /// Leave drill mode: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages to the ramp (auto-ramp) task. Like drill, the SSCP command
/// handler sends these directly; the dispatcher also sends [`RampControl::Stop`]
/// via [`Dispatcher::stop_everything`] and when switching away from the mode.
#[derive(Debug, Clone, PartialEq)]
pub enum RampControl {
    /// Start the auto-ramp program, optionally overriding the climb duration.
    Start { duration_s: Option<f32> },
    /// Nudge the current intensity by a signed fraction; also resets the idle
    /// timeout that would otherwise auto-stop the program.
    Nudge { delta: f32 },
    /// Leave ramp mode: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages to the impale task. The button is held to extend the rod
/// outward; releasing brakes it and arms an auto-retract timer.
#[derive(Debug, Clone, PartialEq)]
pub enum ImpaleControl {
    /// Enter impale mode (servo off; rod free), optionally overriding the feed
    /// rate and hold (auto-retract) duration.
    Start {
        feed_rate_mm_s: Option<f32>,
        retract_after_s: Option<f32>,
    },
    /// Button edge: `down = true` extends the rod; `down = false` brakes it and
    /// arms the auto-retract timer.
    Button { down: bool },
    /// Update the hold duration (seconds) before auto-retract. If the rod is
    /// already braked and waiting, the timer is re-armed from now.
    SetRetractAfter { retract_after_s: f32 },
    /// Leave impale mode: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages to the pulse (heart-rate-reactive) task.
#[derive(Debug, Clone, PartialEq)]
pub enum PulseControl {
    /// Start oscillating; speed = bpm × factor (optional factor override).
    Start { factor: Option<f32> },
    /// Adjust the speed factor (mm/s per BPM) live.
    SetFactor { factor: f32 },
    /// Leave pulse mode: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages to the learn (teach-and-repeat) task. `Button` is a single
/// tap that advances the record → stop → play → re-arm cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum LearnControl {
    /// Enter learn mode (Armed, servo off).
    Start,
    /// A button tap: advance the phase machine.
    Button,
    /// Leave learn mode: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages to the cycle (pattern-playlist) task. The button events
/// carry only down/up edges; the task times them to tell a short press (next
/// pattern) from a long press (pause toggle).
#[derive(Debug, Clone, PartialEq)]
pub enum CycleControl {
    /// Start the playlist at the first pattern, running.
    Start,
    /// Button edge: `down = true` on press, `false` on release.
    Button { down: bool },
    /// Leave cycle mode: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages to the endurance-games task. The SSCP handler sends these;
/// the dispatcher also sends [`GameControl::Stop`] on mode-switch / stop-all.
#[derive(Debug, Clone, PartialEq)]
pub enum GameControl {
    /// Start a game by [`GameKind`](crate::state::GameKind). Play doesn't
    /// begin immediately — the task arms and waits for the triple-tap ready
    /// signal (see [`GameControl::HardwareTap`]).
    Start { kind: crate::state::GameKind },
    /// Button state heartbeat from the client. `down = true` is resent every
    /// ~50 ms while held (deadman); `down = false` (or a heartbeat gap) means
    /// released. Each game interprets hold/release per its own rules.
    Button { down: bool },
    /// A physical hand-switch press (rising edge only). Only the hardware
    /// switch sends this — the web/app button cannot — so the "I am ready"
    /// gesture that arms a game can't be triggered from the phone alone; the
    /// player must be at the actuator itself. Ignored once play has started.
    HardwareTap,
    /// Leave the game: stop motion, release servo, return to Idle.
    Stop,
}

/// Control messages from the dispatcher to the HAMP task. The dispatcher has
/// already written the desired `HampRuntime` into `AppState`; these tell the
/// task to (re)evaluate. `Update` re-triggers immediately (e.g. velocity/zone
/// change) per SPEC §7.2.
#[derive(Debug, Clone, PartialEq)]
pub enum HampControl {
    Start,
    Stop,
    Update,
}

/// Control messages from the dispatcher to the HSP playback task. Buffer
/// mutations (Setup/Add/Flush) are applied to `AppState` by the dispatcher
/// before signalling; these carry only playback-timing parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum HspControl {
    /// Buffer/stream (re)initialised; stop any current playback.
    Setup,
    /// Points were appended (or flushed+appended).
    Added,
    Play {
        start_time: i32,
        server_time: u64,
        playback_rate: f32,
        looped: bool,
        pause_on_starving: bool,
    },
    Stop,
    Pause,
    Resume {
        pick_up: bool,
    },
    SetCurrentTime {
        current_time: i32,
        server_time: u64,
        filter: f32,
    },
    SetPlaybackRate(f32),
    SetLoop(bool),
}

/// Control messages to the plumb (fixed-speed oscillation) task. The hand
/// switch is read directly by the task; these only start/stop the program.
#[derive(Debug, Clone, PartialEq)]
pub enum PlumbControl {
    Start,
    Stop,
}

/// Control messages to the surge (arousal-driven oscillation) task.
#[derive(Debug, Clone, PartialEq)]
pub enum SurgeControl {
    Start,
    Stop,
}

/// Control messages to the tide (speed-easing oscillation) task.
#[derive(Debug, Clone, PartialEq)]
pub enum TideControl {
    Start,
    Stop,
}

/// Control messages to the echo (tap-stepping) task.
#[derive(Debug, Clone, PartialEq)]
pub enum EchoControl {
    Start,
    Stop,
}

/// Control messages to the trace (fixed-ceiling oscillation) task.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceControl {
    Start,
    Stop,
}

/// Control messages to the tempo (rhythm-tapped oscillation) task.
#[derive(Debug, Clone, PartialEq)]
pub enum TempoControl {
    Start,
    Stop,
}
