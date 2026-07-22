/**
 * Device state store — two independent slices to minimise re-renders.
 *
 *  PositionSlice  — position + waveform, updates at telemetry rate (~10 Hz)
 *  StatusSlice    — health bits, alarm, mode, hamp params — only written when
 *                   something actually changes (server already deduplicates)
 *
 * Components that only care about position (StrokeGauge, WaveformChart) don't
 * re-render when a status bit flips, and vice-versa.
 */
import { create } from 'zustand'
import type { ConnectionState, CoyoteState, CycleState, DeviceInfo, DrillState, EchoState, GameState, HampState, HdspState, HeartRateState, HspState, ImpaleState, LearnState, PiuPiuState, PlumbState, ProgramMode, PulseState, RampState, SurgeState, TempoState, TideState, TraceState, Telemetry } from '../types/sscp'

// ── Position slice ────────────────────────────────────────────────────────────

export interface PositionState {
  positionPct: number
  positionMm: number
  moving: boolean
  direction: Telemetry['direction']
  waveform: number[]   // ring buffer of positionPct values (~12 s)
}

// ── Status slice ──────────────────────────────────────────────────────────────

export interface StatusState {
  mode: ProgramMode
  actuatorConnected: boolean
  servoOn: boolean
  controllerReady: boolean
  homed: boolean
  positioningDone: boolean
  pushActive: boolean
  brakeReleased: boolean
  alarmCode: number
  alarmMinor: boolean
  alarmMajor: boolean
  emergencyStop: boolean
  motorVoltageLow: boolean
  safetySpeed: boolean
  handSwitch: boolean
  maxDepthMm: number
  workOriginMm: number | undefined
  hamp:  HampState  | undefined
  hdsp:  HdspState  | undefined
  hsp:   HspState   | undefined
  drill: DrillState | undefined
  ramp:  RampState  | undefined
  game:  GameState  | undefined
  cycle: CycleState | undefined
  learn: LearnState | undefined
  heartRate: HeartRateState | undefined
  pulse: PulseState | undefined
  impale: ImpaleState | undefined
  coyote: CoyoteState | undefined
  piupiu: PiuPiuState | undefined
  /** Persisted "autoconnect at boot" setting — independent of connection. */
  coyoteAutoconnect: boolean
  piupiuAutoconnect: boolean
  plumb: PlumbState | undefined
  surge: SurgeState | undefined
  tide: TideState | undefined
  echo: EchoState | undefined
  trace: TraceState | undefined
  tempo: TempoState | undefined
}

// ── Root store ────────────────────────────────────────────────────────────────

const WAVEFORM_SAMPLES = 150  // ~15 s at 100 ms

interface DeviceStore {
  connectionState: ConnectionState
  deviceInfo: DeviceInfo | null
  activeProgram: 'hamp' | 'hdsp' | 'hsp' | 'drill' | 'ramp' | 'game' | 'cycle' | 'learn' | 'pulse' | 'impale' | 'plumb' | 'surge' | 'tide' | 'echo' | 'trace' | 'tempo'

  position: PositionState
  status: StatusState

  setConnectionState(s: ConnectionState): void
  setTelemetry(t: Telemetry): void
  setDeviceInfo(d: DeviceInfo): void
  setActiveProgram(p: 'hamp' | 'hdsp' | 'hsp' | 'drill' | 'ramp' | 'game' | 'cycle' | 'learn' | 'pulse' | 'impale' | 'plumb' | 'surge' | 'tide' | 'echo' | 'trace' | 'tempo'): void
}

const defaultPosition: PositionState = {
  positionPct: 0,
  positionMm: 0,
  moving: false,
  direction: 'stopped',
  waveform: [],
}

const defaultStatus: StatusState = {
  mode: 'idle',
  actuatorConnected: false,
  servoOn: false,
  controllerReady: false,
  homed: false,
  positioningDone: false,
  pushActive: false,
  brakeReleased: false,
  alarmCode: 0,
  alarmMinor: false,
  alarmMajor: false,
  emergencyStop: false,
  motorVoltageLow: false,
  safetySpeed: false,
  handSwitch: false,
  maxDepthMm: 0,
  workOriginMm: undefined,
  hamp:  undefined,
  hdsp:  undefined,
  hsp:   undefined,
  drill: undefined,
  ramp:  undefined,
  game:  undefined,
  cycle: undefined,
  learn: undefined,
  heartRate: undefined,
  pulse: undefined,
  impale: undefined,
  coyote: undefined,
  piupiu: undefined,
  coyoteAutoconnect: false,
  piupiuAutoconnect: false,
  plumb: undefined,
  surge: undefined,
  tide: undefined,
  echo: undefined,
  trace: undefined,
  tempo: undefined,
}

export const useDeviceStore = create<DeviceStore>((set) => ({
  connectionState: 'disconnected',
  deviceInfo: null,
  activeProgram: 'hamp',
  position: defaultPosition,
  status: defaultStatus,

  setConnectionState(s) {
    set({ connectionState: s })
    if (s === 'disconnected') {
      set({ position: defaultPosition, status: defaultStatus })
    }
  },

  setTelemetry(t) {
    set((prev) => {
      // ── Position slice — always update ───────────────────────────────────
      const waveform = [...prev.position.waveform, t.positionPct]
      if (waveform.length > WAVEFORM_SAMPLES) waveform.shift()

      const newPos: PositionState = {
        positionPct:  t.positionPct,
        positionMm:   t.positionMm,
        moving:       t.moving,
        direction:    t.direction,
        waveform,
      }

      // ── Status slice — only update when something changed ────────────────
      const prev_s = prev.status
      const newHamp: HampState | undefined = t.hamp
        ? { running: t.hamp.running, velocity: t.hamp.velocity,
            zoneMin: t.hamp.zoneMin, zoneMax: t.hamp.zoneMax,
            softness: t.hamp.softness }
        : undefined

      const statusChanged =
        t.mode          !== prev_s.mode          ||
        t.actuatorConnected !== prev_s.actuatorConnected ||
        t.servoOn       !== prev_s.servoOn        ||
        t.controllerReady !== prev_s.controllerReady ||
        t.homed         !== prev_s.homed          ||
        t.alarmCode     !== prev_s.alarmCode      ||
        t.alarmMajor    !== prev_s.alarmMajor     ||
        t.alarmMinor    !== prev_s.alarmMinor     ||
        t.emergencyStop !== prev_s.emergencyStop  ||
        t.motorVoltageLow !== prev_s.motorVoltageLow ||
        t.positioningDone !== prev_s.positioningDone ||
        t.pushActive    !== prev_s.pushActive     ||
        t.brakeReleased !== prev_s.brakeReleased  ||
        t.handSwitch    !== prev_s.handSwitch     ||
        t.maxDepthMm    !== prev_s.maxDepthMm     ||
        t.workOriginMm  !== prev_s.workOriginMm   ||
        t.safetySpeed   !== prev_s.safetySpeed    ||
        // HAMP params
        newHamp?.running  !== prev_s.hamp?.running  ||
        newHamp?.velocity !== prev_s.hamp?.velocity ||
        newHamp?.zoneMin  !== prev_s.hamp?.zoneMin  ||
        newHamp?.zoneMax  !== prev_s.hamp?.zoneMax  ||
        newHamp?.softness !== prev_s.hamp?.softness ||
        // HDSP / HSP state
        t.hdsp?.state       !== prev_s.hdsp?.state       ||
        t.hsp?.state        !== prev_s.hsp?.state        ||
        t.hsp?.bufferPoints !== prev_s.hsp?.bufferPoints ||
        // Drill state
        t.drill?.pushing     !== prev_s.drill?.pushing     ||
        t.drill?.feedRateMmS !== prev_s.drill?.feedRateMmS ||
        // Ramp state
        t.ramp?.intensity   !== prev_s.ramp?.intensity   ||
        t.ramp?.velocityMmS !== prev_s.ramp?.velocityMmS ||
        t.ramp?.zoneMin     !== prev_s.ramp?.zoneMin     ||
        t.ramp?.zoneMax     !== prev_s.ramp?.zoneMax     ||
        // Game state
        t.game?.kind      !== prev_s.game?.kind      ||
        t.game?.phase     !== prev_s.game?.phase     ||
        t.game?.intensity !== prev_s.game?.intensity ||
        t.game?.level     !== prev_s.game?.level     ||
        t.game?.scoreS    !== prev_s.game?.scoreS    ||
        t.game?.holding   !== prev_s.game?.holding   ||
        // Cycle state
        t.cycle?.pattern      !== prev_s.cycle?.pattern      ||
        t.cycle?.patternName  !== prev_s.cycle?.patternName  ||
        t.cycle?.patternCount !== prev_s.cycle?.patternCount ||
        t.cycle?.paused       !== prev_s.cycle?.paused       ||
        // Learn state
        t.learn?.phase     !== prev_s.learn?.phase     ||
        t.learn?.points    !== prev_s.learn?.points    ||
        t.learn?.waypoints !== prev_s.learn?.waypoints ||
        // Heart-rate state
        t.heartRate?.connected !== prev_s.heartRate?.connected ||
        t.heartRate?.scanning  !== prev_s.heartRate?.scanning  ||
        t.heartRate?.bpm       !== prev_s.heartRate?.bpm       ||
        // Pulse state
        t.pulse?.factor      !== prev_s.pulse?.factor      ||
        t.pulse?.bpm         !== prev_s.pulse?.bpm         ||
        t.pulse?.velocityMmS !== prev_s.pulse?.velocityMmS ||
        // Impale state
        t.impale?.extending     !== prev_s.impale?.extending     ||
        t.impale?.waiting       !== prev_s.impale?.waiting       ||
        t.impale?.retracting    !== prev_s.impale?.retracting    ||
        t.impale?.feedRateMmS   !== prev_s.impale?.feedRateMmS   ||
        t.impale?.retractAfterS !== prev_s.impale?.retractAfterS ||
        // Coyote (e-stim) state
        t.coyote?.connected   !== prev_s.coyote?.connected   ||
        t.coyote?.battery     !== prev_s.coyote?.battery     ||
        t.coyote?.strengthA   !== prev_s.coyote?.strengthA   ||
        t.coyote?.strengthB   !== prev_s.coyote?.strengthB   ||
        t.coyote?.maxStrength !== prev_s.coyote?.maxStrength ||
        // PiuPiu (lube launcher) state
        t.piupiu?.connected !== prev_s.piupiu?.connected ||
        t.piupiu?.active    !== prev_s.piupiu?.active    ||
        // Autoconnect settings
        t.coyoteAutoconnect !== prev_s.coyoteAutoconnect ||
        t.piupiuAutoconnect !== prev_s.piupiuAutoconnect ||
        // Plumb state
        t.plumb?.targetMm   !== prev_s.plumb?.targetMm   ||
        t.plumb?.handingOff !== prev_s.plumb?.handingOff ||
        // Surge state
        t.surge?.arousal !== prev_s.surge?.arousal ||
        // Tide state
        t.tide?.speedMmS !== prev_s.tide?.speedMmS ||
        t.tide?.targetMm !== prev_s.tide?.targetMm ||
        // Echo state
        t.echo?.currentDepthMm !== prev_s.echo?.currentDepthMm ||
        t.echo?.stepsTaken     !== prev_s.echo?.stepsTaken     ||
        // Trace state
        t.trace?.lowerMm    !== prev_s.trace?.lowerMm    ||
        t.trace?.handingOff !== prev_s.trace?.handingOff ||
        // Tempo state
        t.tempo?.periodMs !== prev_s.tempo?.periodMs

      const newStatus: StatusState = statusChanged ? {
        mode:             t.mode,
        actuatorConnected: t.actuatorConnected,
        servoOn:          t.servoOn,
        controllerReady:  t.controllerReady,
        homed:            t.homed,
        positioningDone:  t.positioningDone,
        pushActive:       t.pushActive,
        brakeReleased:    t.brakeReleased,
        alarmCode:        t.alarmCode,
        alarmMinor:       t.alarmMinor,
        alarmMajor:       t.alarmMajor,
        emergencyStop:    t.emergencyStop,
        motorVoltageLow:  t.motorVoltageLow,
        safetySpeed:      t.safetySpeed,
        handSwitch:       t.handSwitch,
        maxDepthMm:       t.maxDepthMm,
        workOriginMm:     t.workOriginMm,
        hamp:             newHamp,
        hdsp:             t.hdsp,
        hsp:              t.hsp,
        drill:            t.drill,
        ramp:             t.ramp,
        game:             t.game,
        cycle:            t.cycle,
        learn:            t.learn,
        heartRate:        t.heartRate,
        pulse:            t.pulse,
        impale:           t.impale,
        coyote:           t.coyote,
        piupiu:           t.piupiu,
        coyoteAutoconnect: t.coyoteAutoconnect,
        piupiuAutoconnect: t.piupiuAutoconnect,
        plumb:            t.plumb,
        surge:            t.surge,
        tide:             t.tide,
        echo:             t.echo,
        trace:            t.trace,
        tempo:            t.tempo,
      } : prev_s  // same object reference → no re-render for status subscribers

      return { position: newPos, status: newStatus }
    })
  },

  setDeviceInfo(d) { set({ deviceInfo: d }) },
  setActiveProgram(p) { set({ activeProgram: p }) },
}))
