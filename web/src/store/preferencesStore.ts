/**
 * Client-side user preferences — persisted to localStorage, independent of
 * live device state (see deviceStore.ts).
 */
import { create } from 'zustand'
import { persist } from 'zustand/middleware'

interface PreferencesStore {
  audioFeedbackEnabled: boolean
  setAudioFeedbackEnabled(enabled: boolean): void
  /** Shows per-pattern tuning controls (speed/intensity/reps/pause) in Cycle mode. */
  advancedModeEnabled: boolean
  setAdvancedModeEnabled(enabled: boolean): void
}

export const usePreferencesStore = create<PreferencesStore>()(
  persist(
    (set) => ({
      audioFeedbackEnabled: true,
      setAudioFeedbackEnabled(enabled) { set({ audioFeedbackEnabled: enabled }) },
      advancedModeEnabled: false,
      setAdvancedModeEnabled(enabled) { set({ advancedModeEnabled: enabled }) },
    }),
    { name: 'rod-preferences' },
  ),
)
