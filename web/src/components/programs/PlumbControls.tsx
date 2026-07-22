import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

export function PlumbControls() {
  const { mode, plumb } = useStatus()
  const send = useSendCommand()
  const isActive = mode === 'plumb'
  const [showManual, setShowManual] = useState(false)

  const handingOff = plumb?.handingOff ?? false
  const targetMm = plumb?.targetMm ?? 0

  function toggleActive() {
    send({ type: isActive ? 'plumb_stop' : 'plumb_start' })
  }

  return (
    <div className="flex flex-col gap-6 p-4">
      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive ? (handingOff ? 'bg-amber-400' : 'bg-sky-400 animate-pulse') : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive
            ? handingOff ? 'Repositioning — move the rod by hand' : 'Oscillating at fixed speed'
            : 'Not running'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-sky-600 hover:bg-sky-500 text-white shadow-lg shadow-sky-500/20'
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
            Start Plumb
          </>
        )}
      </button>

      {/* Live status panel */}
      {isActive && (
        <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          <div className="flex items-center justify-between gap-2">
            <div className="flex flex-col">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Upper bound</span>
              <span className="text-base font-semibold text-slate-100 font-mono">{targetMm.toFixed(1)} mm</span>
            </div>
            <span
              className={`inline-flex items-center px-2.5 py-1 rounded-full text-xs font-semibold tracking-wider ${
                handingOff
                  ? 'bg-amber-500/20 text-amber-300 border border-amber-500/40'
                  : 'bg-sky-500/20 text-sky-300 border border-sky-500/30'
              }`}
            >
              {handingOff ? 'Hand-off' : 'Oscillating'}
            </span>
          </div>
          {handingOff && (
            <p className="text-[11px] text-amber-400/80">
              Servo released — move the rod to the desired depth, then let go of the switch to set it.
            </p>
          )}
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
            Steady oscillation between the calibrated work origin and an upper bound.
            <span className="text-slate-300"> Press and hold the hand switch</span> to drop the servo
            and reposition the rod by hand; release to capture the new upper bound and resume.
          </div>
        )}
      </div>
    </div>
  )
}
