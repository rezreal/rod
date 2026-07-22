import { useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { useDeviceStore } from '../../store/deviceStore'

const MIN = 10
/** Minimum enforced gap (mm) between comfortable and max depth; mirrors
 * MIN_DEPTH_GAP_MM in src/state.rs. */
const GAP = 5

/**
 * Two-tier global depth ceiling.
 *
 * "Comfortable depth" bounds every oscillating mode — HAMP, cycle, pulse,
 * plumb, surge, tide, trace, tempo, and the fixed-zone games (edge & recover,
 * gauntlet, deadman's climb, stillness). "Max depth" bounds the modes that
 * press toward or hold a single far point — ramp, HDSP, HSP, learn, drill,
 * impale, echo, and the Hold the Line game.
 *
 * Comfortable depth is always kept below max depth. Max depth can't be
 * changed while a program is running, so it can't shift a program's own
 * range out from under it mid-run; comfortable depth has no such lock.
 * Both are global (not per-mode) and persisted on the device.
 */
export function MaxDepthControl() {
  const connected = useDeviceStore(
    (s) => s.connectionState === 'connected' || s.connectionState === 'reconnecting',
  )
  const deviceInfo = useDeviceStore((s) => s.deviceInfo)
  const { mode, comfortableDepthMm, maxDepthMm, workOriginMm } = useStatus()
  const send = useSendCommand()

  const stroke = deviceInfo?.strokeMm ?? 200
  const programRunning = mode !== 'idle' && mode !== 'homing'

  // Local slider values, synced from the device (another client may change them).
  const [comfortable, setComfortable] = useState(comfortableDepthMm || Math.max(MIN, stroke - GAP))
  const prevComfortable = useRef(comfortableDepthMm)
  if (comfortableDepthMm !== prevComfortable.current) {
    prevComfortable.current = comfortableDepthMm
    if (comfortableDepthMm > 0) setComfortable(comfortableDepthMm)
  }

  const [max, setMax] = useState(maxDepthMm || stroke)
  const prevMax = useRef(maxDepthMm)
  if (maxDepthMm !== prevMax.current) {
    prevMax.current = maxDepthMm
    if (maxDepthMm > 0) setMax(maxDepthMm)
  }

  if (!connected) return null

  // Bounds derived from the device's authoritative (committed) values, not the
  // in-flight drag value, so they only move once the other slider is released.
  const comfortableCeiling = Math.max(MIN, Math.min(stroke - GAP, (maxDepthMm || stroke) - GAP))
  const maxFloor = Math.min(stroke, (comfortableDepthMm || MIN) + GAP)

  const commitComfortable = (mm: number) => {
    const clamped = Math.min(Math.max(mm, MIN), comfortableCeiling)
    setComfortable(clamped)
    send({ type: 'set_comfortable_depth', mm: clamped })
  }
  const commitMax = (mm: number) => {
    if (programRunning) return
    const clamped = Math.max(Math.min(mm, stroke), maxFloor)
    setMax(clamped)
    send({ type: 'set_max_depth', mm: clamped })
  }

  return (
    <div className="flex flex-col gap-4 rounded-xl bg-slate-800/50 border border-slate-700 p-3">
      <DepthSlider
        label="Comfortable depth"
        hint="oscillating modes"
        value={comfortable}
        min={MIN}
        max={comfortableCeiling}
        stroke={stroke}
        onChange={setComfortable}
        onCommit={commitComfortable}
        quickSet={
          workOriginMm !== undefined
            ? { label: `Use contact (${Math.round(workOriginMm)} mm)`, mm: workOriginMm }
            : undefined
        }
      />

      <div className="h-px bg-slate-700/60" />

      <DepthSlider
        label="Max depth"
        hint="ramp, HDSP/HSP, learn, drill, impale, echo, Hold the Line"
        value={max}
        min={maxFloor}
        max={stroke}
        stroke={stroke}
        onChange={setMax}
        onCommit={commitMax}
        disabled={programRunning}
        disabledNote="Stop the running program to change max depth"
      />
    </div>
  )
}

function DepthSlider({
  label,
  hint,
  value,
  min,
  max,
  stroke,
  onChange,
  onCommit,
  disabled,
  disabledNote,
  quickSet,
}: {
  label: string
  hint: string
  value: number
  min: number
  max: number
  stroke: number
  onChange: (v: number) => void
  onCommit: (v: number) => void
  disabled?: boolean
  disabledNote?: string
  quickSet?: { label: string; mm: number }
}) {
  const pct = ((value - min) / Math.max(1, max - min)) * 100

  return (
    <div className={`flex flex-col gap-2 ${disabled ? 'opacity-50' : ''}`}>
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <span className="text-xs font-medium text-slate-400">{label}</span>
          <span className="text-[10px] text-slate-600">{hint}</span>
        </div>
        <span className="text-xs font-mono font-semibold text-amber-400">
          {Math.round(value)} mm
        </span>
      </div>

      <div className="relative h-8 flex items-center">
        <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
          <div className="h-full rounded-full bg-amber-500" style={{ width: `${pct}%` }} />
        </div>
        <input
          type="range"
          min={min}
          max={max}
          step={1}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          onPointerUp={(e) => onCommit(parseFloat((e.target as HTMLInputElement).value))}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer disabled:cursor-not-allowed"
          style={{ touchAction: 'none' }}
        />
        <div
          className="absolute w-5 h-5 rounded-full bg-amber-400 border-2 border-amber-300 shadow-lg pointer-events-none"
          style={{ left: `calc(${pct}% - 10px)` }}
        />
      </div>

      <div className="flex items-center justify-between">
        <span className="text-[10px] text-slate-600">
          {disabled ? disabledNote : `0–${Math.round(value)} mm of ${stroke} mm`}
        </span>
        {quickSet && (
          <button
            onClick={() => onCommit(quickSet.mm)}
            className="text-[10px] font-medium text-amber-400 hover:text-amber-300 transition-colors whitespace-nowrap"
          >
            {quickSet.label}
          </button>
        )}
      </div>
    </div>
  )
}
