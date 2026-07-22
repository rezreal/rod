import { useState, useCallback, useRef } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

function RangeSlider({
  label,
  value,
  min = 0,
  max = 1,
  step = 0.01,
  onChange,
  onCommit,
  formatValue,
  accent = false,
}: {
  label: string
  value: number
  min?: number
  max?: number
  step?: number
  onChange: (v: number) => void
  onCommit: (v: number) => void
  formatValue?: (v: number) => string
  accent?: boolean
}) {
  const pct = ((value - min) / (max - min)) * 100

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-400">{label}</span>
        <span className={`text-sm font-mono font-semibold ${accent ? 'text-cyan-400' : 'text-slate-200'}`}>
          {formatValue ? formatValue(value) : `${Math.round(value * 100)}%`}
        </span>
      </div>
      <div className="relative h-10 flex items-center">
        <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
          <div
            className={`h-full rounded-full ${accent ? 'bg-cyan-500' : 'bg-slate-500'}`}
            style={{ width: `${pct}%` }}
          />
        </div>
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          onPointerUp={(e) => onCommit(parseFloat((e.target as HTMLInputElement).value))}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
          style={{ touchAction: 'none' }}
        />
        {/* Custom thumb */}
        <div
          className={`absolute w-6 h-6 rounded-full border-2 shadow-lg pointer-events-none
            ${accent ? 'bg-cyan-400 border-cyan-300' : 'bg-slate-300 border-slate-200'}`}
          style={{ left: `calc(${pct}% - 12px)` }}
        />
      </div>
    </div>
  )
}

/**
 * DualRangeSlider — two independent thumb handles using pointer capture.
 * Each thumb div calls setPointerCapture so drag works even when the pointer
 * leaves the element. No overlapping invisible inputs — each thumb is
 * independently hittable.
 */
function DualRangeSlider({
  min: initMin,
  max: initMax,
  onCommit,
}: {
  min: number
  max: number
  onCommit: (min: number, max: number) => void
}) {
  const [localMin, setLocalMin] = useState(initMin)
  const [localMax, setLocalMax] = useState(initMax)
  const trackRef = useRef<HTMLDivElement>(null)

  // Sync from parent when device state arrives
  const prevMin = useRef(initMin)
  const prevMax = useRef(initMax)
  if (initMin !== prevMin.current) { prevMin.current = initMin; setLocalMin(initMin) }
  if (initMax !== prevMax.current) { prevMax.current = initMax; setLocalMax(initMax) }

  function pctFromPointer(e: React.PointerEvent): number {
    const rect = trackRef.current!.getBoundingClientRect()
    return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
  }

  function makeHandlers(thumb: 'min' | 'max') {
    return {
      onPointerDown(e: React.PointerEvent<HTMLDivElement>) {
        e.currentTarget.setPointerCapture(e.pointerId)
        e.stopPropagation()
      },
      onPointerMove(e: React.PointerEvent<HTMLDivElement>) {
        if (!e.buttons) return
        const p = pctFromPointer(e)
        if (thumb === 'min') setLocalMin(Math.min(p, localMax - 0.05))
        else                 setLocalMax(Math.max(p, localMin + 0.05))
      },
      onPointerUp(e: React.PointerEvent<HTMLDivElement>) {
        const p = pctFromPointer(e)
        if (thumb === 'min') {
          const v = Math.min(p, localMax - 0.05)
          setLocalMin(v)
          onCommit(v, localMax)
        } else {
          const v = Math.max(p, localMin + 0.05)
          setLocalMax(v)
          onCommit(localMin, v)
        }
      },
    }
  }

  const minPct = localMin * 100
  const maxPct = localMax * 100

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-400">Zone</span>
        <span className="text-sm font-mono font-semibold text-slate-200">
          {Math.round(localMin * 100)}% – {Math.round(localMax * 100)}%
        </span>
      </div>
      {/* Track — relative positioning anchor; use touch-none to prevent scroll */}
      <div ref={trackRef} className="relative h-10 flex items-center touch-none select-none">
        {/* Track bg */}
        <div className="absolute inset-x-3 h-2 bg-slate-700 rounded-full">
          <div
            className="absolute h-full bg-cyan-500/40 rounded-full"
            style={{ left: `${minPct}%`, right: `${100 - maxPct}%` }}
          />
        </div>
        {/* Min thumb */}
        <div
          {...makeHandlers('min')}
          className="absolute w-7 h-7 rounded-full bg-cyan-400 border-2 border-cyan-300 shadow-lg cursor-grab active:cursor-grabbing"
          style={{ left: `calc(${minPct}% - 14px + 12px)`, touchAction: 'none' }}
          aria-label="Zone minimum"
          role="slider"
          aria-valuenow={Math.round(localMin * 100)}
        />
        {/* Max thumb */}
        <div
          {...makeHandlers('max')}
          className="absolute w-7 h-7 rounded-full bg-cyan-400 border-2 border-cyan-300 shadow-lg cursor-grab active:cursor-grabbing"
          style={{ left: `calc(${maxPct}% - 14px + 12px)`, touchAction: 'none' }}
          aria-label="Zone maximum"
          role="slider"
          aria-valuenow={Math.round(localMax * 100)}
        />
      </div>
    </div>
  )
}

export function HampControls() {
  const { hamp } = useStatus()
  const send = useSendCommand()

  const running  = hamp?.running  ?? false
  const velocity = hamp?.velocity ?? 0.5
  const zoneMin  = hamp?.zoneMin  ?? 0.05
  const zoneMax  = hamp?.zoneMax  ?? 0.95
  // Handy-originated commands arrive without softness → bridge defaults to 0.5 (medium)
  const softness = hamp?.softness ?? 0.5

  // Initialise from live device state; follow subsequent device-side changes
  const [localVelocity, setLocalVelocity] = useState(velocity)
  const [localSoftness, setLocalSoftness] = useState(softness)
  const prevVelocity = useRef(velocity)
  const prevSoftness = useRef(softness)
  if (velocity !== prevVelocity.current) { prevVelocity.current = velocity; setLocalVelocity(velocity) }
  if (softness !== prevSoftness.current) { prevSoftness.current = softness; setLocalSoftness(softness) }

  const commitVelocity = useCallback((v: number) => {
    send({ type: 'hamp_config', velocity: v })
  }, [send])

  const commitZone = useCallback((min: number, max: number) => {
    send({ type: 'hamp_config', zoneMin: min, zoneMax: max })
  }, [send])

  const commitSoftness = useCallback((v: number) => {
    send({ type: 'hamp_config', softness: v })
  }, [send])

  function toggleRunning() {
    send(running ? { type: 'hamp_stop' } : { type: 'hamp_start' })
  }

  return (
    <div className="flex flex-col gap-6 p-4">
      <RangeSlider
        label="Speed"
        value={localVelocity}
        onChange={setLocalVelocity}
        onCommit={commitVelocity}
        accent
      />

      <DualRangeSlider
        min={zoneMin}
        max={zoneMax}
        onCommit={commitZone}
      />

      {/* Softness: 0 = hard snappy reversals, 1 = soft gentle reversals */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="text-sm text-slate-400">Softness</span>
            <span className="text-[10px] text-slate-600 bg-slate-800 px-1.5 py-0.5 rounded">
              {localSoftness < 0.25 ? 'Hard' : localSoftness < 0.6 ? 'Medium' : localSoftness < 0.85 ? 'Soft' : 'Very soft'}
            </span>
          </div>
          <span className="text-sm font-mono font-semibold text-violet-400">
            {Math.round(localSoftness * 100)}%
          </span>
        </div>
        {/* Gradient track: left = hard (slate), right = soft (violet) */}
        <div className="relative h-10 flex items-center">
          <div
            className="absolute inset-x-0 h-2 rounded-full"
            style={{ background: `linear-gradient(to right, #475569 0%, #8b5cf6 ${localSoftness * 100}%, #1e293b ${localSoftness * 100}%)` }}
          />
          <input
            type="range" min={0} max={1} step={0.01}
            value={localSoftness}
            onChange={(e) => setLocalSoftness(parseFloat(e.target.value))}
            onPointerUp={(e) => commitSoftness(parseFloat((e.target as HTMLInputElement).value))}
            className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
            style={{ touchAction: 'none' }}
          />
          <div
            className="absolute w-6 h-6 rounded-full bg-violet-400 border-2 border-violet-300 shadow-lg pointer-events-none"
            style={{ left: `calc(${localSoftness * 100}% - 12px)` }}
          />
        </div>
        <div className="flex justify-between text-[10px] text-slate-700 px-0.5">
          <span>Hard</span>
          <span>Very soft</span>
        </div>
      </div>

      <button
        onClick={toggleRunning}
        className={`flex items-center justify-center gap-3 w-full py-4 rounded-2xl font-semibold text-base transition-all min-h-[56px]
          ${running
            ? 'bg-slate-700 hover:bg-slate-600 text-slate-200'
            : 'bg-cyan-600 hover:bg-cyan-500 text-white shadow-lg shadow-cyan-500/20'
          }`}
      >
        {running ? (
          <>
            <svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor">
              <rect x="6" y="6" width="4" height="12" rx="1" />
              <rect x="14" y="6" width="4" height="12" rx="1" />
            </svg>
            Stop
          </>
        ) : (
          <>
            <svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
            Start
          </>
        )}
      </button>
    </div>
  )
}
