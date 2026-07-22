/**
 * Synthesized non-visual feedback cues (Web Audio API — no sound assets).
 * Lazily creates a single shared AudioContext on first use; by the time any
 * cue fires the user has already interacted with the page (BLE connect,
 * mode controls), so autoplay restrictions are a non-issue in practice.
 */

let ctx: AudioContext | null = null

function getContext(): AudioContext {
  if (!ctx) ctx = new AudioContext()
  if (ctx.state === 'suspended') void ctx.resume()
  return ctx
}

function tone(startOffsetS: number, freqHz: number, durationS: number, type: OscillatorType, peakGain: number) {
  const audioCtx = getContext()
  const start = audioCtx.currentTime + startOffsetS

  const osc = audioCtx.createOscillator()
  osc.type = type
  osc.frequency.value = freqHz

  const gain = audioCtx.createGain()
  gain.gain.setValueAtTime(0, start)
  gain.gain.linearRampToValueAtTime(peakGain, start + 0.015)
  gain.gain.linearRampToValueAtTime(0, start + durationS)

  osc.connect(gain).connect(audioCtx.destination)
  osc.start(start)
  osc.stop(start + durationS + 0.02)
}

/** Like `tone()`, but the oscillator's pitch sweeps from startFreq to endFreq. */
function toneSweep(startOffsetS: number, startFreqHz: number, endFreqHz: number, durationS: number, type: OscillatorType, peakGain: number) {
  const audioCtx = getContext()
  const start = audioCtx.currentTime + startOffsetS

  const osc = audioCtx.createOscillator()
  osc.type = type
  osc.frequency.setValueAtTime(startFreqHz, start)
  osc.frequency.linearRampToValueAtTime(endFreqHz, start + durationS)

  const gain = audioCtx.createGain()
  gain.gain.setValueAtTime(0, start)
  gain.gain.linearRampToValueAtTime(peakGain, start + 0.015)
  gain.gain.linearRampToValueAtTime(0, start + durationS)

  osc.connect(gain).connect(audioCtx.destination)
  osc.start(start)
  osc.stop(start + durationS + 0.02)
}

/** A single soft chime — the user is expected to act now. */
export function playInputNeeded() {
  tone(0, 660, 0.14, 'sine', 0.12)
}

/** A quick ascending arpeggio — something finished well / a checkpoint landed. */
export function playSuccess() {
  tone(0, 523.25, 0.1, 'sine', 0.12)
  tone(0.09, 659.25, 0.1, 'sine', 0.12)
  tone(0.18, 783.99, 0.14, 'sine', 0.12)
}

/**
 * Two harsh, insistent low pulses — a hardware/setup problem (e-stop, alarm,
 * actuator disconnected). Deliberately alarm-like: this needs the user's
 * attention on the physical setup, not just "try again".
 */
export function playFault() {
  tone(0, 196, 0.11, 'square', 0.13)
  tone(0.15, 196, 0.11, 'square', 0.13)
}

/**
 * A single downward buzz — the player slipped up / missed a window. Softer
 * and shorter than playFault(): this is normal gameplay feedback, not an
 * error condition.
 */
export function playMistake() {
  toneSweep(0, 320, 160, 0.2, 'triangle', 0.13)
}
