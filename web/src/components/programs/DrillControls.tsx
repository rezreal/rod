import { useEffect, useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

// Interval between deadman push pulses (ms). Must be ≤ half the bridge's
// deadman_timeout_ms (default 50 ms) so the servo never drops mid-push.
const PUSH_INTERVAL_MS = 15

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

export function DrillControls() {
  const { mode, drill } = useStatus()
  const send = useSendCommand()

  const isActive = mode === 'drill'
  const pushing   = drill?.pushing      ?? false
  const devRate   = drill?.feedRateMmS  ?? 5.0

  const [feedRate, setFeedRate] = useState(devRate)

  // Sync slider from device state (e.g. another client changed it)
  const prevRate = useRef(devRate)
  if (devRate !== prevRate.current) { prevRate.current = devRate; setFeedRate(devRate) }

  // Stable ref so the setInterval closure always calls the latest send()
  const sendRef = useRef(send)
  sendRef.current = send

  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const [buttonHeld, setButtonHeld] = useState(false)

  function startPush(e: React.PointerEvent) {
    e.currentTarget.setPointerCapture(e.pointerId)
    setButtonHeld(true)
    // Fire immediately so the first push isn't delayed by one interval.
    sendRef.current({ type: 'drill_push' })
    intervalRef.current = setInterval(() => {
      sendRef.current({ type: 'drill_push' })
    }, PUSH_INTERVAL_MS)
  }

  function stopPush() {
    setButtonHeld(false)
    if (intervalRef.current !== null) {
      clearInterval(intervalRef.current)
      intervalRef.current = null
    }
  }

  // Clean up interval on unmount or mode exit.
  useEffect(() => {
    if (!isActive) stopPush()
    return () => { if (intervalRef.current) clearInterval(intervalRef.current) }
  }, [isActive])

  function toggleActive() {
    if (isActive) {
      stopPush()
      send({ type: 'drill_stop' })
    } else {
      send({ type: 'drill_start', feedRateMmS: feedRate })
    }
  }

  function commitFeedRate(v: number) {
    setFeedRate(v)
    if (isActive) send({ type: 'drill_config', feedRateMmS: v })
  }

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            pushing   ? 'bg-cyan-400 animate-pulse' :
            isActive  ? 'bg-amber-400' :
                        'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {pushing ? 'Pushing outward' : isActive ? 'Servo off — rod free' : 'Inactive'}
        </span>
      </div>

      {/* Feed rate */}
      <RangeSlider
        label="Feed rate"
        value={feedRate}
        min={1}
        max={50}
        step={0.5}
        formatValue={(v) => `${v.toFixed(1)} mm/s`}
        onChange={setFeedRate}
        onCommit={commitFeedRate}
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
            Exit Drill Mode
          </>
        ) : (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
              <line x1="12" y1="2" x2="12" y2="16" strokeLinecap="round" />
              <polyline points="7 11 12 16 17 11" strokeLinecap="round" strokeLinejoin="round" />
              <line x1="7" y1="20" x2="17" y2="20" strokeLinecap="round" />
            </svg>
            Enter Drill Mode
          </>
        )}
      </button>

      {/* Deadman push button — only shown when active */}
      {isActive && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-slate-500 text-center">
            Hold to advance rod outward · release to stop
          </p>

          <button
            onPointerDown={startPush}
            onPointerUp={stopPush}
            onPointerCancel={stopPush}
            onPointerLeave={stopPush}
            style={{ touchAction: 'none', userSelect: 'none' }}
            className={`relative flex items-center justify-center w-full py-8 rounded-2xl font-bold text-lg
              select-none transition-all duration-75
              ${buttonHeld
                ? 'bg-cyan-500 text-white scale-[0.97] shadow-inner shadow-cyan-700'
                : 'bg-slate-700 hover:bg-slate-600 text-slate-200 shadow-lg'
              }`}
          >
            {/* Outer ring pulse when pushing */}
            {buttonHeld && (
              <span className="absolute inset-0 rounded-2xl border-2 border-cyan-400 animate-ping opacity-30" />
            )}

            <span className="flex flex-col items-center gap-1.5 relative">
              {/* Downward-arrow drill icon */}
              <svg viewBox="0 0 24 24" className="w-7 h-7" fill="none" stroke="currentColor" strokeWidth={2}>
                <line x1="12" y1="2" x2="12" y2="16" strokeLinecap="round" />
                <polyline points="7 11 12 16 17 11" strokeLinecap="round" strokeLinejoin="round" />
                <line x1="7" y1="20" x2="17" y2="20" strokeLinecap="round" />
              </svg>
              <span>{buttonHeld ? 'Pushing…' : 'Hold to Push'}</span>
            </span>
          </button>
        </div>
      )}

      {/* What this is */}
      <div className="border-t border-slate-800 pt-4">
        <p className="text-xs leading-relaxed text-slate-500">
          <span className="text-slate-400 font-medium">Drill</span> frees the rod —
          the servo is off so you can move it by hand. Hold the button to drive it
          outward at the feed rate; release and it stops and goes slack again. It's
          a deadman: motion only continues while you hold, so it stops the instant
          you let go.
        </p>
      </div>

    </div>
  )
}
