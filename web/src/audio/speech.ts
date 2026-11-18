/**
 * Spoken guidance via the Web Speech API (`speechSynthesis`).
 *
 * Speech is SUPPLEMENTARY. Every function here is fire-and-forget, never
 * throws, never returns something callers must wait for, and silently does
 * nothing when `speechSynthesis` is missing or misbehaves — so motion, safety
 * logic and the Web Audio tones (`tones.ts`) behave exactly as without it.
 *
 * Queue policy: there is no backlog. Every new cue cancels whatever is queued
 * or being spoken. A normal cue never cuts off a critical one that is still
 * being spoken (it is simply dropped — it would be stale anyway).
 */
import { usePreferencesStore } from '../store/preferencesStore'
import type { SpeechCue } from '../lib/depthSpeech'

/** Upper bound on how long a critical cue is treated as "still speaking" if the browser never fires `end`/`error`. */
export const CRITICAL_HOLD_MS = 3000

let criticalUntil = 0
let activeToken = 0

function synth(): SpeechSynthesis | null {
  try {
    const s = (globalThis as { speechSynthesis?: SpeechSynthesis }).speechSynthesis
    return s ?? null
  } catch {
    return null
  }
}

function makeUtterance(text: string): SpeechSynthesisUtterance | null {
  try {
    const Ctor = (globalThis as { SpeechSynthesisUtterance?: typeof SpeechSynthesisUtterance }).SpeechSynthesisUtterance
    return Ctor ? new Ctor(text) : null
  } catch {
    return null
  }
}

export function speechSupported(): boolean {
  return synth() !== null && makeUtterance('') !== null
}

/** Voices that can read our (English) phrases; all voices if none are English. */
export function listSpeechVoices(): SpeechSynthesisVoice[] {
  try {
    const all = synth()?.getVoices() ?? []
    const en = all.filter((v) => v.lang.toLowerCase().startsWith('en'))
    return en.length > 0 ? en : all
  } catch {
    return []
  }
}

/**
 * Call the callback whenever the voice list may have changed (browsers load
 * voices asynchronously and fire `voiceschanged`). Returns an unsubscribe.
 */
export function onSpeechVoicesChanged(cb: () => void): () => void {
  const s = synth()
  if (!s || typeof s.addEventListener !== 'function') return () => {}
  try {
    s.addEventListener('voiceschanged', cb)
  } catch {
    return () => {}
  }
  return () => {
    try { s.removeEventListener('voiceschanged', cb) } catch { /* ignore */ }
  }
}

/** Stop anything queued or being spoken. */
export function cancelSpeech() {
  activeToken++
  criticalUntil = 0
  try { synth()?.cancel() } catch { /* ignore */ }
}

/** Like `cancelSpeech`, but leaves a safety-critical cue that is still being spoken alone. */
export function cancelStaleSpeech(now: number = Date.now()) {
  if (now < criticalUntil) return
  cancelSpeech()
}

/**
 * Browsers generally refuse to speak until the page has had a user gesture
 * (iOS Safari strictly). Call this from the Start / confirm click handler: it
 * speaks an inaudible empty utterance and touches the voice list.
 */
export function primeSpeech() {
  const s = synth()
  const u = makeUtterance('')
  if (!s || !u) return
  try {
    u.volume = 0
    s.speak(u)
    s.getVoices()
  } catch { /* ignore */ }
}

function buildUtterance(text: string): SpeechSynthesisUtterance | null {
  const { speechVoiceURI, speechRate, speechVolume } = usePreferencesStore.getState()
  const u = makeUtterance(text)
  if (!u) return null
  u.rate = Math.min(1.5, Math.max(0.5, speechRate))
  u.volume = Math.min(1, Math.max(0, speechVolume))
  u.lang = 'en'
  if (speechVoiceURI) {
    const voice = (() => {
      try { return synth()?.getVoices().find((v) => v.voiceURI === speechVoiceURI) } catch { return undefined }
    })()
    if (voice) {
      u.voice = voice
      u.lang = voice.lang
    }
  }
  return u
}

function speakNow(text: string, critical: boolean, now: number) {
  const s = synth()
  const u = buildUtterance(text)
  if (!s || !u) return
  const token = ++activeToken
  if (critical) criticalUntil = now + CRITICAL_HOLD_MS
  const done = () => {
    if (token === activeToken) criticalUntil = 0
  }
  u.onend = done
  u.onerror = done
  try {
    s.cancel()
    s.speak(u)
  } catch { /* ignore */ }
}

/**
 * Speak a cue (no-op when speech is switched off or unavailable). Critical
 * cues interrupt everything; normal cues replace other normal speech but are
 * dropped while a critical cue is still being spoken.
 */
export function speakCue(cue: SpeechCue, now: number = Date.now()) {
  try {
    if (!usePreferencesStore.getState().speechEnabled) return
    if (!cue.critical && now < criticalUntil) return
    speakNow(cue.text, cue.critical, now)
  } catch { /* speech must never affect anything else */ }
}

/** Preview the chosen voice/rate/volume (a user gesture, so it also primes). Ignores the on/off toggle. */
export function speakSample() {
  try { speakNow('Warm-up, gentle strokes', false, Date.now()) } catch { /* ignore */ }
}

/** Test hook: reset module state. */
export function resetSpeechState() {
  criticalUntil = 0
  activeToken = 0
}
