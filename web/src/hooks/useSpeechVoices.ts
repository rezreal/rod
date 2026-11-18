import { useEffect, useState } from 'react'
import { listSpeechVoices, onSpeechVoicesChanged } from '../audio/speech'

/** Available speech voices; refreshes when the browser finishes loading them (`voiceschanged`). */
export function useSpeechVoices(): SpeechSynthesisVoice[] {
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>(() => listSpeechVoices())
  useEffect(() => {
    const refresh = () => setVoices(listSpeechVoices())
    refresh()
    return onSpeechVoicesChanged(refresh)
  }, [])
  return voices
}
