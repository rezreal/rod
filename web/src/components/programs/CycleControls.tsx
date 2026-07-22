import { useEffect, useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { CycleManual } from './CycleManual'

export function CycleControls() {
  const { mode, cycle } = useStatus()
  const send = useSendCommand()

  const isActive = mode === 'cycle'

  const [showManual, setShowManual] = useState(false)
  const [buttonHeld, setButtonHeld] = useState(false)

  // Stable ref so callbacks always call the latest send().
  const sendRef = useRef(send)
  sendRef.current = send

  function pressDown(e: React.PointerEvent) {
    e.currentTarget.setPointerCapture(e.pointerId)
    setButtonHeld(true)
    sendRef.current({ type: 'cycle_button', down: true })
  }

  function pressUp() {
    if (!buttonHeld) return
    setButtonHeld(false)
    sendRef.current({ type: 'cycle_button', down: false })
  }

  // Release the button if the program exits while the press is held.
  useEffect(() => {
    if (!isActive && buttonHeld) {
      setButtonHeld(false)
      sendRef.current({ type: 'cycle_button', down: false })
    }
  }, [isActive, buttonHeld])

  function toggleActive() {
    if (isActive) {
      send({ type: 'cycle_stop' })
    } else {
      send({ type: 'cycle_start' })
    }
  }

  const paused = cycle?.paused ?? false
  const patternName = cycle?.patternName ?? '—'
  const patternIndex = cycle?.pattern ?? 0
  const patternCount = cycle?.patternCount ?? 0

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive ? (paused ? 'bg-amber-400' : 'bg-teal-400 animate-pulse') : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive
            ? paused ? 'Paused' : 'Playing pattern playlist'
            : 'No cycle running'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-teal-600 hover:bg-teal-500 text-white shadow-lg shadow-teal-500/20'
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
            Start Cycle
          </>
        )}
      </button>

      {/* Press button — only while active */}
      {isActive && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-slate-500 text-center">
            Tap = next pattern · Hold 2s = pause
          </p>

          <button
            onPointerDown={pressDown}
            onPointerUp={pressUp}
            onPointerCancel={pressUp}
            onPointerLeave={pressUp}
            style={{ touchAction: 'none', userSelect: 'none' }}
            className={`relative flex items-center justify-center w-full py-8 rounded-2xl font-bold text-lg
              select-none transition-all duration-75
              ${buttonHeld
                ? 'bg-teal-500 text-white scale-[0.97] shadow-inner shadow-teal-700'
                : 'bg-slate-700 hover:bg-slate-600 text-slate-200 shadow-lg'
              }`}
          >
            {buttonHeld && (
              <span className="absolute inset-0 rounded-2xl border-2 border-teal-400 animate-ping opacity-30" />
            )}

            <span className="flex flex-col items-center gap-1.5 relative">
              <svg viewBox="0 0 24 24" className="w-7 h-7" fill="none" stroke="currentColor" strokeWidth={2}>
                <circle cx="12" cy="12" r="9" />
                <circle cx="12" cy="12" r="3" fill="currentColor" stroke="none" />
              </svg>
              <span>{buttonHeld ? 'Pressing…' : 'Press'}</span>
            </span>
          </button>
        </div>
      )}

      {/* Live status panel */}
      {isActive && (
        <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          {/* Current pattern + play/pause */}
          <div className="flex items-center justify-between gap-2">
            <div className="flex flex-col">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Pattern</span>
              <span className="text-base font-semibold text-slate-100">
                {patternName}
                {patternCount > 0 && (
                  <span className="ml-1.5 text-xs font-mono font-normal text-slate-400">
                    ({patternIndex + 1} / {patternCount})
                  </span>
                )}
              </span>
            </div>
            <span
              className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-semibold tracking-wider ${
                paused
                  ? 'bg-amber-500/20 text-amber-300 border border-amber-500/40'
                  : 'bg-teal-500/20 text-teal-300 border border-teal-500/30'
              }`}
            >
              {paused ? (
                <svg viewBox="0 0 24 24" className="w-3 h-3" fill="currentColor" stroke="none">
                  <rect x="6" y="5" width="4" height="14" rx="1" />
                  <rect x="14" y="5" width="4" height="14" rx="1" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" className="w-3 h-3" fill="currentColor" stroke="none">
                  <polygon points="6 4 20 12 6 20 6 4" />
                </svg>
              )}
              {paused ? 'Paused' : 'Playing'}
            </span>
          </div>

          {paused && (
            <p className="text-[11px] text-amber-400/80">
              Motion is paused — hold the button 2 seconds to resume.
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
          <div className="rounded-2xl bg-slate-800/40 border border-slate-700/50 p-4">
            <CycleManual current={isActive ? cycle?.pattern : undefined} />
          </div>
        )}
      </div>

    </div>
  )
}
