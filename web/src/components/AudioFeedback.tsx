import { useEffect, useRef } from 'react'
import { useStatus } from '../hooks/useDeviceState'
import type { StatusState } from '../store/deviceStore'
import { usePreferencesStore } from '../store/preferencesStore'
import { playFault, playInputNeeded, playMistake, playSuccess } from '../audio/tones'

/**
 * Invisible watcher: diffs consecutive status snapshots and plays a cue on
 * the transitions that map to "input needed" / "success" / "fault" /
 * "mistake". Faults (e-stop, alarm, actuator disconnected) are hardware/setup
 * problems and get a distinctly harsher sound than mistakes (game slip),
 * which are normal gameplay feedback. Cycle/Hamp/Pulse/Ramp have no such
 * signal in their state today, so they're left uncovered rather than firing a
 * cue on an arbitrary field change.
 */
export function AudioFeedback() {
  const status = useStatus()
  const enabled = usePreferencesStore((s) => s.audioFeedbackEnabled)
  const prevRef = useRef<StatusState | null>(null)
  const initializedRef = useRef(false)

  useEffect(() => {
    const prev = prevRef.current
    prevRef.current = status

    // Skip the first snapshot after (re)connecting — nothing to diff against,
    // and we don't want a cue firing just because the device connected in an
    // already-alarmed or already-disconnected state.
    if (!initializedRef.current) {
      initializedRef.current = true
      return
    }
    if (!enabled || !prev) return

    if (!prev.emergencyStop && status.emergencyStop) { playFault(); return }
    if (prev.alarmCode === 0 && status.alarmCode !== 0) { playFault(); return }
    if (prev.actuatorConnected && !status.actuatorConnected) { playFault(); return }

    if (prev.game && status.game) {
      if (prev.game.phase !== 'slip' && status.game.phase === 'slip') { playMistake(); return }
      if (status.game.level > prev.game.level) { playSuccess(); return }
    }

    if (prev.impale && status.impale) {
      if (!prev.impale.waiting && status.impale.waiting) { playInputNeeded(); return }
      // Reaching the retract deadline without an explicit stop is the win
      // condition for a hold cycle, not a mistake — see ImpaleRuntime::won.
      if (!prev.impale.won && status.impale.won) { playSuccess(); return }
    }

    if (prev.learn && status.learn) {
      if (prev.learn.phase === 'recording' && status.learn.phase === 'ready') { playSuccess(); return }
    }
  }, [status, enabled])

  return null
}
