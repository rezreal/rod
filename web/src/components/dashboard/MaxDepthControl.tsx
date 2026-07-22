import { useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { useDeviceStore } from '../../store/deviceStore'

/**
 * Global max-depth. Programs address the full stroke; the bridge rescales every
 * program move into [entrance, max-depth], so each mode runs its full pattern
 * between the entrance and this depth (scaled, not hard-clamped). Persisted on
 * the device. A quick-set adopts the calibrated work-piece contact point.
 */
export function MaxDepthControl() {
  const connected = useDeviceStore(
    (s) => s.connectionState === 'connected' || s.connectionState === 'reconnecting',
  )
  const deviceInfo = useDeviceStore((s) => s.deviceInfo)
  const { maxDepthMm, workOriginMm } = useStatus()
  const send = useSendCommand()

  const stroke = deviceInfo?.strokeMm ?? 200
  const MIN = 10

  // Local slider value, synced from the device (another client may change it).
  const [value, setValue] = useState(maxDepthMm || stroke)
  const prev = useRef(maxDepthMm)
  if (maxDepthMm !== prev.current) {
    prev.current = maxDepthMm
    if (maxDepthMm > 0) setValue(maxDepthMm)
  }

  if (!connected) return null

  const pct = ((value - MIN) / (stroke - MIN)) * 100
  const commit = (mm: number) => {
    setValue(mm)
    send({ type: 'set_max_depth', mm })
  }

  return (
    <div className="flex flex-col gap-2 rounded-xl bg-slate-800/50 border border-slate-700 p-3">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-slate-400">Max depth</span>
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
          min={MIN}
          max={stroke}
          step={1}
          value={value}
          onChange={(e) => setValue(parseFloat(e.target.value))}
          onPointerUp={(e) => commit(parseFloat((e.target as HTMLInputElement).value))}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
          style={{ touchAction: 'none' }}
        />
        <div
          className="absolute w-5 h-5 rounded-full bg-amber-400 border-2 border-amber-300 shadow-lg pointer-events-none"
          style={{ left: `calc(${pct}% - 10px)` }}
        />
      </div>

      <div className="flex items-center justify-between">
        <span className="text-[10px] text-slate-600">
          Programs run between 0 and {Math.round(value)} mm of {stroke} mm
        </span>
        {workOriginMm !== undefined && (
          <button
            onClick={() => commit(Math.min(workOriginMm, stroke))}
            className="text-[10px] font-medium text-amber-400 hover:text-amber-300 transition-colors whitespace-nowrap"
          >
            Use contact ({Math.round(workOriginMm)} mm)
          </button>
        )}
      </div>
    </div>
  )
}
