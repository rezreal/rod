import { useState } from 'react'
import { useTransport } from '../../transport/TransportProvider'

export function ConnectScreen() {
  const { connectBle, isBleSupported, connectionState } = useTransport()
  const [error, setError] = useState<string | null>(null)
  const isConnecting = connectionState === 'connecting'

  async function handleBle() {
    setError(null)
    try {
      await connectBle()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      // Ignore user-cancelled picker
      if (!msg.toLowerCase().includes('cancel')) {
        setError(msg)
      }
    }
  }

  return (
    <div className="flex flex-col items-center justify-center flex-1 px-6 gap-8">
      {/* Logo */}
      <div className="flex flex-col items-center gap-4">
        <div className="relative">
          <div className="w-20 h-20 rounded-2xl bg-cyan-500/10 border border-cyan-500/30 flex items-center justify-center">
            <svg viewBox="0 0 24 24" className="w-10 h-10 text-cyan-400" fill="none" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
          </div>
          <div className="absolute -inset-2 rounded-3xl bg-cyan-500/5 -z-10" />
        </div>
        <div className="text-center">
          <h1 className="text-2xl font-bold text-slate-100">Rod</h1>
          <p className="text-sm text-slate-400 mt-1">Machine controller</p>
        </div>
      </div>

      {/* Connect / unsupported */}
      <div className="w-full max-w-sm flex flex-col gap-3">
        {isBleSupported ? (
          <button
            onClick={handleBle}
            disabled={isConnecting}
            className="flex items-center justify-center gap-3 w-full py-4 bg-cyan-600 hover:bg-cyan-500 disabled:bg-slate-700 disabled:text-slate-500 text-white font-semibold rounded-2xl transition-colors text-base min-h-[56px]"
          >
            {isConnecting ? (
              <>
                <span className="w-5 h-5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                Connecting…
              </>
            ) : (
              <>
                <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M8.111 16.404a5.5 5.5 0 0 1 7.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
                </svg>
                Connect via Bluetooth
              </>
            )}
          </button>
        ) : (
          <div className="flex flex-col gap-3 px-4 py-4 bg-slate-800/60 border border-slate-700 rounded-2xl">
            <div className="flex items-center gap-2 text-amber-400">
              <svg viewBox="0 0 24 24" className="w-5 h-5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
              </svg>
              <span className="text-sm font-medium">Bluetooth not available</span>
            </div>
            <p className="text-xs text-slate-400 leading-relaxed">
              Web Bluetooth requires <strong className="text-slate-300">Chrome or Edge</strong> on
              Android, Windows, macOS, or Linux.
              Safari on iOS is not supported.
            </p>
          </div>
        )}
      </div>

      {error && (
        <div className="w-full max-w-sm px-4 py-3 bg-red-500/10 border border-red-500/30 rounded-xl text-sm text-red-400 select-text cursor-text">
          {error}
        </div>
      )}

      <p className="text-xs text-slate-600 text-center">
        Connects directly over Bluetooth — no internet required.
      </p>
    </div>
  )
}
