import { useEffect, useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

interface Props {
  onClose: () => void
}

export function PiuPiuControls({ onClose }: Props) {
  const piupiu = useStatus().piupiu
  const send = useSendCommand()
  const [holding, setHolding] = useState(false)
  const holdingRef = useRef(holding)
  holdingRef.current = holding

  const connected = piupiu?.connected ?? false

  // Safety: if the modal closes (or unmounts) while still held, release the
  // trigger — otherwise the bridge keeps repeating the squirt command.
  useEffect(() => {
    return () => {
      if (holdingRef.current) send({ type: 'piupiu_squirt', active: false })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  function start(e: React.PointerEvent<HTMLButtonElement>) {
    if (!connected) return
    e.currentTarget.setPointerCapture(e.pointerId)
    setHolding(true)
    send({ type: 'piupiu_squirt', active: true })
  }

  function stop(e: React.PointerEvent<HTMLButtonElement>) {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId)
    }
    setHolding(false)
    send({ type: 'piupiu_squirt', active: false })
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="w-full sm:w-96 max-h-[90vh] bg-slate-900 border border-slate-800 rounded-t-3xl sm:rounded-2xl overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-slate-800">
          <h2 className="font-semibold text-slate-100">PiuPiu (lube launcher)</h2>
          <button
            onClick={onClose}
            className="p-2 text-slate-400 hover:text-slate-200 transition-colors rounded-lg"
            aria-label="Close"
          >
            <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-5 flex flex-col gap-5">
          {/* Connection */}
          <div className="flex items-center gap-2 text-sm">
            <span className={`w-2 h-2 rounded-full ${connected ? 'bg-emerald-400' : 'bg-slate-600'}`} />
            <span className={connected ? 'text-slate-300' : 'text-slate-500'}>
              {connected ? 'Connected' : 'Disconnected'}
            </span>
          </div>

          {/* Hold-to-squirt */}
          <button
            onPointerDown={start}
            onPointerUp={stop}
            onPointerCancel={stop}
            disabled={!connected}
            className={`flex items-center justify-center gap-2 py-8 text-lg font-bold rounded-2xl select-none transition-colors
              disabled:bg-slate-800 disabled:text-slate-600
              ${holding ? 'bg-cyan-500 text-white' : 'bg-slate-700 text-slate-100 hover:bg-slate-600'}`}
          >
            <svg viewBox="0 0 24 24" className="w-6 h-6" fill="currentColor">
              <path d="M12 21.75c-3.314 0-6-2.686-6-6 0-3.71 6-11.25 6-11.25s6 7.54 6 11.25c0 3.314-2.686 6-6 6z" />
            </svg>
            {holding ? 'Squirting…' : 'Hold to squirt'}
          </button>

          <p className="text-xs text-slate-500 leading-relaxed">
            Sends a squirt every 100 ms for as long as this is held — release
            (or let go outside the button) to stop.
          </p>
        </div>
      </div>
    </div>
  )
}
