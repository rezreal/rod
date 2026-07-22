export function RangeSlider({
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
