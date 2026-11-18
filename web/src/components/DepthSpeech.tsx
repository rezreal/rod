import { useEffect, useRef } from 'react'
import { useStatus } from '../hooks/useDeviceState'
import type { DepthTrainerState } from '../types/sscp'
import { depthCue } from '../lib/depthSpeech'
import { cancelStaleSpeech, speakCue } from '../audio/speech'

/**
 * Invisible watcher: speaks short guidance on Depth Trainer transitions via
 * the Web Speech API. Purely an observer of status snapshots — it sends no
 * commands and gates nothing; the tone cues in AudioFeedback are independent.
 */
export function DepthSpeech() {
  const { depthTrainer } = useStatus()
  const prevRef = useRef<DepthTrainerState | undefined>(undefined)
  const initializedRef = useRef(false)

  useEffect(() => {
    const prev = prevRef.current
    prevRef.current = depthTrainer

    // First snapshot after (re)connecting: nothing to diff against, and a
    // reconnect into a running session shouldn't re-announce it.
    if (!initializedRef.current) {
      initializedRef.current = true
      return
    }

    if (!depthTrainer) {
      // Left the mode: drop stale speech (but let an in-flight safety cue finish).
      if (prev) cancelStaleSpeech()
      return
    }

    const cue = depthCue(prev, depthTrainer)
    if (cue) speakCue(cue)
  }, [depthTrainer])

  useEffect(() => () => cancelStaleSpeech(), [])

  return null
}
