// Rod Control Protocol — TypeScript types
// Mirrors proto/sscp/v1/sscp.proto

export type ProgramMode = 'idle' | 'hamp' | 'hdsp' | 'hsp' | 'homing' | 'drill' | 'ramp' | 'game' | 'cycle' | 'learn' | 'pulse' | 'impale' | 'plumb' | 'surge' | 'tide' | 'echo' | 'trace' | 'tempo'
/** Phase of the learn (teach-and-repeat) program. */
export type LearnPhase = 'armed' | 'recording' | 'ready' | 'playing'
export type Direction = 'extending' | 'retracting' | 'stopped'

/** Endurance game identifiers (snake_case, matches the GameStart command). */
export type GameKind =
  | 'edge_recover'
  | 'hold_the_line'
  | 'gauntlet'
  | 'deadmans_climb'
  | 'stillness'
/** Coarse phase a game is in. */
export type GamePhase = 'idle' | 'active' | 'recover' | 'rest' | 'hold' | 'slip'
export type HdspMoveState = 'idle' | 'moving' | 'reached'
export type HspPlayState = 'stopped' | 'playing' | 'paused' | 'starving'

export interface HampState {
  running: boolean
  velocity: number   // 0–1
  zoneMin: number    // 0–1
  zoneMax: number    // 0–1
  /** 0 = hard/snappy reversals (max accel), 1 = very soft/gentle (min accel) */
  softness: number   // 0–1
}

export interface HdspState {
  state: HdspMoveState
}

export interface DrillState {
  /** True while the deadman is held (servo on, rod advancing). */
  pushing: boolean
  feedRateMmS: number
}

export interface RampState {
  /** Current intensity 0–1 (time curve + accumulated nudges). */
  intensity: number
  /** Current stroke velocity derived from intensity. */
  velocityMmS: number
  /** Current stroke zone (relative 0–1), widening with intensity. */
  zoneMin: number
  zoneMax: number
}

export interface GameState {
  kind: GameKind
  phase: GamePhase
  /** Generic 0–1 drive / closeness meter. */
  intensity: number
  /** Level / interval / checkpoint / lines-lost, interpreted per game. */
  level: number
  /** Primary endurance score, in seconds. */
  scoreS: number
  /** Whether the deadman button is currently held. */
  holding: boolean
}

export interface CoyoteState {
  connected: boolean
  /** Battery percentage, if reported. */
  battery?: number
  /** Device-reported current strength per channel (0–200). */
  strengthA: number
  strengthB: number
  /** Configured safety cap — clamp sliders to this. */
  maxStrength: number
  /** Following the rod's motion intensity (vs. manual strength). */
  following: boolean
}

export interface HeartRateState {
  connected: boolean
  /** Actively scanning/reconnecting (pairing requested, not yet subscribed). */
  scanning: boolean
  /** Latest BPM, if a sensor is reporting. */
  bpm?: number
}

export interface PulseState {
  /** Speed factor: mm/s of stroke velocity per BPM. */
  factor: number
  /** BPM driving the current speed. */
  bpm: number
  velocityMmS: number
}

export interface LearnState {
  phase: LearnPhase
  /** Raw samples captured so far (while recording). */
  points: number
  /** Support points after simplification (once ready). */
  waypoints: number
}

export interface ImpaleState {
  /** Rod is currently extending (button held). */
  extending: boolean
  /** Button released; auto-retract timer is armed (servo braked). */
  waiting: boolean
  /** Rod is currently retracting back to home. */
  retracting: boolean
  feedRateMmS: number
  /** Hold duration (seconds) after release before the rod auto-retracts. */
  retractAfterS: number
}

export interface CycleState {
  /** Current pattern index (0-based). */
  pattern: number
  /** Human-readable name of the current pattern. */
  patternName: string
  /** Total number of patterns. */
  patternCount: number
  paused: boolean
}

export interface PlumbState {
  /** Current upper bound for oscillation (mm). */
  targetMm: number
  /** Servo off; the user is repositioning the rod by hand. */
  handingOff: boolean
}

export interface SurgeState {
  /** Current arousal level 0–1 (drives speed and depth). */
  arousal: number
}

export interface TideState {
  /** Current stroke speed (mm/s), eased by the hand switch. */
  speedMmS: number
  /** Current upper bound for oscillation (mm). */
  targetMm: number
}

export interface EchoState {
  /** Absolute target depth of the next stroke (mm). */
  currentDepthMm: number
  /** Number of depth steps taken since start. */
  stepsTaken: number
}

export interface TraceState {
  /** Current lower (return) bound (mm), user-set by hand. */
  lowerMm: number
  /** Servo off; the user is repositioning the rod by hand. */
  handingOff: boolean
}

export interface TempoState {
  /** Established stroke cycle period (ms); 0 = no tempo set yet. */
  periodMs: number
}

export interface HspState {
  state: HspPlayState
  bufferPoints: number
  playbackRate: number
  looping: boolean
}

export interface DeviceInfo {
  firmwareVersion: string
  strokeMm: number
  deviceName: string
  sscpVersion: number
}

export interface Telemetry {
  // Position
  positionMm: number
  positionPct: number   // 0–1 within zone
  // Motion
  moving: boolean
  direction: Direction
  // System
  /** Modbus link to the actuator is up (false = unplugged; bridge still runs). */
  actuatorConnected: boolean
  servoOn: boolean
  controllerReady: boolean
  homed: boolean
  positioningDone: boolean
  pushActive: boolean
  brakeReleased: boolean
  // Faults
  alarmCode: number
  alarmMinor: boolean
  alarmMajor: boolean
  emergencyStop: boolean
  motorVoltageLow: boolean
  safetySpeed: boolean
  /** Hand/palm switch on the controller's PIO input (DIPM bit 0). */
  handSwitch: boolean
  /** Comfortable-depth ceiling (mm) for oscillating modes (HAMP, cycle, pulse,
   *  plumb, surge, tide, trace, tempo, and the fixed-zone games). Never
   *  exceeds maxDepthMm (may equal it). */
  comfortableDepthMm: number
  /** Max-depth ceiling (mm) for modes that press toward or hold a single far
   *  point (ramp, HDSP, HSP, learn, drill, impale, echo, Hold the Line).
   *  Locked while a program is running. */
  maxDepthMm: number
  /** Work-piece origin (mm) from the last calibration, if any. */
  workOriginMm?: number
  // Mode
  mode: ProgramMode
  hamp?: HampState
  hdsp?: HdspState
  hsp?: HspState
  drill?: DrillState
  ramp?: RampState
  game?: GameState
  cycle?: CycleState
  learn?: LearnState
  heartRate?: HeartRateState
  pulse?: PulseState
  impale?: ImpaleState
  coyote?: CoyoteState
  plumb?: PlumbState
  surge?: SurgeState
  tide?: TideState
  echo?: EchoState
  trace?: TraceState
  tempo?: TempoState
}

// ── Commands ────────────────────────────────────────────────

export interface StopAllCommand   { type: 'stop_all' }
export interface HampStartCommand { type: 'hamp_start' }
export interface HampStopCommand  { type: 'hamp_stop' }
export interface HampConfigCommand {
  type: 'hamp_config'
  velocity?: number
  zoneMin?: number
  zoneMax?: number
  /** 0 = hard, 1 = soft — translated to ACMD on the bridge */
  softness?: number
}
export interface HdspMoveCommand {
  type: 'hdsp_move'
  positionPct: number
  velocityPct: number
}
export interface HdspStopCommand  { type: 'hdsp_stop' }
export interface HspLoadCommand {
  type: 'hsp_load'
  points: HspPoint[]
  append: boolean
}
export interface HspPlayCommand   { type: 'hsp_play'; loop: boolean; rate: number }
export interface HspPauseCommand  { type: 'hsp_pause' }
export interface HspStopCommand   { type: 'hsp_stop' }
export interface HspSetRateCommand { type: 'hsp_rate'; rate: number }
export interface CalibrateCommand { type: 'calibrate' }
export interface ResetAlarmCommand { type: 'reset_alarm' }

/** Enter drill mode — servo off, rod moves freely. */
export interface DrillStartCommand  { type: 'drill_start'; feedRateMmS?: number }
/** Deadman pulse — send repeatedly (~15 ms) while button is held. */
export interface DrillPushCommand   { type: 'drill_push'; feedRateMmS?: number }
/** Update feed rate without changing push state. */
export interface DrillConfigCommand { type: 'drill_config'; feedRateMmS: number }
/** Exit drill mode — stop motion, release servo. */
export interface DrillStopCommand   { type: 'drill_stop' }

/** Start the auto-ramp program — servo on, builds speed/depth over time. */
export interface RampStartCommand   { type: 'ramp_start'; durationS?: number }
/** Nudge ramp intensity up/down a notch; also resets the idle timeout. */
export interface RampNudgeCommand   { type: 'ramp_nudge'; delta: number }
/** Exit ramp mode — stop motion, release servo. */
export interface RampStopCommand    { type: 'ramp_stop' }

/** Start an endurance game by kind. */
export interface GameStartCommand   { type: 'game_start'; kind: GameKind }
/** Deadman button state for the active game. Resend `down: true` every ~50 ms
 *  while held; send `down: false` on release. */
export interface GameButtonCommand  { type: 'game_button'; down: boolean }
/** Exit the game — stop motion, release servo. */
export interface GameStopCommand    { type: 'game_stop' }

/** Start the cycle pattern playlist. */
export interface CycleStartCommand  { type: 'cycle_start' }
/** Button edge for cycle mode. Send `down: true` on press, `down: false` on
 *  release; the bridge times the hold (short = next pattern, long = pause). */
export interface CycleButtonCommand { type: 'cycle_button'; down: boolean }
/** Exit cycle mode — stop motion, release servo. */
export interface CycleStopCommand   { type: 'cycle_stop' }

/** Enter learn (teach-and-repeat) mode. */
export interface LearnStartCommand  { type: 'learn_start' }
/** A button tap in learn mode: advance record → stop → play → re-arm. */
export interface LearnButtonCommand { type: 'learn_button' }
/** Exit learn mode — stop motion, release servo. */
export interface LearnStopCommand   { type: 'learn_stop' }

/** Start the pulse (heart-rate-reactive) program. */
export interface PulseStartCommand     { type: 'pulse_start'; factor?: number }
/** Adjust the pulse speed factor (mm/s per BPM). */
export interface PulseSetFactorCommand { type: 'pulse_set_factor'; factor: number }
/** Exit pulse mode — stop motion, release servo. */
export interface PulseStopCommand      { type: 'pulse_stop' }

/** Enter impale mode — servo off, rod moves freely. */
export interface ImpaleStartCommand  { type: 'impale_start'; feedRateMmS?: number; retractAfterS?: number }
/** Button edge for impale mode: `down: true` extends the rod; `down: false`
 *  brakes it and arms the auto-retract timer. */
export interface ImpaleButtonCommand { type: 'impale_button'; down: boolean }
/** Update the hold duration (seconds) before the rod auto-retracts. */
export interface ImpaleConfigCommand { type: 'impale_config'; retractAfterS: number }
/** Exit impale mode — stop motion, release servo. */
export interface ImpaleStopCommand   { type: 'impale_stop' }

/** Start the plumb (fixed-speed oscillation) program. */
export interface PlumbStartCommand { type: 'plumb_start' }
/** Exit plumb mode — stop motion, release servo. */
export interface PlumbStopCommand  { type: 'plumb_stop' }

/** Start the surge (arousal-driven oscillation) program. */
export interface SurgeStartCommand { type: 'surge_start' }
/** Exit surge mode — stop motion, release servo. */
export interface SurgeStopCommand  { type: 'surge_stop' }

/** Start the tide (speed-easing oscillation) program. */
export interface TideStartCommand  { type: 'tide_start' }
/** Exit tide mode — stop motion, release servo. */
export interface TideStopCommand   { type: 'tide_stop' }

/** Start the echo (tap-stepping) program. */
export interface EchoStartCommand  { type: 'echo_start' }
/** Exit echo mode — stop motion, release servo. */
export interface EchoStopCommand   { type: 'echo_stop' }

/** Start the trace (fixed-ceiling oscillation) program. */
export interface TraceStartCommand { type: 'trace_start' }
/** Exit trace mode — stop motion, release servo. */
export interface TraceStopCommand  { type: 'trace_stop' }

/** Start the tempo (rhythm-tapped oscillation) program. */
export interface TempoStartCommand { type: 'tempo_start' }
/** Exit tempo mode — stop motion, release servo. */
export interface TempoStopCommand  { type: 'tempo_stop' }

/** Set DG-LAB Coyote per-channel strength (bridge clamps to the safety cap). */
export interface CoyoteSetStrengthCommand { type: 'coyote_set_strength'; a: number; b: number }
/** Make the Coyote follow the rod's motion intensity (or stop following). */
export interface CoyoteFollowCommand      { type: 'coyote_follow'; enable: boolean; scale: number }
/** Immediately zero both Coyote channels. */
export interface CoyoteStopCommand        { type: 'coyote_stop' }

/** Start scanning for / connecting a heart-rate sensor (BLE central, on the Pi). */
export interface HrConnectCommand    { type: 'hr_connect' }
/** Disconnect the heart-rate sensor and stop scanning. */
export interface HrDisconnectCommand { type: 'hr_disconnect' }

/** Set the comfortable-depth ceiling (mm) for oscillating modes; bridge clamps
 *  to at most max depth (may equal it) and persists. Always accepted
 *  (silently clamped). */
export interface SetComfortableDepthCommand { type: 'set_comfortable_depth'; mm: number }
/** Set the max-depth ceiling (mm) for modes that press toward or hold a single
 *  far point; bridge clamps to stroke and persists. Authoritative: lowering
 *  this below the current comfortable depth pulls comfortable depth down to
 *  fit. Silently ignored while a program is running. */
export interface SetMaxDepthCommand  { type: 'set_max_depth'; mm: number }

export type Command =
  | StopAllCommand
  | HampStartCommand
  | HampStopCommand
  | HampConfigCommand
  | HdspMoveCommand
  | HdspStopCommand
  | HspLoadCommand
  | HspPlayCommand
  | HspPauseCommand
  | HspStopCommand
  | HspSetRateCommand
  | CalibrateCommand
  | ResetAlarmCommand
  | DrillStartCommand
  | DrillPushCommand
  | DrillConfigCommand
  | DrillStopCommand
  | RampStartCommand
  | RampNudgeCommand
  | RampStopCommand
  | GameStartCommand
  | GameButtonCommand
  | GameStopCommand
  | CycleStartCommand
  | CycleButtonCommand
  | CycleStopCommand
  | LearnStartCommand
  | LearnButtonCommand
  | LearnStopCommand
  | PulseStartCommand
  | PulseSetFactorCommand
  | PulseStopCommand
  | ImpaleStartCommand
  | ImpaleButtonCommand
  | ImpaleConfigCommand
  | ImpaleStopCommand
  | CoyoteSetStrengthCommand
  | CoyoteFollowCommand
  | CoyoteStopCommand
  | HrConnectCommand
  | HrDisconnectCommand
  | SetComfortableDepthCommand
  | SetMaxDepthCommand
  | PlumbStartCommand
  | PlumbStopCommand
  | SurgeStartCommand
  | SurgeStopCommand
  | TideStartCommand
  | TideStopCommand
  | EchoStartCommand
  | EchoStopCommand
  | TraceStartCommand
  | TraceStopCommand
  | TempoStartCommand
  | TempoStopCommand

export interface HspPoint {
  timeMs: number
  position: number  // 0–255
}

export interface CommandAck {
  seq: number
  ok: boolean
  error?: string
}

// ── Transport interface ──────────────────────────────────────

export type ConnectionState =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'unsupported'

export interface ITransport {
  readonly connectionState: ConnectionState
  send(cmd: Command, seq: number): void
  connect(): Promise<void>
  disconnect(): void
  onTelemetry: ((t: Telemetry) => void) | null
  onAck: ((ack: CommandAck) => void) | null
  onConnectionChange: ((state: ConnectionState) => void) | null
  // Delivered out-of-band from telemetry: device info is read once on connect
  // and must not be smuggled through a telemetry frame (that would clobber the
  // live status slice with stale values).
  onDeviceInfo: ((info: DeviceInfo) => void) | null
}
