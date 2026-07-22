import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

export function SurgeControls() {
  const { mode, surge } = useStatus()
  const send = useSendCommand()
  const isActive = mode === 'surge'
  const [showManual, setShowManual] = useState(false)
  const arousal = surge?.arousal ?? 0

  function toggleActive() {
    send({ type: isActive ? 'surge_stop' : 'surge_start' })
  }

  return (
    <div className="flex flex-col gap-6 p-4">
      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive ? 'bg-rose-400 animate-pulse' : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive ? 'Hold the switch to build intensity' : 'Not running'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-rose-600 hover:bg-rose-500 text-white shadow-lg shadow-rose-500/20'
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
            Start Surge
          </>
        )}
      </button>

      {/* Live status panel */}
      {isActive && (
        <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between gap-2">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Arousal</span>
              <span className="text-base font-semibold text-rose-200 font-mono">{Math.round(arousal * 100)}%</span>
            </div>
            <div className="h-2 rounded-full bg-slate-700 overflow-hidden">
              <div className="h-full bg-rose-500 transition-[width] duration-200" style={{ width: `${Math.round(arousal * 100)}%` }} />
            </div>
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
            Arousal-driven oscillation. <span className="text-slate-300">Hold the hand switch</span> to build intensity — strokes get faster and reach deeper as arousal climbs; release to let it ebb back down.
          </div>
        )}
      </div>
    </div>
  )
}
