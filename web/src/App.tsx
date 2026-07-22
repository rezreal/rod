import { useState } from 'react'
import { usePosition, useStatus } from './hooks/useDeviceState'
import { useDeviceStore } from './store/deviceStore'
import { useSendCommand } from './hooks/useSendCommand'
import { TopBar } from './components/layout/TopBar'
import { NavRail } from './components/layout/NavRail'
import { BottomNav } from './components/layout/BottomNav'
import { ConnectScreen } from './components/connect/ConnectScreen'
import { GuestConnectScreen } from './components/remote/GuestConnectScreen'
import { useTransport } from './transport/TransportProvider'
import { StrokeGauge } from './components/dashboard/StrokeGauge'
import { WaveformChart } from './components/dashboard/WaveformChart'
import { HealthRow } from './components/dashboard/HealthRow'
import { MaxDepthControl } from './components/dashboard/MaxDepthControl'
import { AlarmBanner } from './components/dashboard/AlarmBanner'
import { ProgramBadge } from './components/dashboard/ProgramBadge'
import { ProgramDrawer } from './components/programs/ProgramDrawer'
import { MaintenancePanel } from './components/maintenance/MaintenancePanel'
import { AudioFeedback } from './components/AudioFeedback'
import { fmtMm, fmtPct } from './lib/units'

function Dashboard() {
  const [showMaintenance, setShowMaintenance] = useState(false)
  // position slice — re-renders at telemetry rate for live readout
  const { positionPct, positionMm } = usePosition()
  // status slice — re-renders only on state change
  const { mode, emergencyStop, hamp, actuatorConnected } = useStatus()
  // device info is written once after connect
  const deviceInfo = useDeviceStore((s) => s.deviceInfo)
  // remote-control state (host side)
  const { role, guestConnected, stopShare } = useTransport()
  const send = useSendCommand()

  const isHoming = mode === 'homing'
  const programRunning = mode !== 'idle' && mode !== 'homing'

  return (
    <div className="flex flex-col h-full">
      <AudioFeedback />

      <TopBar onSettings={() => setShowMaintenance(true)} />

      {/* Guest-control safety banner (host side) — take-back always reachable */}
      {role === 'host' && guestConnected && (
        <div className="flex items-center gap-3 px-4 py-3 bg-red-900/60 border-b border-red-700 text-red-200 text-sm font-medium">
          <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0zM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632z" />
          </svg>
          <span className="flex-1">A guest is controlling this device</span>
          <button
            onClick={stopShare}
            className="px-3 py-1.5 bg-red-600 hover:bg-red-500 text-white text-sm font-semibold rounded-lg transition-colors shrink-0"
          >
            Take back control
          </button>
        </div>
      )}

      <AlarmBanner />

      {/* Actuator-disconnected banner */}
      {!actuatorConnected && (
        <div className="flex items-center gap-3 px-4 py-3 bg-orange-900/50 border-b border-orange-700 text-orange-300 text-sm">
          <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13.5 6H5.25A2.25 2.25 0 0 0 3 8.25v7.5A2.25 2.25 0 0 0 5.25 18h9a2.25 2.25 0 0 0 2.25-2.25V8.25A2.25 2.25 0 0 0 14.25 6H13.5m-3 0V4.5m0 0a1.5 1.5 0 0 1 3 0m-3 0a1.5 1.5 0 0 0 3 0M21 12l-3-3m0 6 3-3" />
          </svg>
          Actuator not connected — plug in the device; it will attach automatically. Motion controls won't work until then.
        </div>
      )}

      {/* Homing banner */}
      {mode === 'homing' && (
        <div className="flex items-center gap-3 px-4 py-3 bg-amber-900/50 border-b border-amber-700 text-amber-300 text-sm">
          <span className="w-4 h-4 border-2 border-amber-400/40 border-t-amber-400 rounded-full animate-spin shrink-0" />
          Homing in progress — controls disabled
        </div>
      )}

      {/* Emergency stop banner */}
      {emergencyStop && (
        <div className="flex items-center gap-3 px-4 py-3 bg-red-900/60 border-b border-red-700 text-red-300 text-sm font-medium">
          <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9.303 3.376c.866 1.5-.217 3.374-1.948 3.374H4.645c-1.73 0-2.813-1.874-1.948-3.374l7.3-12.748a2.25 2.25 0 0 1 3.898 0l7.3 12.748z" />
          </svg>
          Emergency stop active
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        <NavRail onSettings={() => setShowMaintenance(true)} />

        {/* Main content area */}
        <main className="flex flex-1 overflow-hidden">
          {/* Left: gauge + stats */}
          <div className="flex flex-col flex-1 overflow-y-auto p-4 gap-4">

            {/* Position card */}
            <div className="flex gap-4 bg-slate-900 border border-slate-800 rounded-2xl p-4">
              <StrokeGauge className="shrink-0" />

              <div className="flex flex-col justify-between flex-1 min-w-0 py-1">
                {/* Mode + position */}
                <div>
                  <ProgramBadge />
                  <div className="mt-3">
                    <div className="text-3xl font-bold font-mono text-slate-100 leading-none">
                      {fmtPct(positionPct)}
                      <span className="text-lg text-slate-500 ml-1">%</span>
                    </div>
                    <div className="text-sm text-slate-500 mt-1 font-mono">
                      {fmtMm(positionMm)}
                      {deviceInfo && (
                        <span className="text-slate-700"> / {deviceInfo.strokeMm} mm</span>
                      )}
                    </div>
                  </div>
                </div>

                {/* Health indicators */}
                <div className="mt-4">
                  <HealthRow />
                </div>

                {/* Calibration — locates the work-piece origin the "Use contact"
                    quick-set below relies on. Hidden while a program is running,
                    same as the recalibrate shortcut in GamesControls. */}
                {!programRunning && (
                  <div className="mt-4 flex items-center justify-between gap-3 rounded-xl bg-slate-800/50 border border-slate-700 p-3">
                    <div>
                      <p className="text-xs font-medium text-slate-400">Calibration</p>
                      <p className="text-[10px] text-slate-600 mt-0.5">Homes, then senses contact to locate depth zero</p>
                    </div>
                    <button
                      onClick={() => send({ type: 'calibrate' })}
                      disabled={!actuatorConnected || isHoming}
                      className="flex items-center justify-center gap-2 px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-sm font-semibold rounded-xl transition-colors shrink-0"
                    >
                      {isHoming ? (
                        <>
                          <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                          Homing…
                        </>
                      ) : 'Calibrate'}
                    </button>
                  </div>
                )}

                {/* Global comfortable/max depth ceilings */}
                <div className="mt-4">
                  <MaxDepthControl />
                </div>

                {/* HAMP stats */}
                {hamp?.running && (
                  <div className="mt-4 grid grid-cols-3 gap-2">
                    <div className="bg-slate-800/60 rounded-xl p-2.5">
                      <p className="text-[10px] text-slate-600 uppercase tracking-wider">Speed</p>
                      <p className="text-sm font-semibold text-cyan-400 mt-0.5 font-mono">
                        {Math.round((hamp.velocity) * 100)}%
                      </p>
                    </div>
                    <div className="bg-slate-800/60 rounded-xl p-2.5">
                      <p className="text-[10px] text-slate-600 uppercase tracking-wider">Zone</p>
                      <p className="text-sm font-semibold text-slate-300 mt-0.5 font-mono">
                        {Math.round(hamp.zoneMin * 100)}–{Math.round(hamp.zoneMax * 100)}%
                      </p>
                    </div>
                    <div className="bg-slate-800/60 rounded-xl p-2.5">
                      <p className="text-[10px] text-slate-600 uppercase tracking-wider">Soft</p>
                      <p className="text-sm font-semibold text-violet-400 mt-0.5 font-mono">
                        {Math.round(hamp.softness * 100)}%
                      </p>
                    </div>
                  </div>
                )}
              </div>
            </div>

            {/* Waveform card */}
            <div className="bg-slate-900 border border-slate-800 rounded-2xl p-4">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs text-slate-500">Position · last 15 s</span>
                <div className="flex gap-2 text-[10px] text-slate-600">
                  <span>0%</span>
                  <span className="text-slate-700">—</span>
                  <span>100%</span>
                </div>
              </div>
              <WaveformChart className="h-20" />
            </div>

          </div>

          {/* Right: program drawer (desktop) */}
          <div className="hidden md:flex">
            <ProgramDrawer />
          </div>
        </main>
      </div>

      {/* Mobile program controls */}
      <div className="md:hidden border-t border-slate-800">
        <ProgramDrawer />
      </div>

      <BottomNav onSettings={() => setShowMaintenance(true)} />

      {showMaintenance && (
        <MaintenancePanel onClose={() => setShowMaintenance(false)} />
      )}
    </div>
  )
}

export function App() {
  const connectionState = useDeviceStore((s) => s.connectionState)
  const { role } = useTransport()
  const isConnected = connectionState === 'connected' || connectionState === 'reconnecting'

  function renderDisconnected() {
    // Guests never see the Bluetooth connect screen — they finish a WebRTC
    // handshake instead.
    return role === 'guest' ? <GuestConnectScreen /> : <ConnectScreen />
  }

  return (
    <div className="h-dvh flex flex-col bg-[#0a0f1a] text-slate-100 overflow-hidden">
      {isConnected ? <Dashboard /> : renderDisconnected()}
    </div>
  )
}
