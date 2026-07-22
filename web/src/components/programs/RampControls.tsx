import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

export function RampControls() {
  const { mode, ramp } = useStatus()
  const send = useSendCommand()

  const isActive = mode === 'ramp'

  function toggleActive() {
    if (isActive) {
      send({ type: 'ramp_stop' })
    } else {
      send({ type: 'ramp_start' })
    }
  }

  const intensity = ramp?.intensity ?? 0
  const velocity = ramp?.velocityMmS ?? 0
  const zoneMin = ramp?.zoneMin ?? 0
  const zoneMax = ramp?.zoneMax ?? 0

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive ? 'bg-rose-400 animate-pulse' : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive ? 'Ramping — building over time' : 'No ramp running'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-rose-700/40 hover:bg-rose-700/60 text-rose-300 border border-rose-700/50'
            : 'bg-rose-600 hover:bg-rose-500 text-white shadow-lg shadow-rose-500/20'
          }`}
      >
        {isActive ? (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth={2}>
              <rect x="6" y="6" width="12" height="12" rx="1" />
            </svg>
            Stop
          </>
        ) : (
          <>
            <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor" stroke="none">
              <polygon points="6 4 20 12 6 20 6 4" />
            </svg>
            Start Ramp
          </>
        )}
      </button>

      {/* Nudge buttons — only while active */}
      {isActive && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-slate-500 text-center">
            Nudge to steer the build
          </p>
          <div className="grid grid-cols-2 gap-3">
            <button
              onClick={() => send({ type: 'ramp_nudge', delta: -0.1 })}
              className="flex items-center justify-center gap-2 py-6 rounded-2xl font-bold text-base
                bg-slate-700 hover:bg-slate-600 active:scale-[0.97] text-slate-200 shadow-lg transition-all duration-75"
            >
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2.5}>
                <line x1="5" y1="12" x2="19" y2="12" strokeLinecap="round" />
              </svg>
              Ease off
            </button>
            <button
              onClick={() => send({ type: 'ramp_nudge', delta: 0.1 })}
              className="flex items-center justify-center gap-2 py-6 rounded-2xl font-bold text-base
                bg-rose-600 hover:bg-rose-500 active:scale-[0.97] text-white shadow-lg shadow-rose-500/20 transition-all duration-75"
            >
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2.5}>
                <line x1="12" y1="5" x2="12" y2="19" strokeLinecap="round" />
                <line x1="5" y1="12" x2="19" y2="12" strokeLinecap="round" />
              </svg>
              Build
            </button>
          </div>
        </div>
      )}

      {/* Live status panel */}
      {isActive && (
        <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
          {/* Intensity meter */}
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Intensity</span>
              <span className="text-sm font-mono font-semibold text-rose-400">
                {Math.round(intensity * 100)}%
              </span>
            </div>
            <div className="h-2 bg-slate-700 rounded-full overflow-hidden">
              <div
                className="h-full rounded-full bg-rose-500 transition-all duration-300"
                style={{ width: `${Math.round(intensity * 100)}%` }}
              />
            </div>
          </div>

          {/* Velocity + zone */}
          <div className="grid grid-cols-2 gap-4">
            <div className="flex flex-col">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Velocity</span>
              <span className="text-base font-semibold text-slate-100 font-mono">
                {Math.round(velocity)} mm/s
              </span>
            </div>
            <div className="flex flex-col">
              <span className="text-[10px] uppercase tracking-widest text-slate-500">Zone</span>
              <span className="text-base font-semibold text-slate-100 font-mono">
                {Math.round(zoneMin * 100)}–{Math.round(zoneMax * 100)}%
              </span>
            </div>
          </div>
        </div>
      )}

      {/* What this is */}
      <div className="border-t border-slate-800 pt-4">
        <p className="text-xs leading-relaxed text-slate-500">
          <span className="text-slate-400 font-medium">Ramp</span> builds speed and
          depth on its own over time, then holds. Nudge to ease off or build
          faster. It stops itself if left untouched for a while.
        </p>
      </div>

    </div>
  )
}
