import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { describeAlarm, isAlarmCritical } from '../../lib/alarmCodes'

export function AlarmBanner() {
  const { alarmCode } = useStatus()
  const send = useSendCommand()

  if (alarmCode === 0) return null

  const critical = isAlarmCritical(alarmCode)

  return (
    <div
      role="alert"
      className={`flex items-center gap-3 px-4 py-3 text-sm
        ${critical
          ? 'bg-red-900/60 border-b border-red-700 text-red-300'
          : 'bg-amber-900/50 border-b border-amber-700 text-amber-300'
        }`}
    >
      <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
      </svg>
      <span className="flex-1 font-medium">
        {describeAlarm(alarmCode)}
        <span className="ml-2 font-normal opacity-70">
          (0x{alarmCode.toString(16).toUpperCase().padStart(2, '0')})
        </span>
      </span>
      {critical && (
        <span className="text-xs opacity-70">Recalibration required after reset</span>
      )}
      <button
        onClick={() => send({ type: 'reset_alarm' })}
        className="px-3 py-1.5 bg-white/10 hover:bg-white/20 rounded-lg text-xs font-medium transition-colors"
      >
        Reset
      </button>
    </div>
  )
}
