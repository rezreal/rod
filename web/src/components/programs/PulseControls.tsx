import { useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

const FACTOR_MIN = 0.5
const FACTOR_MAX = 4.0
const FACTOR_STEP = 0.1
const DEFAULT_FACTOR = 1.0

function FactorSlider({
  label,
  value,
  onChange,
  onCommit,
}: {
  label: string
  value: number
  onChange: (v: number) => void
  onCommit: (v: number) => void
}) {
  const pct = ((value - FACTOR_MIN) / (FACTOR_MAX - FACTOR_MIN)) * 100

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-400">{label}</span>
        <span className="text-sm font-mono font-semibold text-red-400">
          {value.toFixed(1)}
        </span>
      </div>
      <div className="relative h-10 flex items-center">
        <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
          <div
            className="h-full rounded-full bg-red-500"
            style={{ width: `${pct}%` }}
          />
        </div>
        <input
          type="range"
          min={FACTOR_MIN}
          max={FACTOR_MAX}
          step={FACTOR_STEP}
          value={value}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          onPointerUp={(e) => onCommit(parseFloat((e.target as HTMLInputElement).value))}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
          style={{ touchAction: 'none' }}
        />
        <div
          className="absolute w-6 h-6 rounded-full bg-red-400 border-2 border-red-300 shadow-lg pointer-events-none"
          style={{ left: `calc(${pct}% - 12px)` }}
        />
      </div>
    </div>
  )
}

export function PulseControls() {
  const { mode, heartRate, pulse } = useStatus()
  const send = useSendCommand()

  const isActive = mode === 'pulse'

  const [showHow, setShowHow] = useState(false)

  // Factor slider — initialise from device pulse state when available.
  const devFactor = pulse?.factor ?? DEFAULT_FACTOR
  const [factor, setFactor] = useState(devFactor)

  // Sync slider from device state (e.g. another client changed it).
  const prevFactor = useRef(devFactor)
  if (pulse?.factor !== undefined && pulse.factor !== prevFactor.current) {
    prevFactor.current = pulse.factor
    setFactor(pulse.factor)
  }

  function toggleActive() {
    if (isActive) {
      send({ type: 'pulse_stop' })
    } else {
      send({ type: 'pulse_start', factor })
    }
  }

  function commitFactor(v: number) {
    setFactor(v)
    if (isActive) send({ type: 'pulse_set_factor', factor: v })
  }

  const sensorConnected = heartRate?.connected ?? false
  const sensorScanning = heartRate?.scanning ?? false
  const bpm = heartRate?.bpm ?? pulse?.bpm
  const velocity = pulse?.velocityMmS

  return (
    <div className="flex flex-col gap-6 p-4">

      {/* Status pill */}
      <div className="flex items-center gap-2">
        <span
          className={`inline-block w-2 h-2 rounded-full ${
            isActive ? 'bg-red-400 animate-pulse' : 'bg-slate-600'
          }`}
        />
        <span className="text-xs text-slate-400">
          {isActive ? 'Pulse running — following heart rate' : 'No pulse running'}
        </span>
      </div>

      {/* Start / Stop */}
      <button
        onClick={toggleActive}
        className={`flex items-center justify-center gap-2 w-full py-3 rounded-2xl font-semibold text-sm transition-all
          ${isActive
            ? 'bg-red-700/40 hover:bg-red-700/60 text-red-300 border border-red-700/50'
            : 'bg-red-600 hover:bg-red-500 text-white shadow-lg shadow-red-500/20'
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
            Start Pulse
          </>
        )}
      </button>

      {/* Factor slider */}
      <FactorSlider
        label="Speed factor — mm/s per BPM"
        value={factor}
        onChange={setFactor}
        onCommit={commitFactor}
      />

      {/* Live readout */}
      <div className="flex flex-col gap-4 rounded-2xl bg-slate-800/60 border border-slate-700/60 p-4">
        {/* Heart rate */}
        <div className="flex flex-col gap-1">
          <span className="text-[10px] uppercase tracking-widest text-slate-500">Heart rate</span>
          <div className="flex items-baseline gap-2">
            <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0 text-red-500 animate-pulse" fill="currentColor" stroke="none">
              <path d="M12 21s-7.5-4.7-10-9.3C.6 8.9 2 5.5 5.2 5.5c1.9 0 3.2 1 3.8 2.1.6-1.1 1.9-2.1 3.8-2.1 3.2 0 4.6 3.4 3.2 6.2C19.5 16.3 12 21 12 21z" />
            </svg>
            <span className="text-4xl font-bold font-mono text-slate-100 leading-none">
              {bpm !== undefined ? Math.round(bpm) : '—'}
            </span>
            <span className="text-sm text-slate-500">BPM</span>
          </div>
          {!sensorConnected && !sensorScanning && bpm === undefined && (
            <span className="text-xs text-slate-500">No sensor — using base rate</span>
          )}

          {/* Pair / unpair the heart-rate sensor (BLE central on the Pi). */}
          <button
            onClick={() =>
              send({ type: sensorConnected || sensorScanning ? 'hr_disconnect' : 'hr_connect' })
            }
            className={`mt-1 flex items-center justify-center gap-2 w-full py-2 rounded-xl text-xs font-semibold transition-colors
              ${sensorConnected
                ? 'bg-red-700/30 hover:bg-red-700/50 text-red-300 border border-red-700/40'
                : sensorScanning
                  ? 'bg-slate-700 text-slate-300 border border-slate-600'
                  : 'bg-red-600 hover:bg-red-500 text-white'
              }`}
          >
            {sensorScanning && !sensorConnected && (
              <span className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
            )}
            {sensorConnected
              ? 'Disconnect sensor'
              : sensorScanning
                ? 'Scanning… tap to cancel'
                : 'Connect heart-rate sensor'}
          </button>
        </div>

        {/* Current speed + formula hint */}
        <div className="flex items-end justify-between">
          <div className="flex flex-col">
            <span className="text-[10px] uppercase tracking-widest text-slate-500">Current speed</span>
            <span className="text-base font-semibold text-slate-100 font-mono">
              {velocity !== undefined ? Math.round(velocity) : '—'} mm/s
            </span>
          </div>
          <span className="text-[10px] font-mono text-slate-600">speed = BPM × factor</span>
        </div>
      </div>

      {/* How it works */}
      <div className="flex flex-col gap-3 border-t border-slate-800 pt-4">
        <button
          onClick={() => setShowHow((v) => !v)}
          className="flex items-center justify-center gap-1.5 text-xs font-medium text-slate-400 hover:text-slate-200 transition-colors"
        >
          <svg
            viewBox="0 0 24 24"
            className={`w-3.5 h-3.5 transition-transform ${showHow ? 'rotate-180' : ''}`}
            fill="none" stroke="currentColor" strokeWidth={2}
          >
            <polyline points="6 9 12 15 18 9" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {showHow ? 'Hide how it works' : 'How it works'}
        </button>

        {showHow && (
          <div className="rounded-2xl bg-slate-800/40 border border-slate-700/50 p-4">
            <p className="text-xs leading-relaxed text-slate-400">
              Pulse oscillates at a speed that follows your heart rate — faster pulse,
              faster strokes. Adjust the factor to scale how strongly it reacts. Tap
              <span className="text-slate-300"> Connect heart-rate sensor</span> to pair a
              Bluetooth strap/watch (the device scans for it) and your BPM drives the
              motion; without one it uses a base rate.
            </p>
          </div>
        )}
      </div>

    </div>
  )
}
