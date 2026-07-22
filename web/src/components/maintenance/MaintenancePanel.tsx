import { useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { useDeviceStore } from '../../store/deviceStore'
import { usePreferencesStore } from '../../store/preferencesStore'
import { describeAlarm } from '../../lib/alarmCodes'
import { MaxDepthControl } from '../dashboard/MaxDepthControl'
import { RawTelemetry } from './RawTelemetry'

interface Props {
  onClose: () => void
}

export function MaintenancePanel({ onClose }: Props) {
  const connectionState = useDeviceStore((s) => s.connectionState)
  const deviceInfo      = useDeviceStore((s) => s.deviceInfo)
  const { mode, alarmCode, coyoteAutoconnect, piupiuAutoconnect } = useStatus()
  const send = useSendCommand()
  const [showDiag, setShowDiag] = useState(false)
  const audioFeedbackEnabled    = usePreferencesStore((s) => s.audioFeedbackEnabled)
  const setAudioFeedbackEnabled = usePreferencesStore((s) => s.setAudioFeedbackEnabled)

  const isConnected = connectionState === 'connected'
  const isHoming    = mode === 'homing'
  const hasAlarm    = alarmCode !== 0

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="w-full sm:w-96 max-h-[90vh] bg-slate-900 border border-slate-800 rounded-t-3xl sm:rounded-2xl overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-slate-800">
          <h2 className="font-semibold text-slate-100">Settings & Maintenance</h2>
          <button
            onClick={onClose}
            className="p-2 text-slate-400 hover:text-slate-200 transition-colors rounded-lg"
          >
            <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-5 flex flex-col gap-5">
          {/* Device info */}
          {deviceInfo && (
            <div className="flex flex-col gap-2 p-4 bg-slate-800/50 rounded-xl border border-slate-700">
              <h3 className="text-xs font-semibold text-slate-400 uppercase tracking-wider">Device</h3>
              <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
                <span className="text-slate-500">Name</span>
                <span className="text-slate-200">{deviceInfo.deviceName}</span>
                <span className="text-slate-500">Stroke</span>
                <span className="text-slate-200">{deviceInfo.strokeMm} mm</span>
                <span className="text-slate-500">Firmware</span>
                <span className="text-slate-200">{deviceInfo.firmwareVersion}</span>
                <span className="text-slate-500">SSCP</span>
                <span className="text-slate-200">v{deviceInfo.sscpVersion}</span>
              </div>
            </div>
          )}

          {/* Alarm */}
          {hasAlarm && (
            <div className="flex flex-col gap-3 p-4 bg-red-900/30 border border-red-700/40 rounded-xl">
              <div className="flex items-start gap-2">
                <svg viewBox="0 0 24 24" className="w-5 h-5 text-red-400 shrink-0 mt-0.5" fill="none" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
                </svg>
                <div>
                  <p className="text-sm font-medium text-red-300">{describeAlarm(alarmCode)}</p>
                  <p className="text-xs text-red-400/70 mt-0.5">
                    Code 0x{alarmCode.toString(16).toUpperCase().padStart(2, '0')}
                  </p>
                </div>
              </div>
              <button
                onClick={() => send({ type: 'reset_alarm' })}
                disabled={!isConnected}
                className="flex items-center justify-center gap-2 py-2.5 bg-red-700 hover:bg-red-600 disabled:opacity-50 text-white text-sm font-semibold rounded-xl transition-colors"
              >
                Reset Alarm
              </button>
            </div>
          )}

          {/* Calibrate */}
          <div className="flex flex-col gap-3 p-4 bg-slate-800/50 rounded-xl border border-slate-700">
            <div>
              <h3 className="text-sm font-semibold text-slate-200">Calibration</h3>
              <p className="text-xs text-slate-500 mt-1">
                Homes, then gently steps inward — releasing the servo at each step
                to sense contact by spring-back (no sustained push) — to locate the
                work-piece origin. Takes a little while; controls disabled meanwhile.
              </p>
            </div>
            <button
              onClick={() => send({ type: 'calibrate' })}
              disabled={!isConnected || isHoming}
              className="flex items-center justify-center gap-2 py-3 bg-amber-600 hover:bg-amber-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-sm font-semibold rounded-xl transition-colors"
            >
              {isHoming ? (
                <>
                  <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  Homing…
                </>
              ) : 'Calibrate (find contact)'}
            </button>

            {/* Comfortable/max travel depth from the calibrated origin — global */}
            <MaxDepthControl />
          </div>

          {/* Autoconnect */}
          <div className="flex flex-col gap-3 p-4 bg-slate-800/50 rounded-xl border border-slate-700">
            <div>
              <h3 className="text-sm font-semibold text-slate-200">Autoconnect</h3>
              <p className="text-xs text-slate-500 mt-1">
                Automatically connect these BLE toys whenever they're in range.
              </p>
            </div>
            <ToggleSwitch
              label="Coyote (e-stim)"
              checked={coyoteAutoconnect}
              onChange={(enabled) => send({ type: 'set_coyote_autoconnect', enabled })}
            />
            <ToggleSwitch
              label="PiuPiu (lube launcher)"
              checked={piupiuAutoconnect}
              onChange={(enabled) => send({ type: 'set_piupiu_autoconnect', enabled })}
            />
          </div>

          {/* Handy compatibility */}
          <div className="flex items-center gap-3 p-4 bg-slate-800/30 rounded-xl border border-slate-700">
            <div className="w-8 h-8 rounded-lg bg-slate-700 flex items-center justify-center shrink-0">
              <svg viewBox="0 0 24 24" className="w-4 h-4 text-slate-400" fill="none" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M8.111 16.404a5.5 5.5 0 0 1 7.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
              </svg>
            </div>
            <div>
              <p className="text-xs font-medium text-slate-400">Handy compatibility</p>
              <p className="text-xs text-slate-600 mt-0.5">Available on separate BLE service</p>
            </div>
          </div>

          {/* Preferences */}
          <div className="flex items-center justify-between gap-3 p-4 bg-slate-800/50 rounded-xl border border-slate-700">
            <div>
              <h3 className="text-sm font-semibold text-slate-200">Audio feedback</h3>
              <p className="text-xs text-slate-500 mt-1">
                Plays a sound for input needed, success, hardware faults, and mistakes.
              </p>
            </div>
            <button
              onClick={() => setAudioFeedbackEnabled(!audioFeedbackEnabled)}
              aria-pressed={audioFeedbackEnabled}
              className={`relative shrink-0 w-11 h-6 rounded-full transition-colors ${
                audioFeedbackEnabled ? 'bg-cyan-600' : 'bg-slate-700'
              }`}
            >
              <span
                className={`absolute top-0.5 left-0.5 w-5 h-5 bg-white rounded-full transition-transform ${
                  audioFeedbackEnabled ? 'translate-x-5' : ''
                }`}
              />
            </button>
          </div>

          {/* Diagnostics toggle */}
          <button
            onClick={() => setShowDiag(!showDiag)}
            className="flex items-center justify-between w-full text-sm text-slate-400 hover:text-slate-200 transition-colors"
          >
            <span>Raw diagnostics</span>
            <svg
              viewBox="0 0 24 24"
              className={`w-4 h-4 transition-transform ${showDiag ? 'rotate-180' : ''}`}
              fill="none" stroke="currentColor" strokeWidth={2}
            >
              <path strokeLinecap="round" strokeLinejoin="round" d="m19 9-7 7-7-7" />
            </svg>
          </button>

          {showDiag && (
            <div className="bg-slate-950 rounded-xl border border-slate-800 overflow-hidden -mx-1">
              <RawTelemetry />
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

interface ToggleSwitchProps {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}

function ToggleSwitch({ label, checked, onChange }: ToggleSwitchProps) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className="flex items-center justify-between gap-3"
    >
      <span className="text-sm text-slate-300">{label}</span>
      <span
        className={`relative w-10 h-6 rounded-full transition-colors shrink-0
          ${checked ? 'bg-cyan-600' : 'bg-slate-700'}`}
      >
        <span
          className={`absolute top-0.5 left-0.5 w-5 h-5 bg-white rounded-full transition-transform
            ${checked ? 'translate-x-4' : 'translate-x-0'}`}
        />
      </span>
    </button>
  )
}
