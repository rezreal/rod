import { useDeviceStore } from '../../store/deviceStore'
import { useStatus } from '../../hooks/useDeviceState'

interface Indicator {
  label: string
  value: boolean
  good: boolean
  alwaysShow?: boolean
}

export function HealthRow() {
  const connected = useDeviceStore((s) => s.connectionState === 'connected' || s.connectionState === 'reconnecting')
  const s = useStatus()
  if (!connected) return null

  // Heart rate — show whenever a sensor is connected or a BPM is reporting.
  const hrConnected = s.heartRate?.connected ?? false
  const hrBpm = s.heartRate?.bpm
  const showHr = hrConnected || hrBpm !== undefined

  const indicators: Indicator[] = [
    { label: 'Servo',    value: s.servoOn,          good: true,  alwaysShow: true },
    { label: 'Homed',    value: s.homed,            good: true,  alwaysShow: true },
    { label: 'Ready',    value: s.controllerReady,  good: true,  alwaysShow: false },
    { label: 'E-Stop',   value: s.emergencyStop,    good: false, alwaysShow: true },
    { label: 'Volt Low', value: s.motorVoltageLow,  good: false, alwaysShow: false },
    { label: 'Alarm',    value: s.alarmCode !== 0,  good: false, alwaysShow: true },
    { label: 'Hand',     value: s.handSwitch,       good: true,  alwaysShow: true },
  ]

  return (
    <div className="flex flex-wrap gap-2">
      {indicators
        .filter((i) => i.alwaysShow || i.value)
        .map((ind) => {
          const active = ind.value
          let dotColor: string
          let textColor: string
          if (active && ind.good)   { dotColor = 'bg-emerald-400'; textColor = 'text-emerald-400' }
          else if (!active && ind.good) { dotColor = 'bg-slate-600'; textColor = 'text-slate-500' }
          else if (active && !ind.good) { dotColor = 'bg-red-400 animate-pulse'; textColor = 'text-red-400' }
          else                          { dotColor = 'bg-slate-600'; textColor = 'text-slate-500' }

          return (
            <div
              key={ind.label}
              className="flex items-center gap-1.5 text-xs"
              role="status"
              aria-label={`${ind.label}: ${active ? 'active' : 'inactive'}`}
            >
              <span className={`w-2 h-2 rounded-full ${dotColor}`} />
              <span className={textColor}>{ind.label}</span>
            </div>
          )
        })}

      {showHr && (
        <div
          className="flex items-center gap-1.5 text-xs"
          role="status"
          aria-label={`Heart rate: ${hrBpm !== undefined ? `${Math.round(hrBpm)} BPM` : 'no reading'}`}
        >
          <svg viewBox="0 0 24 24" className="w-3 h-3 text-red-500 animate-pulse" fill="currentColor" stroke="none">
            <path d="M12 21s-7.5-4.7-10-9.3C.6 8.9 2 5.5 5.2 5.5c1.9 0 3.2 1 3.8 2.1.6-1.1 1.9-2.1 3.8-2.1 3.2 0 4.6 3.4 3.2 6.2C19.5 16.3 12 21 12 21z" />
          </svg>
          <span className="text-red-400 font-mono">
            {hrBpm !== undefined ? `${Math.round(hrBpm)} BPM` : '—'}
          </span>
        </div>
      )}
    </div>
  )
}
