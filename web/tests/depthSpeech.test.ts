import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { depthCue, emergencyText } from '../src/lib/depthSpeech'
import {
  CRITICAL_HOLD_MS, cancelStaleSpeech, cancelSpeech, listSpeechVoices, onSpeechVoicesChanged,
  primeSpeech, resetSpeechState, speakCue, speakSample, speechSupported,
} from '../src/audio/speech'
import { usePreferencesStore } from '../src/store/preferencesStore'
import type { DepthTrainerState } from '../src/types/sscp'

const base: DepthTrainerState = {
  phase: 'arming', setIndex: 0, setCount: 0, strokeIndex: 0, strokeCount: 0,
  targetDepthMm: 0, plannedHoldS: null, depthBeyondOriginMm: 0, submergedS: 0, submergedCapS: 18,
  restRemainingS: 0, restTotalS: 0, deadmanHeld: true, awaitingDeadman: false,
  emergency: false, emergencyReason: null, earlyRelease: false, cueSoonS: null,
  warmupRemainingS: null, awaitingWarmupConfirm: false, warmupCanExtend: true,
}
const st = (extra: Partial<DepthTrainerState>): DepthTrainerState => ({ ...base, ...extra })

describe('depthCue (phase → utterance)', () => {
  it('announces the armed state on the first snapshot of a session', () => {
    expect(depthCue(undefined, st({ awaitingDeadman: true, deadmanHeld: false }))).toEqual({
      text: 'Armed. Hold the switch when ready', critical: false,
    })
    expect(depthCue(undefined, st({}))).toBeNull()
  })

  it('maps each phase change to a short cue', () => {
    const arming = st({})
    expect(depthCue(arming, st({ phase: 'warmup', warmupRemainingS: 120 }))?.text).toBe('Warm-up, gentle strokes')
    expect(depthCue(arming, st({ phase: 'oscillation', setIndex: 1, setCount: 2 }))?.text).toBe('Strokes, set 1 of 2')
    expect(depthCue(arming, st({ phase: 'hold', setIndex: 2, setCount: 3 }))?.text).toBe('Hold, set 2 of 3')
    expect(depthCue(arming, st({ phase: 'hold' }))?.text).toBe('Hold')
    expect(depthCue(st({ phase: 'hold' }), st({ phase: 'rest', restTotalS: 20, restRemainingS: 20 }))?.text).toBe('Rest, 20 seconds')
    expect(depthCue(st({ phase: 'hold' }), st({ phase: 'rest', restTotalS: 90, restRemainingS: 90 }))?.text).toBe('Rest, 1 minute 30 seconds')
    expect(depthCue(arming, st({ phase: 'cooldown' }))?.text).toBe('Cool-down, easy strokes')
    expect(depthCue(st({ phase: 'cooldown' }), st({ phase: 'rating' }))?.text).toBe('Session done. How did that feel?')
    expect(depthCue(st({ phase: 'rating' }), st({ phase: 'ended' }))?.text).toBe('Session ended')
  })

  it('says nothing for arming/ordinary retracting and for unchanged state', () => {
    expect(depthCue(st({ phase: 'hold' }), st({ phase: 'retracting' }))).toBeNull()
    expect(depthCue(st({ phase: 'rest', restTotalS: 20 }), st({ phase: 'rest', restTotalS: 20, restRemainingS: 10 }))).toBeNull()
    expect(depthCue(st({ phase: 'oscillation', setIndex: 1, setCount: 2 }), st({ phase: 'oscillation', setIndex: 1, setCount: 2, strokeIndex: 3 }))).toBeNull()
  })

  it('prompts for the hand switch when a rest ends, and for the warm-up decision', () => {
    expect(depthCue(st({ phase: 'rest' }), st({ phase: 'arming', awaitingDeadman: true, deadmanHeld: false }))?.text)
      .toBe('Breathe. Hold the switch when ready')
    expect(depthCue(st({ phase: 'warmup' }), st({ phase: 'warmup', awaitingWarmupConfirm: true, warmupRemainingS: 0 }))?.text)
      .toBe('Warm-up done. Continue when you are ready')
  })

  it('counts down 3, 2, 1 once per number; the pre-move countdown is critical, the hold-end one is not', () => {
    const a = (c: number | null) => st({ phase: 'arming', cueSoonS: c })
    expect(depthCue(a(null), a(2.9))).toEqual({ text: 'Moving in Three', critical: true })
    expect(depthCue(a(2.9), a(2.6))).toBeNull()
    expect(depthCue(a(2.9), a(1.9))).toEqual({ text: 'Two', critical: true })
    expect(depthCue(a(1.9), a(0.9))).toEqual({ text: 'One', critical: true })
    expect(depthCue(a(0.9), a(0))).toBeNull()
    const h = (c: number | null) => st({ phase: 'hold', cueSoonS: c })
    expect(depthCue(h(null), h(3))).toEqual({ text: 'Coming out in Three', critical: false })
    expect(depthCue(h(2.5), h(1.5))).toEqual({ text: 'Two', critical: false })
  })

  it('warns once as the warm-up crosses 30 s', () => {
    const w = (s: number) => st({ phase: 'warmup', warmupRemainingS: s })
    expect(depthCue(w(31), w(29.5))?.text).toBe('Thirty seconds left')
    expect(depthCue(w(29.5), w(28))).toBeNull()
    expect(depthCue(w(20), w(0))).toBeNull()
  })

  it('emergency retract is critical, wins over everything, and is announced once', () => {
    const held = st({ phase: 'hold', cueSoonS: 1.5 })
    const lapse = st({ phase: 'retracting', emergency: true, emergencyReason: 'hand switch released', cueSoonS: null })
    expect(depthCue(held, lapse)).toEqual({ text: 'Released, retracting', critical: true })
    // later snapshots of the same emergency (even phase changes) stay quiet
    expect(depthCue(lapse, { ...lapse, phase: 'rating' })).toBeNull()
    expect(emergencyText('breath-hold limit reached')).toBe('Time limit, retracting')
    expect(emergencyText('stopped')).toBe('Stopping, retracting')
    expect(emergencyText('move timeout')).toBe('Retracting now')
    expect(emergencyText(null)).toBe('Retracting now')
  })
})

// ── Engine with a mocked speechSynthesis ─────────────────────────────────────

class FakeUtterance {
  voice: unknown = null
  rate = 1
  volume = 1
  lang = ''
  onend: (() => void) | null = null
  onerror: (() => void) | null = null
  constructor(public text: string) {}
}

function installSynth(voices: Partial<SpeechSynthesisVoice>[] = []) {
  const listeners = new Map<string, Set<() => void>>()
  const synth = {
    speak: vi.fn(),
    cancel: vi.fn(),
    getVoices: vi.fn(() => voices as SpeechSynthesisVoice[]),
    addEventListener: vi.fn((t: string, cb: () => void) => {
      if (!listeners.has(t)) listeners.set(t, new Set())
      listeners.get(t)!.add(cb)
    }),
    removeEventListener: vi.fn((t: string, cb: () => void) => { listeners.get(t)?.delete(cb) }),
    emit: (t: string) => listeners.get(t)?.forEach((cb) => cb()),
  }
  vi.stubGlobal('speechSynthesis', synth)
  vi.stubGlobal('SpeechSynthesisUtterance', FakeUtterance)
  return synth
}

const spoken = (synth: ReturnType<typeof installSynth>) =>
  synth.speak.mock.calls.map((c) => (c[0] as FakeUtterance).text)

describe('speech engine', () => {
  beforeEach(() => {
    resetSpeechState()
    usePreferencesStore.setState({ speechEnabled: true, speechVoiceURI: null, speechRate: 0.9, speechVolume: 1 })
  })
  afterEach(() => { vi.unstubAllGlobals() })

  it('defaults to on at a calm rate below 1', () => {
    localStorage.clear()
    expect(usePreferencesStore.getInitialState().speechEnabled).toBe(true)
    expect(usePreferencesStore.getInitialState().speechRate).toBeLessThan(1)
    expect(usePreferencesStore.getInitialState().speechRate).toBeGreaterThan(0.5)
  })

  it('cancels stale speech before every utterance (no backlog) and applies rate/volume/voice', () => {
    const voice = { voiceURI: 'v1', name: 'Calm', lang: 'en-GB' }
    const synth = installSynth([voice])
    usePreferencesStore.setState({ speechVoiceURI: 'v1', speechRate: 0.8, speechVolume: 0.5 })
    speakCue({ text: 'Warm-up, gentle strokes', critical: false }, 0)
    speakCue({ text: 'Rest, 20 seconds', critical: false }, 10)
    expect(spoken(synth)).toEqual(['Warm-up, gentle strokes', 'Rest, 20 seconds'])
    expect(synth.cancel).toHaveBeenCalledTimes(2)
    // cancel always happens before the matching speak
    expect(synth.cancel.mock.invocationCallOrder[1]).toBeLessThan(synth.speak.mock.invocationCallOrder[1])
    const u = synth.speak.mock.calls[1][0] as FakeUtterance
    expect(u).toMatchObject({ rate: 0.8, volume: 0.5, lang: 'en-GB', voice })
  })

  it('critical cues interrupt anything being spoken', () => {
    const synth = installSynth()
    speakCue({ text: 'Rest, 20 seconds', critical: false }, 0)
    speakCue({ text: 'Released, retracting', critical: true }, 5)
    expect(spoken(synth)).toEqual(['Rest, 20 seconds', 'Released, retracting'])
    expect(synth.cancel).toHaveBeenCalledTimes(2)
  })

  it('a normal cue does not cut off a critical one still being spoken, but speaks once it is over', () => {
    const synth = installSynth()
    speakCue({ text: 'Released, retracting', critical: true }, 0)
    speakCue({ text: 'Session ended', critical: false }, 500)
    expect(spoken(synth)).toEqual(['Released, retracting'])
    expect(synth.cancel).toHaveBeenCalledTimes(1)
    // finished normally
    ;(synth.speak.mock.calls[0][0] as FakeUtterance).onend?.()
    speakCue({ text: 'Session ended', critical: false }, 600)
    expect(spoken(synth)).toEqual(['Released, retracting', 'Session ended'])
  })

  it('does not hold the critical lock forever if the browser never fires end', () => {
    const synth = installSynth()
    speakCue({ text: 'Released, retracting', critical: true }, 0)
    speakCue({ text: 'Rest', critical: false }, CRITICAL_HOLD_MS - 1)
    expect(spoken(synth)).toEqual(['Released, retracting'])
    speakCue({ text: 'Rest', critical: false }, CRITICAL_HOLD_MS + 1)
    expect(spoken(synth)).toEqual(['Released, retracting', 'Rest'])
  })

  it('a failed critical utterance releases the lock', () => {
    const synth = installSynth()
    speakCue({ text: 'Released, retracting', critical: true }, 0)
    ;(synth.speak.mock.calls[0][0] as FakeUtterance).onerror?.()
    speakCue({ text: 'Rest', critical: false }, 100)
    expect(spoken(synth)).toEqual(['Released, retracting', 'Rest'])
  })

  it('speaks nothing when switched off', () => {
    const synth = installSynth()
    usePreferencesStore.setState({ speechEnabled: false })
    speakCue({ text: 'Released, retracting', critical: true })
    expect(synth.speak).not.toHaveBeenCalled()
    expect(synth.cancel).not.toHaveBeenCalled()
  })

  it('cancelStaleSpeech spares an in-flight critical cue; cancelSpeech does not', () => {
    const synth = installSynth()
    speakCue({ text: 'Released, retracting', critical: true }, 0)
    cancelStaleSpeech(100)
    expect(synth.cancel).toHaveBeenCalledTimes(1) // only the cancel inside speak
    cancelSpeech()
    expect(synth.cancel).toHaveBeenCalledTimes(2)
  })

  it('primes with an inaudible utterance and previews on request', () => {
    const synth = installSynth()
    primeSpeech()
    expect(synth.speak).toHaveBeenCalledTimes(1)
    const u = synth.speak.mock.calls[0][0] as FakeUtterance
    expect(u.text).toBe('')
    expect(u.volume).toBe(0)
    speakSample()
    expect(synth.speak).toHaveBeenCalledTimes(2)
  })

  it('lists English voices first-class and reacts to voiceschanged', () => {
    const synth = installSynth([
      { voiceURI: 'de', name: 'Anna', lang: 'de-DE' },
      { voiceURI: 'en', name: 'Sam', lang: 'en-US' },
    ])
    expect(listSpeechVoices().map((v) => v.voiceURI)).toEqual(['en'])
    const cb = vi.fn()
    const off = onSpeechVoicesChanged(cb)
    synth.emit('voiceschanged')
    expect(cb).toHaveBeenCalledTimes(1)
    off()
    synth.emit('voiceschanged')
    expect(cb).toHaveBeenCalledTimes(1)
  })

  it('falls back to all voices when none are English', () => {
    installSynth([{ voiceURI: 'de', name: 'Anna', lang: 'de-DE' }])
    expect(listSpeechVoices()).toHaveLength(1)
  })

  it('does nothing and never throws when speechSynthesis is unavailable', () => {
    vi.stubGlobal('speechSynthesis', undefined)
    expect(speechSupported()).toBe(false)
    expect(() => {
      speakCue({ text: 'x', critical: true })
      primeSpeech()
      speakSample()
      cancelSpeech()
      cancelStaleSpeech()
    }).not.toThrow()
    expect(listSpeechVoices()).toEqual([])
    expect(onSpeechVoicesChanged(() => {})).toBeTypeOf('function')
  })

  it('swallows errors thrown by speechSynthesis', () => {
    const synth = installSynth()
    synth.speak.mockImplementation(() => { throw new Error('boom') })
    synth.cancel.mockImplementation(() => { throw new Error('boom') })
    synth.getVoices.mockImplementation(() => { throw new Error('boom') })
    expect(() => {
      speakCue({ text: 'x', critical: true })
      primeSpeech()
      cancelSpeech()
    }).not.toThrow()
    expect(listSpeechVoices()).toEqual([])
  })
})
