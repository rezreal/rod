import { primeSpeech, speakSample, speechSupported } from '../../audio/speech'
import { useSpeechVoices } from '../../hooks/useSpeechVoices'
import { usePreferencesStore } from '../../store/preferencesStore'

/**
 * Spoken-guidance settings for Depth Trainer (Web Speech API): on/off, voice,
 * rate and volume. Persisted per viewer in the preferences store. Speech is
 * supplementary — it never affects the rod, the safety logic or the tones.
 */
export function DepthSpeechSettings() {
  const enabled = usePreferencesStore((s) => s.speechEnabled)
  const setEnabled = usePreferencesStore((s) => s.setSpeechEnabled)
  const voiceURI = usePreferencesStore((s) => s.speechVoiceURI)
  const setVoiceURI = usePreferencesStore((s) => s.setSpeechVoiceURI)
  const rate = usePreferencesStore((s) => s.speechRate)
  const setRate = usePreferencesStore((s) => s.setSpeechRate)
  const volume = usePreferencesStore((s) => s.speechVolume)
  const setVolume = usePreferencesStore((s) => s.setSpeechVolume)
  const voices = useSpeechVoices()
  const supported = speechSupported()

  return (
    <div className="flex flex-col gap-3 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <span className="text-[10px] uppercase tracking-widest text-slate-500">Spoken guidance</span>
          <p className="text-[11px] text-slate-500 leading-snug mt-1">
            {supported
              ? 'Short spoken cues for each phase and countdown. The tones still play; the hand switch still rules.'
              : 'Not available in this browser.'}
          </p>
        </div>
        <button
          onClick={() => {
            const next = !enabled
            setEnabled(next)
            if (next) primeSpeech()
          }}
          disabled={!supported}
          aria-pressed={enabled && supported}
          aria-label="Spoken guidance"
          className={`relative shrink-0 w-11 h-6 rounded-full transition-colors disabled:opacity-40 ${
            enabled && supported ? 'bg-purple-600' : 'bg-slate-700'
          }`}
        >
          <span
            className={`absolute top-0.5 left-0.5 w-5 h-5 bg-white rounded-full transition-transform ${
              enabled && supported ? 'translate-x-5' : ''
            }`}
          />
        </button>
      </div>

      {supported && enabled && (
        <>
          <label className="flex flex-col gap-1">
            <span className="text-xs text-slate-400">Voice</span>
            <select
              value={voiceURI ?? ''}
              onChange={(e) => setVoiceURI(e.target.value || null)}
              className="rounded-lg bg-slate-900 border border-slate-600 px-2 py-1.5 text-sm text-slate-100 focus:outline-none focus:border-purple-400"
            >
              <option value="">Browser default</option>
              {/* A saved voice that is not (yet) in the list stays selectable so it isn't silently dropped. */}
              {voiceURI && !voices.some((v) => v.voiceURI === voiceURI) && (
                <option value={voiceURI}>{voiceURI} (unavailable)</option>
              )}
              {voices.map((v) => (
                <option key={v.voiceURI} value={v.voiceURI}>{v.name} ({v.lang})</option>
              ))}
            </select>
          </label>
          <SliderRow label="Rate" value={rate} min={0.5} max={1.5} step={0.05} format={(v) => `${v.toFixed(2)}×`} onChange={setRate} />
          <SliderRow label="Volume" value={volume} min={0} max={1} step={0.05} format={(v) => `${Math.round(v * 100)}%`} onChange={setVolume} />
          <button
            onClick={speakSample}
            className="py-2 rounded-xl text-xs font-semibold border border-slate-600 text-slate-200 hover:bg-slate-700 transition-colors"
          >
            Test voice
          </button>
        </>
      )}
    </div>
  )
}

function SliderRow({
  label, value, min, max, step, format, onChange,
}: {
  label: string
  value: number
  min: number
  max: number
  step: number
  format: (v: number) => string
  onChange: (v: number) => void
}) {
  return (
    <label className="flex items-center gap-3 text-xs text-slate-400">
      <span className="w-14">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="flex-1 accent-purple-500"
      />
      <span className="w-12 text-right font-mono text-slate-300">{format(value)}</span>
    </label>
  )
}
