import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

export function TempoControls() {
  const { mode, tempo } = useStatus()
  const send = useSendCommand()
  const isActive = mode === 'tempo'
  const [showManual, setShowManual] = useState(false)
  const periodMs = tempo?.periodMs ?? 0
  const bpm = periodMs > 0 ? Math.round(60000 / periodMs) : 0

  function toggleActive() {
    send({ type: isActive ? 'tempo_stop' : 'tempo_start' })
  }

  return (
    <div className="flex flex-col gap-6 p-4">
      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive ? 'bg-amber-400 animate-pulse' : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive ? 'Tap a rhythm · hold to stop' : 'Not running — tap out a rhythm to begin'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-amber-600 hover:bg-amber-500 text-white shadow-lg shadow-amber-500/20'
          }`}
      >
        {isActive ? (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
              <rect x="6" y="6" width="12" height="12" rx="1" />
            </svg>
            Stop
          </>
        ) : (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor" stroke="none">
              <polygon points="6 4 20 12 6 20 6 4" />
            </svg>
            Start Tempo
          </>
        )}
      </button>

      {/* Live status panel */}
      {isActive && (
        <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-[10px] uppercase tracking-widest text-slate-500">Period</span>
            <span className="text-base font-semibold text-amber-200 font-mono">{periodMs > 0 ? `${periodMs} ms` : '— set by tapping'}</span>
          </div>
          <div className="flex items-center justify-between gap-2">
            <span className="text-[10px] uppercase tracking-widest text-slate-500">Tempo</span>
            <span className="text-base font-semibold text-amber-200 font-mono">{bpm > 0 ? `${bpm} BPM` : '—'}</span>
          </div>
        </div>
      )}

      {/* How it works */}
      <div className="flex flex-col gap-3 border-t border-slate-800 pt-4">
        <button
          onClick={() => setShowManual((v) => !v)}
          className="flex items-center justify-center gap-1.5 text-xs font-medium text-slate-400 hover:text-slate-200 transition-colors"
        >
          <svg
            viewBox="0 0 24 24"
            className={`w-3.5 h-3.5 transition-transform ${showManual ? 'rotate-180' : ''}`}
            fill="none" stroke="currentColor" strokeWidth={2}
          >
            <polyline points="6 9 12 15 18 9" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {showManual ? 'Hide how it works' : 'How it works'}
        </button>
        {showManual && (
          <div className="rounded-2xl bg-slate-800/40 border border-slate-700/50 p-4 text-xs text-slate-400 leading-relaxed">
            <span className="text-slate-300">Tap out a rhythm on the hand switch</span> — the interval between taps sets the stroke period, and oscillation follows your beat. A <span className="text-slate-300">long hold</span> stops it; if taps cease for a few cycles it auto-stops.
          </div>
        )}
      </div>
    </div>
  )
}
