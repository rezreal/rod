import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { usePosition, useStatus } from '../../hooks/useDeviceState'

const PRESETS = [
  { label: 'Bottom', value: 0.0 },
  { label: 'Mid',    value: 0.5 },
  { label: 'Top',    value: 1.0 },
]

export function HdspControls() {
  const { positionPct: livePct } = usePosition()
  const { hdsp } = useStatus()
  const send = useSendCommand()

  // Pre-populate target position from current actuator position
  const [positionPct, setPositionPct] = useState(livePct)
  const [velocityPct, setVelocityPct] = useState(0.5)

  const moveState = hdsp?.state ?? 'idle'
  const isMoving = moveState === 'moving'

  function moveTo(pos: number) {
    // Use camelCase — Rust accepts both via serde alias
    send({ type: 'hdsp_move', positionPct: pos, velocityPct })
  }

  function stop() {
    send({ type: 'hdsp_stop' })
  }

  return (
    <div className="flex flex-col gap-6 p-4">
      {/* Position */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-sm text-slate-400">Target position</span>
          <span className="text-sm font-mono font-semibold text-violet-400">
            {Math.round(positionPct * 100)}%
          </span>
        </div>
        <div className="relative h-10 flex items-center">
          <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
            <div className="h-full bg-violet-500 rounded-full" style={{ width: `${positionPct * 100}%` }} />
          </div>
          <input
            type="range" min={0} max={1} step={0.01}
            value={positionPct}
            onChange={(e) => setPositionPct(parseFloat(e.target.value))}
            className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
            style={{ touchAction: 'none' }}
          />
          <div
            className="absolute w-6 h-6 rounded-full bg-violet-400 border-2 border-violet-300 shadow-lg pointer-events-none"
            style={{ left: `calc(${positionPct * 100}% - 12px)` }}
          />
        </div>
      </div>

      {/* Velocity */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-sm text-slate-400">Velocity</span>
          <span className="text-sm font-mono font-semibold text-slate-200">
            {Math.round(velocityPct * 100)}%
          </span>
        </div>
        <div className="relative h-10 flex items-center">
          <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
            <div className="h-full bg-slate-500 rounded-full" style={{ width: `${velocityPct * 100}%` }} />
          </div>
          <input
            type="range" min={0.05} max={1} step={0.01}
            value={velocityPct}
            onChange={(e) => setVelocityPct(parseFloat(e.target.value))}
            className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
            style={{ touchAction: 'none' }}
          />
          <div
            className="absolute w-6 h-6 rounded-full bg-slate-300 border-2 border-slate-200 shadow-lg pointer-events-none"
            style={{ left: `calc(${velocityPct * 100}% - 12px)` }}
          />
        </div>
      </div>

      {/* Move / Stop */}
      <div className="flex gap-3">
        <button
          onClick={() => moveTo(positionPct)}
          disabled={isMoving}
          className="flex-1 flex items-center justify-center gap-2 py-4 bg-violet-600 hover:bg-violet-500 disabled:bg-slate-700 disabled:text-slate-500 text-white font-semibold rounded-2xl transition-colors min-h-[56px]"
        >
          {isMoving ? (
            <>
              <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
              Moving…
            </>
          ) : 'Move'}
        </button>
        <button
          onClick={stop}
          className="px-5 py-4 bg-slate-700 hover:bg-slate-600 text-slate-300 font-semibold rounded-2xl transition-colors"
        >
          Stop
        </button>
      </div>

      {/* Presets */}
      <div className="flex gap-2">
        {PRESETS.map((p) => (
          <button
            key={p.label}
            onClick={() => {
              setPositionPct(p.value)
              moveTo(p.value)
            }}
            className="flex-1 py-3 bg-slate-800 hover:bg-slate-700 border border-slate-700 text-slate-400 hover:text-slate-200 text-sm rounded-xl transition-colors"
          >
            {p.label}
          </button>
        ))}
      </div>
    </div>
  )
}
