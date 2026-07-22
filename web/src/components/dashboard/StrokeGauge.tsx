import { usePosition, useStatus } from '../../hooks/useDeviceState'

interface Props {
  className?: string
}

export function StrokeGauge({ className = '' }: Props) {
  const { positionPct, moving, direction } = usePosition()
  const { hamp } = useStatus()

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
    </div>
  )
}
