import { usePosition, useStatus } from '../../hooks/useDeviceState'
import { useDeviceStore } from '../../store/deviceStore'

interface Props {
  className?: string
}

export function StrokeGauge({ className = '' }: Props) {
  const { positionPct, moving, direction } = usePosition()
  const { hamp, comfortableDepthMm, maxDepthMm, workOriginMm } = useStatus()
  const stroke = useDeviceStore((s) => s.deviceInfo?.strokeMm ?? 200)

  const zoneMin = hamp?.zoneMin ?? 0.05
  const zoneMax = hamp?.zoneMax ?? 0.95
  const extending = direction === 'extending'

  // SVG coordinate system: 0 = top, 100 = bottom
  // We flip so 0% = bottom (retracted), 100% = top (extended)
  const trackH = 260
  const trackX = 28
  const trackW = 20
  const indicatorY = trackH * (1 - positionPct)
  const zoneMinY = trackH * (1 - zoneMin)
  const zoneMaxY = trackH * (1 - zoneMax)
  const zoneH = zoneMinY - zoneMaxY

  // Depth ceilings + calibrated origin — all live in the same absolute
  // [0, stroke] mm frame as positionMm (0 = retracted/home), so they convert
  // to the gauge's y-axis the same way positionPct does.
  const clampPct = (p: number) => Math.min(1, Math.max(0, p))
  const comfortablePct = comfortableDepthMm > 0 ? clampPct(comfortableDepthMm / stroke) : undefined
  const maxPct = maxDepthMm > 0 ? clampPct(maxDepthMm / stroke) : undefined
  const originPct = workOriginMm !== undefined ? clampPct(workOriginMm / stroke) : undefined

  const comfortableY = comfortablePct !== undefined ? trackH * (1 - comfortablePct) : undefined
  const maxY = maxPct !== undefined ? trackH * (1 - maxPct) : undefined
  const originY = originPct !== undefined ? trackH * (1 - originPct) : undefined

  return (
    <div className={`flex flex-col items-center gap-3 ${className}`}>
      <svg
        width={76}
        height={trackH + 24}
        viewBox={`0 0 76 ${trackH + 24}`}
        role="img"
        aria-label={`Actuator position ${Math.round(positionPct * 100)}%`}
      >
        {/* Track background */}
        <rect
          x={trackX}
          y={12}
          width={trackW}
          height={trackH}
          rx={trackW / 2}
          className="fill-slate-800"
        />

        {/* Depth range bands — comfortable depth, extended-but-allowed range
            up to max depth, and the off-limits range beyond max depth. */}
        {comfortableY !== undefined && (
          <rect
            x={trackX}
            y={12 + comfortableY}
            width={trackW}
            height={trackH - comfortableY}
            rx={trackW / 2}
            className="fill-emerald-500/20"
          >
            <title>{`Comfortable depth: 0–${Math.round(comfortableDepthMm)} mm`}</title>
          </rect>
        )}
        {comfortableY !== undefined && maxY !== undefined && maxY < comfortableY && (
          <rect
            x={trackX}
            y={12 + maxY}
            width={trackW}
            height={comfortableY - maxY}
            className="fill-amber-500/15"
          >
            <title>{`Extended range (beyond comfortable, within max): ${Math.round(comfortableDepthMm)}–${Math.round(maxDepthMm)} mm`}</title>
          </rect>
        )}
        {maxY !== undefined && maxY > 0 && (
          <rect
            x={trackX}
            y={12}
            width={trackW}
            height={maxY}
            rx={trackW / 2}
            className="fill-slate-950/60"
          >
            <title>{`Beyond max depth (not reachable): ${Math.round(maxDepthMm)}–${stroke} mm`}</title>
          </rect>
        )}

        {/* Zone highlight */}
        <rect
          x={trackX}
          y={12 + zoneMaxY}
          width={trackW}
          height={zoneH}
          rx={4}
          className="fill-cyan-500/20"
        />

        {/* Zone boundary ticks */}
        <line
          x1={trackX - 6}
          y1={12 + zoneMinY}
          x2={trackX + trackW + 6}
          y2={12 + zoneMinY}
          stroke="#22d3ee"
          strokeWidth={1.5}
          opacity={0.5}
        />
        <line
          x1={trackX - 6}
          y1={12 + zoneMaxY}
          x2={trackX + trackW + 6}
          y2={12 + zoneMaxY}
          stroke="#22d3ee"
          strokeWidth={1.5}
          opacity={0.5}
        />

        {/* Fill bar */}
        <rect
          x={trackX}
          y={12 + indicatorY}
          width={trackW}
          height={trackH - indicatorY}
          rx={trackW / 2}
          className="fill-cyan-500/40"
          style={{ transition: 'y 0.08s linear, height 0.08s linear' }}
        />

        {/* Indicator knob */}
        <circle
          cx={trackX + trackW / 2}
          cy={12 + indicatorY}
          r={trackW / 2 + 2}
          className={`fill-cyan-400 ${moving ? 'drop-shadow-[0_0_6px_rgba(34,211,238,0.8)]' : ''}`}
          style={{ transition: 'cy 0.08s linear' }}
        />

        {/* Direction arrow inside knob */}
        {moving && (
          <polygon
            points={
              extending
                ? `${trackX + trackW / 2},${12 + indicatorY - 7} ${trackX + trackW / 2 - 4},${12 + indicatorY - 1} ${trackX + trackW / 2 + 4},${12 + indicatorY - 1}`
                : `${trackX + trackW / 2},${12 + indicatorY + 7} ${trackX + trackW / 2 - 4},${12 + indicatorY + 1} ${trackX + trackW / 2 + 4},${12 + indicatorY + 1}`
            }
            className="fill-slate-900"
          />
        )}

        {/* Comfortable / max depth boundary ticks, drawn on top of the fill
            bar so they stay visible against the current position. */}
        {comfortableY !== undefined && (
          <line
            x1={trackX - 6}
            y1={12 + comfortableY}
            x2={trackX + trackW + 6}
            y2={12 + comfortableY}
            stroke="#34d399"
            strokeWidth={1.5}
            opacity={0.85}
          >
            <title>{`Comfortable depth limit: ${Math.round(comfortableDepthMm)} mm`}</title>
          </line>
        )}
        {maxY !== undefined && (
          <line
            x1={trackX - 6}
            y1={12 + maxY}
            x2={trackX + trackW + 6}
            y2={12 + maxY}
            stroke="#fb7185"
            strokeWidth={1.5}
            opacity={0.85}
          >
            <title>{`Max depth limit: ${Math.round(maxDepthMm)} mm`}</title>
          </line>
        )}

        {/* Calibrated origin marker — dashed to stay distinguishable from
            the solid depth-limit ticks even by shape, not just color. */}
        {originY !== undefined && (
          <line
            x1={trackX - 9}
            y1={12 + originY}
            x2={trackX + trackW + 9}
            y2={12 + originY}
            stroke="#e2e8f0"
            strokeWidth={1.5}
            strokeDasharray="2,2"
            opacity={0.9}
          >
            <title>{`Calibrated origin: ${Math.round(workOriginMm ?? 0)} mm`}</title>
          </line>
        )}

        {/* Percentage label */}
        <text
          x={trackX + trackW / 2}
          y={trackH + 22}
          textAnchor="middle"
          className="fill-slate-300 text-xs font-mono"
          fontSize={11}
        >
          {Math.round(positionPct * 100)}%
        </text>
      </svg>

      {(originY !== undefined || comfortableY !== undefined || maxY !== undefined) && (
        <div className="flex flex-col gap-1 text-[9px] leading-none font-mono text-slate-500 self-stretch">
          {originY !== undefined && (
            <div className="flex items-center gap-1.5">
              <span className="inline-block w-3 border-t border-dashed border-slate-300" />
              origin
            </div>
          )}
          {comfortableY !== undefined && (
            <div className="flex items-center gap-1.5">
              <span className="inline-block w-3 h-0.5 bg-emerald-400 rounded-full" />
              comfort
            </div>
          )}
          {maxY !== undefined && (
            <div className="flex items-center gap-1.5">
              <span className="inline-block w-3 h-0.5 bg-rose-400 rounded-full" />
              max
            </div>
          )}
        </div>
      )}
    </div>
  )
}
