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
  /** Depth Trainer spoken guidance (Web Speech API). Supplementary only. */
  speechEnabled: boolean
  setSpeechEnabled(enabled: boolean): void
  /** `voiceURI` of the chosen voice; null = browser default. */
  speechVoiceURI: string | null
  setSpeechVoiceURI(uri: string | null): void
  /** SpeechSynthesisUtterance.rate (0.5–1.5); default is calm, slightly below 1. */
  speechRate: number
  setSpeechRate(rate: number): void
  /** SpeechSynthesisUtterance.volume (0–1). */
  speechVolume: number
  setSpeechVolume(volume: number): void
}

export const usePreferencesStore = create<PreferencesStore>()(
  persist(
    (set) => ({
      audioFeedbackEnabled: true,
      setAudioFeedbackEnabled(enabled) { set({ audioFeedbackEnabled: enabled }) },
      advancedModeEnabled: false,
      setAdvancedModeEnabled(enabled) { set({ advancedModeEnabled: enabled }) },
      speechEnabled: true,
      setSpeechEnabled(enabled) { set({ speechEnabled: enabled }) },
      speechVoiceURI: null,
      setSpeechVoiceURI(uri) { set({ speechVoiceURI: uri }) },
      speechRate: 0.9,
      setSpeechRate(rate) { set({ speechRate: rate }) },
      speechVolume: 1,
      setSpeechVolume(volume) { set({ speechVolume: volume }) },
    }),
    { name: 'rod-preferences' },
  ),
)
