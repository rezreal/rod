import { useEffect, useState } from 'react'
import { useDeviceStore } from '../../store/deviceStore'
import { useStatus } from '../../hooks/useDeviceState'
import { useTransport } from '../../transport/TransportProvider'
import { describeAlarm } from '../../lib/alarmCodes'
import { RemoteSharePanel } from '../remote/RemoteSharePanel'
import { CoyoteControls } from '../devices/CoyoteControls'

export function TopBar({ onSettings }: { onSettings: () => void }) {
  const connectionState = useDeviceStore((s) => s.connectionState)
  const deviceInfo      = useDeviceStore((s) => s.deviceInfo)
  const { alarmCode, coyote } = useStatus()
  const { send, disconnect, role, shareInterrupted, clearShareInterrupted } = useTransport()
  const [showShare, setShowShare] = useState(false)
  const [showCoyote, setShowCoyote] = useState(false)
  const [recovered, setRecovered] = useState(false)

  const coyoteConnected = coyote?.connected ?? false

  const hasAlarm  = alarmCode !== 0
  const isConnected = connectionState === 'connected'
  const canShare    = role === 'host' && isConnected

  // A share was live when the page reloaded: once the device is reconnected,
  // reopen the share panel with a fresh link and a recovery notice.
  useEffect(() => {
    if (shareInterrupted && canShare) {
      setRecovered(true)
      setShowShare(true)
      clearShareInterrupted()
    }
  }, [shareInterrupted, canShare, clearShareInterrupted])

  function handleStop() {
    send({ type: 'stop_all' })
  }

  const connDot = {
    connected:    'bg-emerald-400',
    connecting:   'bg-amber-400 animate-pulse',
    reconnecting: 'bg-amber-400 animate-pulse',
    disconnected: 'bg-slate-600',
    unsupported:  'bg-slate-600',
  }[connectionState]

  return (
    <>
    <header className="flex items-center gap-3 px-4 h-14 bg-slate-900 border-b border-slate-800 shrink-0">
      {/* Logo / name */}
      <div className="flex items-center gap-2 min-w-0">
        <svg viewBox="0 0 24 24" className="w-6 h-6 text-cyan-400 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z" />
        </svg>
        <span className="font-semibold text-slate-100 text-sm truncate">
          {deviceInfo?.deviceName ?? 'Rod'}
        </span>
      </div>

      {/* Connection badge */}
      <div className="flex items-center gap-1.5 ml-1">
        <span className={`w-2 h-2 rounded-full ${connDot}`} />
        <span className="text-xs text-slate-400 capitalize hidden sm:block">
          {connectionState === 'connected' ? 'BLE' : connectionState}
        </span>
      </div>

      {/* Alarm indicator */}
      {hasAlarm && (
        <div className="flex items-center gap-1.5 px-2 py-1 bg-red-500/20 border border-red-500/40 rounded text-xs text-red-400">
          <span>⚠</span>
          <span className="hidden sm:block">{describeAlarm(alarmCode)}</span>
        </div>
      )}

      <div className="flex-1" />

      {/* Stop button */}
      <button
        onClick={handleStop}
        disabled={!isConnected}
        className="flex items-center gap-1.5 px-3 py-1.5 bg-red-600 hover:bg-red-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-sm font-semibold rounded-lg transition-colors min-h-[36px]"
        aria-label="Emergency stop"
      >
        <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor">
          <rect x="6" y="6" width="12" height="12" rx="1" />
        </svg>
        <span className="hidden sm:block">STOP</span>
      </button>

      {/* Share remote control (host only, when connected) */}
      {canShare && (
        <button
          onClick={() => setShowShare(true)}
          className="p-2 text-slate-400 hover:text-cyan-400 transition-colors rounded-lg"
          aria-label="Share remote control"
        >
          <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M7.217 10.907a2.25 2.25 0 1 0 0 2.186m0-2.186c.18.324.283.696.283 1.093s-.103.77-.283 1.093m0-2.186 9.566-5.314m-9.566 7.5 9.566 5.314m0 0a2.25 2.25 0 1 0 3.935 2.186 2.25 2.25 0 0 0-3.935-2.186zm0-12.814a2.25 2.25 0 1 0 3.933-2.185 2.25 2.25 0 0 0-3.933 2.185z" />
          </svg>
        </button>
      )}

      {/* Coyote e-stim (only when a device is connected) */}
      {coyoteConnected && (
        <button
          onClick={() => setShowCoyote(true)}
          className="p-2 text-slate-400 hover:text-cyan-400 transition-colors rounded-lg"
          aria-label="Coyote e-stim controls"
        >
          <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z" />
          </svg>
        </button>
      )}

      {/* Settings */}
      <button
        onClick={onSettings}
        className="p-2 text-slate-400 hover:text-slate-200 transition-colors rounded-lg"
        aria-label="Settings"
      >
        <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      </button>

      {/* Disconnect */}
      {isConnected && (
        <button
          onClick={disconnect}
          className="p-2 text-slate-400 hover:text-slate-200 transition-colors rounded-lg"
          aria-label="Disconnect"
        >
          <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M18.364 18.364A9 9 0 0 0 5.636 5.636m12.728 12.728A9 9 0 0 1 5.636 5.636m12.728 12.728L5.636 5.636" />
          </svg>
        </button>
      )}
    </header>

    {showShare && (
      <RemoteSharePanel
        recovered={recovered}
        onClose={() => { setShowShare(false); setRecovered(false) }}
      />
    )}

    {showCoyote && (
      <CoyoteControls onClose={() => setShowCoyote(false)} />
    )}
    </>
  )
}
