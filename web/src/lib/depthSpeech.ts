import type { DepthTrainerState } from '../types/sscp'

/**
 * Spoken guidance for Depth Trainer: a pure mapping from two consecutive
 * `depthTrainer` status snapshots to at most ONE short cue. No side effects and
 * no knowledge of `speechSynthesis` — see `audio/speech.ts` for the engine.
 *
 * Speech is supplementary only. Nothing here (or in the engine) can influence
 * the rod: the bridge owns all motion and safety logic.
 */
export interface SpeechCue {
  text: string
  /** Safety-critical: interrupts anything being spoken and is never dropped for a normal cue. */
  critical: boolean
}

const NUMBER_WORDS = ['', 'One', 'Two', 'Three']

/** Spoken ones place for small counts; falls back to digits. */
function say(n: number): string {
  return NUMBER_WORDS[n] || String(n)
}

/** A duration as natural spoken words: "20 seconds", "1 minute 30 seconds". */
function spokenSeconds(s: number): string {
  const whole = Math.max(1, Math.round(s))
  if (whole < 60) return `${whole} second${whole === 1 ? '' : 's'}`
  const min = Math.floor(whole / 60)
  const rem = whole % 60
  const m = `${min} minute${min === 1 ? '' : 's'}`
  return rem === 0 ? m : `${m} ${rem} seconds`
}

/** Spoken text for an emergency retract, from the bridge's `emergencyReason`. */
export function emergencyText(reason: string | null): string {
  switch (reason) {
    case 'hand switch released': return 'Released, retracting'
    case 'breath-hold limit reached': return 'Time limit, retracting'
    case 'stopped': return 'Stopping, retracting'
    default: return 'Retracting now'
  }
}

/** Whole-second countdown value (3, 2, 1) for a `cueSoonS` reading, or null. */
function countdownValue(cueSoonS: number | null | undefined): number | null {
  if (cueSoonS === null || cueSoonS === undefined || cueSoonS <= 0 || cueSoonS > 3) return null
  return Math.min(3, Math.ceil(cueSoonS))
}

/**
 * The cue (if any) to speak when status moves from `prev` to `next`.
 * `prev === undefined` means this is the first depth snapshot of a session.
 * Priority when several things change at once: emergency, countdown, warm-up
 * gate, phase change, hand-switch prompt.
 */
export function depthCue(prev: DepthTrainerState | undefined, next: DepthTrainerState): SpeechCue | null {
  // 1. Emergency retract / deadman lapse — always first and critical.
  if (next.emergency && !prev?.emergency) {
    return { text: emergencyText(next.emergencyReason), critical: true }
  }
  // While retracting under emergency nothing else is worth saying.
  if (next.emergency) return null

  // 2. Countdowns. `cueSoonS` is the rod-about-to-move countdown while armed
  //    with the switch held (critical: the rod is about to go out), and the
  //    seconds left in a hold otherwise (the rod is about to come back).
  const n = countdownValue(next.cueSoonS)
  if (n !== null && n !== countdownValue(prev?.cueSoonS)) {
    if (next.phase === 'arming') return { text: n === 3 ? `Moving in ${say(n)}` : say(n), critical: true }
    if (next.phase === 'hold') return { text: n === 3 ? `Coming out in ${say(n)}` : say(n), critical: false }
  }

  // 3. Warm-up gate: parked, waiting for the user's decision.
  if (next.awaitingWarmupConfirm && !prev?.awaitingWarmupConfirm) {
    return { text: 'Warm-up done. Continue when you are ready', critical: false }
  }

  // 4. Phase changes.
  if (prev && prev.phase !== next.phase) {
    switch (next.phase) {
      case 'warmup': return { text: 'Warm-up, gentle strokes', critical: false }
      case 'oscillation': return { text: setText('Strokes', next), critical: false }
      case 'hold': return { text: setText('Hold', next), critical: false }
      case 'rest': {
        const s = next.restTotalS || next.restRemainingS
        return { text: s > 0 ? `Rest, ${spokenSeconds(s)}` : 'Rest', critical: false }
      }
      case 'cooldown': return { text: 'Cool-down, easy strokes', critical: false }
      case 'rating': return { text: 'Session done. How did that feel?', critical: false }
      case 'ended': return { text: 'Session ended', critical: false }
      default: break // 'arming' / non-emergency 'retracting': stay quiet
    }
  }

  // 5. Warm-up time check, once, as it crosses 30 s.
  if (
    next.phase === 'warmup' && prev?.phase === 'warmup' &&
    prev.warmupRemainingS !== null && next.warmupRemainingS !== null &&
    prev.warmupRemainingS > 30 && next.warmupRemainingS <= 30 && next.warmupRemainingS > 0
  ) {
    return { text: 'Thirty seconds left', critical: false }
  }

  // 6. The rod is waiting for the hand switch.
  if (next.awaitingDeadman && !prev?.awaitingDeadman) {
    return {
      text: prev ? 'Breathe. Hold the switch when ready' : 'Armed. Hold the switch when ready',
      critical: false,
    }
  }

  return null
}

function setText(label: string, s: DepthTrainerState): string {
  return s.setCount > 0 ? `${label}, set ${s.setIndex} of ${s.setCount}` : label
}
