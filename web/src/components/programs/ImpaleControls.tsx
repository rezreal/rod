import { useEffect, useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

/** Format a countdown in seconds as `m:ss`. */
function fmtCountdown(s: number): string {
  const total = Math.max(0, Math.ceil(s))
  const m = Math.floor(total / 60)
  const sec = total % 60
  return `${m}:${sec.toString().padStart(2, '0')}`
}

/** Circular countdown: an emptying ring with the remaining time in the center. */
function CountdownRing({
  remainingS,
  totalS,
  size = 160,
}: {
  remainingS: number
  totalS: number
  size?: number
}) {
  const stroke = 10
  const r = (size - stroke) / 2
  const circumference = 2 * Math.PI * r
  const frac = totalS > 0 ? Math.min(1, Math.max(0, remainingS / totalS)) : 0
  const offset = circumference * (1 - frac)

  return (
    <div className="relative flex items-center justify-center" style={{ width: size, height: size }}>
      <svg width={size} height={size} className="-rotate-90">
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="currentColor"
          strokeWidth={stroke}
          className="text-slate-700"
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="currentColor"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          className="text-amber-400"
          style={{ transition: 'stroke-dashoffset 0.25s linear' }}
        />
      </svg>
      <div className="absolute flex flex-col items-center">
        <span className="text-2xl font-mono font-bold text-amber-300 tabular-nums">
          {fmtCountdown(remainingS)}
        </span>
        <span className="text-[10px] uppercase tracking-wide text-slate-500">until retract</span>
      </div>
    </div>
  )
}

// Interval between deadman heartbeats (ms) while the button is held. Must be
// ≤ half the bridge's impale deadman_timeout_ms (default 150 ms) so the rod
// never brakes mid-extension; a gap (release or connection loss) brakes it.
const HEARTBEAT_MS = 50

function RangeSlider({
  label,
  value,
  min = 0,
  max = 1,
  step = 0.01,
  onChange,
  onCommit,
  formatValue,
  disabled = false,
}: {
  label: string
  value: number
  min?: number
  max?: number
  step?: number
  onChange: (v: number) => void
  onCommit: (v: number) => void
  formatValue?: (v: number) => string
  disabled?: boolean
}) {
  const pct = ((value - min) / (max - min)) * 100

  return (
    <div className={`flex flex-col gap-2 ${disabled ? 'opacity-40 pointer-events-none' : ''}`}>
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-400">{label}</span>
        <span className="text-sm font-mono font-semibold text-cyan-400">
          {formatValue ? formatValue(value) : `${Math.round(value * 100)}%`}
        </span>
      </div>
      <div className="relative h-10 flex items-center">
        <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
          <div
            className="h-full rounded-full bg-cyan-500"
            style={{ width: `${pct}%` }}
          />
        </div>
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          onPointerUp={(e) => onCommit(parseFloat((e.target as HTMLInputElement).value))}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer disabled:cursor-default"
          style={{ touchAction: 'none' }}
        />
        <div
          className="absolute w-6 h-6 rounded-full bg-cyan-400 border-2 border-cyan-300 shadow-lg pointer-events-none"
          style={{ left: `calc(${pct}% - 12px)` }}
        />
      </div>
    </div>
  )
}

export function ImpaleControls() {
  const { mode, impale } = useStatus()
  const send = useSendCommand()

  const isActive   = mode === 'impale'
  const extending  = impale?.extending   ?? false
  const waiting    = impale?.waiting      ?? false
  const retracting = impale?.retracting   ?? false
  const devRate    = impale?.feedRateMmS  ?? 3.0
  const devHoldS   = impale?.retractAfterS ?? 600
  const remainingS = impale?.retractRemainingS ?? 0
  const won        = impale?.won ?? false

  // Show the win banner once per win (won stays true until the next extend
  // cycle clears it server-side, so guard against re-showing on every
  // telemetry tick). The chime itself is handled centrally by
  // <AudioFeedback> — it already watches this same transition.
  const [showWin, setShowWin] = useState(false)
  const wonFiredRef = useRef(false)
  useEffect(() => {
    if (won && !wonFiredRef.current) {
      wonFiredRef.current = true
      setShowWin(true)
      const t = setTimeout(() => setShowWin(false), 4000)
      return () => clearTimeout(t)
    }
    if (!won) wonFiredRef.current = false
  }, [won])

  const [feedRate, setFeedRate] = useState(devRate)
  // Hold duration is edited in minutes; sent to the device in seconds.
  const [holdMin, setHoldMin] = useState(devHoldS / 60)

  // Sync sliders from device state (e.g. another client changed them)
  const prevRate = useRef(devRate)
  if (devRate !== prevRate.current) { prevRate.current = devRate; setFeedRate(devRate) }
  const prevHoldS = useRef(devHoldS)
  if (devHoldS !== prevHoldS.current) { prevHoldS.current = devHoldS; setHoldMin(devHoldS / 60) }

  // Stable ref so the setInterval closure always calls the latest send()
  const sendRef = useRef(send)
  sendRef.current = send

  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const [buttonHeld, setButtonHeld] = useState(false)

  function startExtend(e: React.PointerEvent) {
    e.currentTarget.setPointerCapture(e.pointerId)
    setButtonHeld(true)
    // Fire immediately, then resend as a deadman heartbeat.
    sendRef.current({ type: 'impale_button', down: true })
    intervalRef.current = setInterval(() => {
      sendRef.current({ type: 'impale_button', down: true })
    }, HEARTBEAT_MS)
  }

  function stopExtend() {
    if (intervalRef.current !== null) {
      clearInterval(intervalRef.current)
      intervalRef.current = null
    }
    if (!buttonHeld) return
    setButtonHeld(false)
    sendRef.current({ type: 'impale_button', down: false })
  }

  // Clean up interval on unmount or mode exit.
  useEffect(() => {
    if (!isActive) stopExtend()
    return () => { if (intervalRef.current) clearInterval(intervalRef.current) }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive])

  function toggleActive() {
    if (isActive) {
      send({ type: 'impale_stop' })
    } else {
      send({ type: 'impale_start', feedRateMmS: feedRate, retractAfterS: holdMin * 60 })
    }
  }

  function commitHold(min: number) {
    setHoldMin(min)
    // Live-adjustable: while active (incl. already waiting) the bridge re-arms
    // the retract timer with the new duration.
    if (isActive) send({ type: 'impale_config', retractAfterS: min * 60 })
  }

  // A retract deadline is armed (and ticking) whenever remainingS is
  // positive — this stays true across repeated extend/release cycles, since
  // the backend only pauses the deadline's *effect* while extending, it
  // doesn't reset it.
  const timerArmed = remainingS > 0

  const statusLabel =
    extending  ? (timerArmed ? 'Extending — retract timer still running' : 'Extending') :
    retracting ? 'Retracting to home' :
    waiting    ? 'Holding — waiting to retract' :
    isActive   ? 'Servo off — rod free' :
                 'Inactive'

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            extending  ? 'bg-cyan-400 animate-pulse' :
            retracting ? 'bg-violet-400 animate-pulse' :
            waiting    ? 'bg-amber-400' :
            isActive   ? 'bg-amber-400' :
                         'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">{statusLabel}</span>
      </div>

      {/* Countdown ring — shown whenever the retract deadline is armed, so it
          stays visible (and keeps counting) through extend/release cycles
          instead of disappearing every time the button is held again. */}
      {timerArmed && (
        <div className="flex justify-center py-2">
          <CountdownRing remainingS={remainingS} totalS={devHoldS} />
        </div>
      )}

      {/* Win banner — shown briefly when the retract deadline is reached
          without an explicit stop. */}
      {showWin && (
        <div className="flex items-center justify-center gap-2 py-2 rounded-xl bg-emerald-500/15 border border-emerald-500/40 text-emerald-300 text-sm font-semibold animate-pulse">
          Held to the timer — you win!
        </div>
      )}

      {/* Feed rate */}
      <RangeSlider
        label="Feed rate"
        value={feedRate}
        min={1}
        max={50}
        step={0.5}
        formatValue={(v) => `${v.toFixed(1)} mm/s`}
        onChange={setFeedRate}
        onCommit={setFeedRate}
        disabled={isActive}
      />

      {/* Hold duration before auto-retract */}
      <RangeSlider
        label="Hold before retract"
        value={holdMin}
        min={0.5}
        max={30}
        step={0.5}
        formatValue={(v) => (v < 1 ? `${Math.round(v * 60)} s` : `${v.toFixed(1)} min`)}
        onChange={setHoldMin}
        onCommit={commitHold}
      />

      {/* Activate / deactivate */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-amber-700/40 hover:bg-amber-700/60 text-amber-300 border border-amber-700/50'
            : 'bg-cyan-600 hover:bg-cyan-500 text-white shadow-lg shadow-cyan-500/20'
          }`}
      >
        {isActive ? (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
            Exit Impale Mode
          </>
        ) : (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
              <line x1="12" y1="2" x2="12" y2="16" strokeLinecap="round" />
              <polyline points="7 11 12 16 17 11" strokeLinecap="round" strokeLinejoin="round" />
              <line x1="7" y1="20" x2="17" y2="20" strokeLinecap="round" />
            </svg>
            Enter Impale Mode
          </>
        )}
      </button>

      {/* Hold-to-extend button — only shown when active */}
      {isActive && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-slate-500 text-center">
            Hold to extend slowly · release to hold position and arm the retract timer
          </p>

          <button
            onPointerDown={startExtend}
            onPointerUp={stopExtend}
            onPointerCancel={stopExtend}
            onPointerLeave={stopExtend}
            style={{ touchAction: 'none', userSelect: 'none' }}
            className={`relative flex items-center justify-center w-full py-8 rounded-2xl font-bold text-lg
              select-none transition-all duration-75
              ${buttonHeld
                ? 'bg-cyan-500 text-white scale-[0.97] shadow-inner shadow-cyan-700'
                : 'bg-slate-700 hover:bg-slate-600 text-slate-200 shadow-lg'
              }`}
          >
            {/* Outer ring pulse while extending */}
            {buttonHeld && (
              <span className="absolute inset-0 rounded-2xl border-2 border-cyan-400 animate-ping opacity-30" />
            )}

            <span className="flex flex-col items-center gap-1.5 relative">
              <svg viewBox="0 0 24 24" className="w-7 h-7" fill="none" stroke="currentColor" strokeWidth={2}>
                <line x1="12" y1="2" x2="12" y2="16" strokeLinecap="round" />
                <polyline points="7 11 12 16 17 11" strokeLinecap="round" strokeLinejoin="round" />
                <line x1="7" y1="20" x2="17" y2="20" strokeLinecap="round" />
              </svg>
              <span>{buttonHeld ? 'Extending…' : 'Hold to Extend'}</span>
            </span>
          </button>
        </div>
      )}

      {/* What this is */}
      <div className="border-t border-slate-800 pt-4">
        <p className="text-xs leading-relaxed text-slate-500">
          <span className="text-slate-400 font-medium">Impale</span> extends the rod
          slowly while you hold the button. Release and the servo holds position in
          place, starting the retract timer (default 10 min). Reach it without
          pressing Exit and you win — the rod then retracts to home on its own.
          The timer keeps running even if you hold the button to extend further;
          only Exit Impale Mode resets it.
        </p>
      </div>

    </div>
  )
}
