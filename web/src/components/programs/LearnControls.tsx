import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import type { LearnPhase } from '../../types/sscp'

const PHASES: { id: LearnPhase; label: string }[] = [
  { id: 'armed',     label: 'Armed' },
  { id: 'recording', label: 'Recording' },
  { id: 'ready',     label: 'Ready' },
  { id: 'playing',   label: 'Playing' },
]

// Label for the single big action button, per phase.
const ACTION_LABEL: Record<LearnPhase, string> = {
  armed:     'Start recording',
  recording: 'Stop recording',
  ready:     'Play imitation',
  playing:   'Record new',
}

// Short hint describing what's happening in each phase.
const PHASE_HINT: Record<LearnPhase, string> = {
  armed:     'Servo off — move the rod freely. Tap to start recording.',
  recording: "Move the tip by hand — it's being recorded. Tap to stop.",
  ready:     'Motion reduced to support points. Tap to play it back.',
  playing:   'Repeating the recorded motion on a loop. Tap to teach a new one.',
}

export function LearnControls() {
  const { mode, learn } = useStatus()
  const send = useSendCommand()

  const isActive = mode === 'learn'
  const phase: LearnPhase = learn?.phase ?? 'armed'
  const points = learn?.points ?? 0
  const waypoints = learn?.waypoints ?? 0

  const [showHelp, setShowHelp] = useState(false)

  function toggleActive() {
    if (isActive) {
      send({ type: 'learn_stop' })
    } else {
      send({ type: 'learn_start' })
    }
  }

  function tap() {
    send({ type: 'learn_button' })
  }

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive
              ? phase === 'recording' || phase === 'playing'
                ? 'bg-lime-400 animate-pulse'
                : 'bg-lime-400'
              : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive ? PHASE_HINT[phase] : 'Learn not running'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-lime-600 hover:bg-lime-500 text-white shadow-lg shadow-lime-500/20'
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
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
              <circle cx="12" cy="12" r="9" />
              <circle cx="12" cy="12" r="3.5" fill="currentColor" stroke="none" />
            </svg>
            Start Learn
          </>
        )}
      </button>

      {isActive && (
        <>
          {/* Phase step indicator */}
          <div className="flex items-center gap-1.5">
            {PHASES.map((p, i) => {
              const activeStep = p.id === phase
              const doneStep = PHASES.findIndex((x) => x.id === phase) > i
              return (
                <div key={p.id} className="flex-1 flex flex-col items-center gap-1.5">
                  <div
                    className={`h-1.5 w-full rounded-full transition-colors ${
                      activeStep
                        ? 'bg-lime-400'
                        : doneStep
                        ? 'bg-lime-700'
                        : 'bg-slate-700'
                    }`}
                  />
                  <span
                    className={`text-[10px] font-medium tracking-wide ${
                      activeStep ? 'text-lime-300' : 'text-slate-500'
                    }`}
                  >
                    {p.label}
                  </span>
                </div>
              )
            })}
          </div>

          {/* Phase pill + counts */}
          <div className="flex flex-col gap-3 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
            <div className="flex items-center justify-between gap-2">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Phase</span>
              <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-semibold tracking-wider bg-lime-500/20 text-lime-300 border border-lime-500/30">
                {PHASES.find((p) => p.id === phase)?.label ?? phase}
              </span>
            </div>

            {phase === 'recording' && (
              <div className="flex items-baseline justify-between">
                <span className="text-xs text-slate-400">Captured</span>
                <span className="text-sm font-mono font-semibold text-lime-300">
                  {points} samples
                </span>
              </div>
            )}

            {(phase === 'ready' || phase === 'playing') && (
              <div className="flex items-baseline justify-between">
                <span className="text-xs text-slate-400">Support points</span>
                <span className="text-sm font-mono font-semibold text-lime-300">
                  {waypoints} support points
                </span>
              </div>
            )}

            <p className="text-[11px] text-slate-500">{PHASE_HINT[phase]}</p>
          </div>

          {/* Single big action button — one tap per phase */}
          <button
            onClick={tap}
            className="relative flex items-center justify-center w-full py-8 rounded-2xl font-bold text-lg
              select-none transition-all duration-75
              bg-lime-600 hover:bg-lime-500 active:scale-[0.97] text-white shadow-lg shadow-lime-500/20"
          >
            <span className="flex flex-col items-center gap-1.5">
              <svg viewBox="0 0 24 24" className="w-7 h-7" fill="none" stroke="currentColor" strokeWidth={2}>
                <circle cx="12" cy="12" r="9" />
                <circle cx="12" cy="12" r="3.5" fill="currentColor" stroke="none" />
              </svg>
              <span>{ACTION_LABEL[phase]}</span>
            </span>
          </button>
        </>
      )}

      {/* How it works */}
      <div className="flex flex-col gap-3 border-t border-slate-800 pt-4">
        <button
          onClick={() => setShowHelp((v) => !v)}
          className="flex items-center justify-center gap-1.5 text-xs font-medium text-slate-400 hover:text-slate-200 transition-colors"
        >
          <svg
            viewBox="0 0 24 24"
            className={`w-3.5 h-3.5 transition-transform ${showHelp ? 'rotate-180' : ''}`}
            fill="none" stroke="currentColor" strokeWidth={2}
          >
            <polyline points="6 9 12 15 18 9" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {showHelp ? 'Hide how it works' : 'How it works'}
        </button>

        {showHelp && (
          <div className="rounded-2xl bg-slate-800/40 border border-slate-700/50 p-4">
            <p className="text-xs leading-relaxed text-slate-400">
              Tap to record, move the rod by hand, tap to stop. The motion is
              reduced to a handful of support points. Tap to play and the machine
              repeats it on a loop — tap again to teach a new one. While recording
              and armed the rod moves freely (servo off).
            </p>
          </div>
        )}
      </div>

    </div>
  )
}
