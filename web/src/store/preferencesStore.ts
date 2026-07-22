/**
 * Client-side user preferences — persisted to localStorage, independent of
 * live device state (see deviceStore.ts).
 */
import { create } from 'zustand'
import { persist } from 'zustand/middleware'

interface PreferencesStore {
  audioFeedbackEnabled: boolean
  setAudioFeedbackEnabled(enabled: boolean): void
}

export const usePreferencesStore = create<PreferencesStore>()(
  persist(
    (set) => ({
      audioFeedbackEnabled: true,
      setAudioFeedbackEnabled(enabled) { set({ audioFeedbackEnabled: enabled }) },
    }),
    { name: 'rod-preferences' },
  ),
)
