import { useEffect, useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { useDeviceStore } from '../../store/deviceStore'
import type { GameKind, GamePhase } from '../../types/sscp'
import { GAME_DOCS, GamesManual } from './GamesManual'

// Interval between deadman heartbeat pulses (ms). Must stay well under the
// bridge's 150 ms deadman window so the game never lapses mid-hold.
const BUTTON_INTERVAL_MS = 50

// Hardware taps required to arm a game, mirroring the bridge's default
// `ready_taps` (src/config.rs). Only the physical hand switch counts — this
// screen can't fake the gesture, which is the point.
const READY_TAPS = 3

const GAME_NAMES: Record<GameKind, string> = {
  edge_recover:  'Edge & Recover',
  hold_the_line: 'Hold the Line',
  gauntlet:      'The Gauntlet',
  deadmans_climb: "Deadman's Climb",
  stillness:     'Stillness',
}

/** Per-game label for the `level` metric (— means not used). */
const LEVEL_LABELS: Record<GameKind, string> = {
  edge_recover:  'edges',
  hold_the_line: 'lines lost',
  gauntlet:      'interval',
  deadmans_climb: 'checkpoint',
  stillness:     'lives left',
}

const PHASE_STYLES: Record<GamePhase, string> = {
  idle:    'bg-slate-700 text-slate-400',
  armed:   'bg-amber-500/20 text-amber-300 border border-amber-500/30',
  active:  'bg-fuchsia-500/20 text-fuchsia-300 border border-fuchsia-500/30',
  recover: 'bg-cyan-500/20 text-cyan-300 border border-cyan-500/30',
  rest:    'bg-slate-600/40 text-slate-300 border border-slate-600/50',
  hold:    'bg-amber-500/20 text-amber-300 border border-amber-500/30',
  slip:    'bg-rose-500/20 text-rose-300 border border-rose-500/30',
  win:     'bg-emerald-500/20 text-emerald-300 border border-emerald-500/30',
}

const PHASE_LABELS: Record<GamePhase, string> = {
  idle:    'Idle',
  armed:   'Armed',
  active:  'Active',
  recover: 'Recover',
  rest:    'Rest',
  hold:    'Hold',
  slip:    'Slip',
  win:     'Win',
}

/** Format a duration in seconds as `Xs` (< 60 s) or `M:SS`. */
function formatDuration(sec: number): string {
  const total = Math.max(0, Math.round(sec))
  if (total < 60) return `${total}s`
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}

export function GamesControls() {
  const { mode, game } = useStatus()
  const connectionState = useDeviceStore((s) => s.connectionState)
  const send = useSendCommand()

  const isActive = mode === 'game'
  const isConnected = connectionState === 'connected'
  const isHoming = mode === 'homing'

  const [selected, setSelected] = useState<GameKind>('edge_recover')
  const [showManual, setShowManual] = useState(false)

  // Stable ref so the setInterval closure always calls the latest send().
  const sendRef = useRef(send)
  sendRef.current = send

  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const [buttonHeld, setButtonHeld] = useState(false)

  function startHold(e: React.PointerEvent) {
    e.currentTarget.setPointerCapture(e.pointerId)
    setButtonHeld(true)
    // Fire immediately so the first heartbeat isn't delayed by one interval.
    sendRef.current({ type: 'game_button', down: true })
    intervalRef.current = setInterval(() => {
      sendRef.current({ type: 'game_button', down: true })
    }, BUTTON_INTERVAL_MS)
  }

  function stopHold() {
    if (intervalRef.current !== null) {
      clearInterval(intervalRef.current)
      intervalRef.current = null
      // Only emit the release when we were actually holding.
      sendRef.current({ type: 'game_button', down: false })
    }
    setButtonHeld(false)
  }

  // Clean up the heartbeat on unmount or when the game ends.
  useEffect(() => {
    if (!isActive) stopHold()
    return () => { if (intervalRef.current) clearInterval(intervalRef.current) }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive])

  function toggleActive() {
    if (isActive) {
      stopHold()
      send({ type: 'game_stop' })
    } else {
      send({ type: 'game_start', kind: selected })
    }
  }

  const phase = game?.phase ?? 'idle'
  const isArmed = isActive && phase === 'armed'
  const intensity = Math.min(1, Math.max(0, game?.intensity ?? 0))
  // While armed, `level` is repurposed as the hardware ready-tap count.
  const level = Math.round(game?.level ?? 0)
  const readyTaps = isArmed ? Math.min(level, READY_TAPS) : 0
  const duration = game?.durationS ?? 0
  const holding = game?.holding ?? false
  const activeKind = game?.kind ?? selected
  const levelLabel = LEVEL_LABELS[activeKind]

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            holding  ? 'bg-fuchsia-400 animate-pulse' :
            isActive ? 'bg-amber-400' :
                       'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isArmed
            ? `Tap the device button ${READY_TAPS}× to start`
            : isActive
              ? holding ? `Playing ${GAME_NAMES[activeKind]}` : 'Game ready — hold to play'
              : 'No game running'}
        </span>
      </div>

      {/* Game picker — disabled while a game is active */}
      <div className={`flex flex-col gap-2 ${isActive ? 'opacity-40 pointer-events-none' : ''}`}>
        <span className="text-[10px] font-semibold uppercase tracking-widest text-slate-500">
          Choose a game
        </span>
        <div className="grid grid-cols-1 gap-2">
          {GAME_DOCS.map((doc) => {
            const sel = selected === doc.kind
            return (
              <button
                key={doc.kind}
                onClick={() => setSelected(doc.kind)}
                disabled={isActive}
                className={`flex flex-col items-start gap-0.5 px-3 py-2.5 rounded-xl text-left transition-colors
                  ${sel
                    ? 'bg-fuchsia-500/20 text-fuchsia-200 border border-fuchsia-500/40'
                    : 'bg-slate-800 text-slate-300 border border-transparent hover:bg-slate-700'
                  }`}
              >
                <span className="text-sm font-semibold">{doc.name}</span>
                <span className="text-[11px] text-slate-400">{doc.tagline}</span>
              </button>
            )
          })}
        </div>
      </div>

      {/* Recalibrate — quick access so users don't have to leave for Settings */}
      {!isActive && (
        <button
          onClick={() => send({ type: 'calibrate' })}
          disabled={!isConnected || isHoming}
          className="flex items-center justify-center gap-1.5 text-xs font-medium text-amber-400 hover:text-amber-300 disabled:text-slate-600 disabled:cursor-not-allowed transition-colors -mt-2"
        >
          {isHoming ? (
            <>
              <span className="w-3 h-3 border-2 border-amber-400/30 border-t-amber-400 rounded-full animate-spin" />
              Calibrating…
            </>
          ) : (
            <>
              <svg viewBox="0 0 24 24" className="w-3.5 h-3.5" fill="none" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182m0-4.991v4.99" />
              </svg>
              Recalibrate
            </>
          )}
        </button>
      )}

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-fuchsia-600 hover:bg-fuchsia-500 text-white shadow-lg shadow-fuchsia-500/20'
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
            Start {GAME_NAMES[selected]}
          </>
        )}
      </button>

      {/* Armed — waiting for the physical hand-switch ready gesture. The
          on-screen button can't fake this; it must happen on the device. */}
      {isArmed && (
        <div className="flex flex-col items-center gap-3 rounded-2xl bg-slate-800/60 border border-amber-500/30 p-5">
          <svg viewBox="0 0 24 24" className="w-7 h-7 text-amber-400 animate-pulse" fill="none" stroke="currentColor" strokeWidth={2}>
            <circle cx="12" cy="12" r="9" />
            <circle cx="12" cy="12" r="3" fill="currentColor" stroke="none" />
          </svg>
          <p className="text-sm text-amber-300 text-center font-semibold">
            Tap the button on the device {READY_TAPS} times
          </p>
          <p className="text-xs text-slate-500 text-center">
            This confirms you're at the actuator and ready — the app can't do it for you.
          </p>
          <div className="flex items-center gap-2">
            {Array.from({ length: READY_TAPS }, (_, i) => (
              <span
                key={i}
                className={`w-3 h-3 rounded-full transition-colors ${
                  i < readyTaps ? 'bg-amber-400' : 'bg-slate-700'
                }`}
              />
            ))}
          </div>
        </div>
      )}

      {/* Deadman hold button — only once the game is actually playing */}
      {isActive && !isArmed && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-slate-500 text-center">
            Hold to play · release to stop motion
          </p>

          <button
            onPointerDown={startHold}
            onPointerUp={stopHold}
            onPointerCancel={stopHold}
            onPointerLeave={stopHold}
            style={{ touchAction: 'none', userSelect: 'none' }}
            className={`relative flex items-center justify-center w-full py-8 rounded-2xl font-bold text-lg
              select-none transition-all duration-75
              ${buttonHeld
                ? 'bg-fuchsia-500 text-white scale-[0.97] shadow-inner shadow-fuchsia-700'
                : 'bg-slate-700 hover:bg-slate-600 text-slate-200 shadow-lg'
              }`}
          >
            {buttonHeld && (
              <span className="absolute inset-0 rounded-2xl border-2 border-fuchsia-400 animate-ping opacity-30" />
            )}

            <span className="flex flex-col items-center gap-1.5 relative">
              <svg viewBox="0 0 24 24" className="w-7 h-7" fill="none" stroke="currentColor" strokeWidth={2}>
                <circle cx="12" cy="12" r="9" />
                <circle cx="12" cy="12" r="3" fill="currentColor" stroke="none" />
              </svg>
              <span>{buttonHeld ? 'Holding…' : 'Hold'}</span>
            </span>
          </button>
        </div>
      )}

      {/* Live status panel — once actually playing */}
      {isActive && !isArmed && (
        <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          {/* Phase + holding */}
          <div className="flex items-center justify-between">
            <span className={`inline-flex items-center px-2.5 py-1 rounded-full text-xs font-semibold tracking-wider ${PHASE_STYLES[phase]}`}>
              {PHASE_LABELS[phase]}
            </span>
            <span className="flex items-center gap-1.5 text-xs text-slate-400">
              <span className={`inline-block w-1.5 h-1.5 rounded-full ${holding ? 'bg-fuchsia-400' : 'bg-slate-600'}`} />
              {holding ? 'Holding' : 'Released'}
            </span>
          </div>

          {/* Intensity meter */}
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs text-slate-400">Intensity</span>
              <span className="text-xs font-mono font-semibold text-fuchsia-300">
                {Math.round(intensity * 100)}%
              </span>
            </div>
            <div className="h-2 bg-slate-700 rounded-full overflow-hidden">
              <div
                className="h-full rounded-full bg-fuchsia-500 transition-[width] duration-100"
                style={{ width: `${intensity * 100}%` }}
              />
            </div>
          </div>

          {/* Duration + level */}
          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Duration</span>
              <span className="text-lg font-mono font-semibold text-slate-100">{formatDuration(duration)}</span>
            </div>
            <div className="flex flex-col">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">
                {levelLabel === '—' ? 'Level' : levelLabel}
              </span>
              <span className="text-lg font-mono font-semibold text-slate-100">
                {levelLabel === '—' ? '—' : level}
              </span>
            </div>
          </div>
        </div>
      )}

      {/* How to play */}
      <div className="flex flex-col gap-3 border-t border-slate-800 pt-4">
        <div className="flex flex-col gap-1">
          <span className="text-[10px] font-semibold uppercase tracking-widest text-slate-500">
            How to play · {GAME_NAMES[selected]}
          </span>
          <GamesManual only={selected} />
        </div>

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
          {showManual ? 'Hide full manual' : 'Manual — all games'}
        </button>

        {showManual && (
          <div className="rounded-2xl bg-slate-800/40 border border-slate-700/50 p-4">
            <GamesManual />
          </div>
        )}
      </div>

    </div>
  )
}
