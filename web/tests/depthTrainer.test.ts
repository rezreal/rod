import { describe, it, expect, beforeEach } from 'vitest'
import { useDeviceStore } from '../src/store/deviceStore'
import {
  crossedCueSoon, fmtLongCountdown, mandatoryRestS, phaseLabel, submergedCapS,
} from '../src/lib/depthTrainer'
import type { Command, DepthProgress, DepthTrainerState, Telemetry } from '../src/types/sscp'

const progress: DepthProgress = {
  breathHoldS: 30, breathOffsetMm: 8,
  oscillationDepthMm: 10, holdDepthMm: 5, holdS: 5, holdCapS: 18, sessions: 2,
  nextAdvanceInS: 3600, suggestStepBack: false,
  canStart: true, blockReason: null, history: [],
}
const live: DepthTrainerState = {
  phase: 'hold', setIndex: 1, setCount: 3, strokeIndex: 0, strokeCount: 0,
  targetDepthMm: 5, plannedHoldS: 5, depthBeyondOriginMm: 4.9, submergedS: 3.2, submergedCapS: 18,
  restRemainingS: 0, restTotalS: 0, deadmanHeld: true, awaitingDeadman: false,
  emergency: false, emergencyReason: null, earlyRelease: false, cueSoonS: 1.5,
  warmupRemainingS: null, awaitingWarmupConfirm: false, warmupCanExtend: true,
}

function telemetry(extra: Partial<Telemetry>): Telemetry {
  return {
    positionMm: 0, positionPct: 0, moving: false, direction: 'stopped', actuatorConnected: true,
    servoOn: true, controllerReady: true, homed: true, positioningDone: true, pushActive: false,
    brakeReleased: false, alarmCode: 0, alarmMinor: false, alarmMajor: false, emergencyStop: false,
    motorVoltageLow: false, safetySpeed: false, handSwitch: false, comfortableDepthMm: 100,
    maxDepthMm: 100, mode: 'idle', coyoteAutoconnect: false, piupiuAutoconnect: false,
    ...extra,
  }
}

describe('depth trainer store mapping', () => {
  beforeEach(() => useDeviceStore.getState().setConnectionState('disconnected'))

  it('maps depthProgress and depthTrainer into status', () => {
    useDeviceStore.getState().setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: progress, depthTrainer: live }))
    const s = useDeviceStore.getState().status
    expect(s.mode).toBe('depth_trainer')
    expect(s.depthProgress).toEqual(progress)
    expect(s.depthTrainer).toEqual(live)
  })

  it('updates status when only a live field changes and keeps the reference when nothing changed', () => {
    const st = useDeviceStore.getState()
    st.setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: progress, depthTrainer: live }))
    const before = useDeviceStore.getState().status
    // Identical content in fresh objects: same status reference.
    st.setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: { ...progress }, depthTrainer: { ...live } }))
    expect(useDeviceStore.getState().status).toBe(before)
    st.setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: progress, depthTrainer: { ...live, emergency: true, emergencyReason: 'deadman released' } }))
    const after = useDeviceStore.getState().status
    expect(after).not.toBe(before)
    expect(after.depthTrainer?.emergency).toBe(true)
  })

  it('carries the warm-up fields and detects changes in them', () => {
    const st = useDeviceStore.getState()
    const warm = { ...live, phase: 'warmup' as const, warmupRemainingS: 90 }
    st.setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: progress, depthTrainer: warm }))
    const before = useDeviceStore.getState().status
    expect(before.depthTrainer?.warmupRemainingS).toBe(90)
    st.setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: progress, depthTrainer: { ...warm, warmupRemainingS: 0, awaitingWarmupConfirm: true, warmupCanExtend: false } }))
    const after = useDeviceStore.getState().status
    expect(after).not.toBe(before)
    expect(after.depthTrainer).toMatchObject({ warmupRemainingS: 0, awaitingWarmupConfirm: true, warmupCanExtend: false })
  })

  it('clears depth fields when the mode state disappears', () => {
    const st = useDeviceStore.getState()
    st.setTelemetry(telemetry({ mode: 'depth_trainer', depthProgress: progress, depthTrainer: live }))
    st.setTelemetry(telemetry({ mode: 'idle', depthProgress: progress }))
    expect(useDeviceStore.getState().status.depthTrainer).toBeUndefined()
  })
})

describe('depth trainer helpers', () => {
  it('applies the cap and rest rules', () => {
    expect(submergedCapS(30)).toBe(18)
    expect(submergedCapS(60)).toBe(20)
    expect(mandatoryRestS(2)).toBe(20)
    expect(mandatoryRestS(10)).toBe(30)
  })

  it('fires the soft cue once per downward crossing of 2 s', () => {
    expect(crossedCueSoon(2.4, 1.9)).toBe(true)
    expect(crossedCueSoon(null, 1.9)).toBe(true)
    expect(crossedCueSoon(1.9, 1.4)).toBe(false)
    expect(crossedCueSoon(2.4, 2.2)).toBe(false)
    expect(crossedCueSoon(null, 2.9)).toBe(false)
    expect(crossedCueSoon(1.1, null)).toBe(false)
    expect(crossedCueSoon(null, null)).toBe(false)
  })

  it('labels phases and long countdowns', () => {
    expect(phaseLabel('rating')).toBe('Rate this session')
    expect(phaseLabel('retracting')).toBe('Retracting to origin')
    expect(phaseLabel('mystery')).toBe('mystery')
    expect(fmtLongCountdown(0)).toBe('now')
    expect(fmtLongCountdown(5400)).toBe('1h 30m')
  })

  it('command shapes use snake_case tags and camelCase fields', () => {
    const cmds: Command[] = [
      { type: 'depth_start' },
      { type: 'depth_stop' },
      { type: 'depth_settings', breathHoldS: 30, breathOffsetMm: 8 },
      { type: 'depth_rate', rating: 'too_much' },
      { type: 'depth_step_back' },
      { type: 'depth_warmup_continue' },
      { type: 'depth_warmup_extend' },
    ]
    expect(JSON.parse(JSON.stringify(cmds[2]))).toEqual({ type: 'depth_settings', breathHoldS: 30, breathOffsetMm: 8 })
    expect(cmds.map((c) => c.type)).toEqual(['depth_start', 'depth_stop', 'depth_settings', 'depth_rate', 'depth_step_back', 'depth_warmup_continue', 'depth_warmup_extend'])
  })
})
