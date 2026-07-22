import { useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { useDeviceStore } from '../../store/deviceStore'

const MIN = 10

// Mirrors the backend's `HARD_STOP_MARGIN_MM` (src/modbus/driver.rs) — the
// gap kept off the physical hard stop. Used here only to advise the user at
// configuration time, not to enforce anything (the backend clamp is what
// actually protects the hardware).
const SAFETY_MARGIN_MM = 3

/**
 * Two-tier global depth ceiling.
 *
 * "Comfortable depth" bounds every oscillating mode — HAMP, cycle, pulse,
 * plumb, surge, tide, trace, tempo, and the fixed-zone games (edge & recover,
 * gauntlet, deadman's climb, stillness). "Max depth" bounds the modes that
 * press toward or hold a single far point — ramp, HDSP, HSP, learn, drill,
 * impale, echo.
 *
 * Comfortable depth never exceeds max depth (it may equal it — no minimum
 * gap is enforced). Max depth is authoritative: lowering it pulls comfortable
 * depth down to fit (the bridge does this and echoes the new comfortable
 * value back over telemetry — see `SetMaxDepth` in src/modbus/driver.rs).
 * Max depth can't be changed while a program is running; comfortable depth
 * has no such lock. Both are global (not per-mode) and persisted on the
 * device.
 *
 * Each slider's visual scale is fixed at [0, stroke] — it never rescales when
 * the *other* slider moves the allowed range, only the draggable portion
 * (and the dimmed-out region marking what's currently off-limits) does.
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

  // Local slider values, synced from the device (another client may change
  // them — including the bridge itself pulling comfortable down when max
  // depth shrinks below it).
  const [comfortable, setComfortable] = useState(comfortableDepthMm || stroke)
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

  // Allowed-value limits derived from the device's authoritative (committed)
  // values, not the in-flight drag value — but note these bound the *value*,
  // not the slider's visual scale (see DepthSlider).
  const comfortableLimitMax = Math.min(stroke, maxDepthMm || stroke)
  const comfortableLimitMin = Math.min(MIN, comfortableLimitMax)

  // Warn if the calibrated origin plus the configured depth would land past
  // the physical travel available (stroke minus the hard-stop margin) —
  // several modes (plumb, tide, surge, tempo, echo, trace) target
  // `origin + depth` directly, so a generous origin can eat most or all of
  // the room a depth setting expects to have.
  const origin = workOriginMm ?? 0
  const travelLimitMm = stroke - SAFETY_MARGIN_MM
  const overflowWarnings: string[] = []
  if (origin + comfortable > travelLimitMm) {
    overflowWarnings.push(
      `origin (${Math.round(origin)} mm) + comfortable depth (${Math.round(comfortable)} mm) = ${Math.round(origin + comfortable)} mm`,
    )
  }
  if (origin + max > travelLimitMm) {
    overflowWarnings.push(
      `origin (${Math.round(origin)} mm) + max depth (${Math.round(max)} mm) = ${Math.round(origin + max)} mm`,
    )
  }

  const commitComfortable = (mm: number) => {
    const clamped = Math.min(Math.max(mm, comfortableLimitMin), comfortableLimitMax)
    setComfortable(clamped)
    send({ type: 'set_comfortable_depth', mm: clamped })
  }
  const commitMax = (mm: number) => {
    if (programRunning) return
    const clamped = Math.min(Math.max(mm, MIN), stroke)
    setMax(clamped)
    send({ type: 'set_max_depth', mm: clamped })
    // If this drops below the current comfortable depth, the bridge pulls
    // comfortable down and echoes the new value back over telemetry — no
    // separate command needed here.
  }

  return (
    <div className="flex flex-col gap-4 rounded-xl bg-slate-800/50 border border-slate-700 p-3">
      <DepthSlider
        label="Comfortable depth"
        hint="oscillating modes"
        value={comfortable}
        limitMin={comfortableLimitMin}
        limitMax={comfortableLimitMax}
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
        hint="ramp, HDSP/HSP, learn, drill, impale, echo"
        value={max}
        limitMin={MIN}
        limitMax={stroke}
        stroke={stroke}
        onChange={setMax}
        onCommit={commitMax}
        disabled={programRunning}
        disabledNote="Stop the running program to change max depth"
      />

      {overflowWarnings.length > 0 && (
        <div className="rounded-lg bg-rose-950/40 border border-rose-700/50 px-3 py-2 text-[11px] text-rose-300 leading-snug">
          <span className="font-semibold">Exceeds available travel</span> — only{' '}
          {Math.round(travelLimitMm)} mm is reachable past the {SAFETY_MARGIN_MM} mm safety
          margin: {overflowWarnings.join('; ')}. Recalibrate closer to the surface or lower the
          depth.
        </div>
      )}
    </div>
  )
}

function DepthSlider({
  label,
  hint,
  value,
  limitMin,
  limitMax,
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
  /** Lowest/highest value the user may currently commit — may be narrower
   * than [0, stroke], but the slider's own visual scale always spans the
   * full [0, stroke] so it never rescales when this narrows or widens. */
  limitMin: number
  limitMax: number
  stroke: number
  onChange: (v: number) => void
  onCommit: (v: number) => void
  disabled?: boolean
  disabledNote?: string
  quickSet?: { label: string; mm: number }
}) {
  const clampToLimit = (v: number) => Math.min(Math.max(v, limitMin), limitMax)
  const toPct = (v: number) => (v / Math.max(1, stroke)) * 100

  const pct = toPct(value)
  const limitMinPct = toPct(limitMin)
  const limitMaxPct = toPct(limitMax)

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
        {/* Full fixed-scale track (0–stroke), dimmed outside what's currently allowed */}
        <div className="absolute inset-x-0 h-2 bg-slate-700/30 rounded-full" />
        <div
          className="absolute h-2 bg-slate-700 rounded-full"
          style={{ left: `${limitMinPct}%`, width: `${Math.max(0, limitMaxPct - limitMinPct)}%` }}
        />
        <div className="absolute h-2 rounded-full bg-amber-500" style={{ width: `${pct}%` }} />
        {/* Native input spans the full fixed scale so dragging always maps to
         * the same physical position; the committed/displayed value is
         * clamped to [limitMin, limitMax] so the custom thumb below soft-stops
         * at the edge of the allowed region instead of rescaling it. */}
        <input
          type="range"
          min={0}
          max={stroke}
          step={1}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(clampToLimit(parseFloat(e.target.value)))}
          onPointerUp={(e) => onCommit(clampToLimit(parseFloat((e.target as HTMLInputElement).value)))}
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
