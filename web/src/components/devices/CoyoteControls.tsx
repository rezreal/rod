import { useEffect, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'

interface Props {
  onClose: () => void
}

export function CoyoteControls({ onClose }: Props) {
  const coyote = useStatus().coyote
  const send = useSendCommand()

  const connected = coyote?.connected ?? false
  const maxStrength = coyote?.maxStrength ?? 0
  const following = coyote?.following ?? false
  const [scale, setScale] = useState(1)

  // Local slider state, initialised from device-reported strength.
  const [a, setA] = useState(coyote?.strengthA ?? 0)
  const [b, setB] = useState(coyote?.strengthB ?? 0)

  // Re-sync local sliders to the device when it (re)connects or the cap changes.
  useEffect(() => {
    if (coyote?.connected) {
      setA((prev) => Math.min(prev, coyote.maxStrength))
      setB((prev) => Math.min(prev, coyote.maxStrength))
    }
  }, [coyote?.connected, coyote?.maxStrength])

  function clamp(v: number) {
    return Math.max(0, Math.min(v, maxStrength))
  }

  function setStrength(next: { a?: number; b?: number }) {
    const na = clamp(next.a ?? a)
    const nb = clamp(next.b ?? b)
    setA(na)
    setB(nb)
    send({ type: 'coyote_set_strength', a: na, b: nb })
  }

  function handleStop() {
    setA(0)
    setB(0)
    send({ type: 'coyote_stop' })
  }

  function toggleFollow() {
    send({ type: 'coyote_follow', enable: !following, scale })
  }

  function changeScale(v: number) {
    setScale(v)
    if (following) send({ type: 'coyote_follow', enable: true, scale: v })
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="w-full sm:w-96 max-h-[90vh] bg-slate-900 border border-slate-800 rounded-t-3xl sm:rounded-2xl overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-slate-800">
          <h2 className="font-semibold text-slate-100">Coyote (e-stim)</h2>
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
          {/* Connection / battery */}
          <div className="flex items-center gap-2 text-sm">
            <span className={`w-2 h-2 rounded-full ${connected ? 'bg-emerald-400' : 'bg-slate-600'}`} />
            {connected ? (
              <span className="text-slate-300">
                Connected
                {coyote?.battery != null && (
                  <span className="text-slate-500"> · {Math.round(coyote.battery)}% battery</span>
                )}
              </span>
            ) : (
              <span className="text-slate-500">Disconnected</span>
            )}
          </div>

          {/* Follow program */}
          <div className="flex flex-col gap-2 p-4 bg-slate-800/50 rounded-xl border border-slate-700">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-semibold text-slate-200">Follow program</h3>
                <p className="text-xs text-slate-500">e-stim tracks how hard the rod moves</p>
              </div>
              <button
                onClick={toggleFollow}
                disabled={!connected}
                className={`px-3 py-1.5 rounded-lg text-sm font-semibold transition-colors disabled:opacity-40
                  ${following ? 'bg-cyan-600 text-white' : 'bg-slate-700 text-slate-200 hover:bg-slate-600'}`}
              >
                {following ? 'On' : 'Off'}
              </button>
            </div>
            <div className="flex items-center gap-3">
              <span className="text-xs text-slate-500 w-10">scale</span>
              <input
                type="range" min={0} max={1} step={0.05} value={scale}
                disabled={!connected}
                onChange={(e) => changeScale(Number(e.target.value))}
                className="flex-1 accent-cyan-400 disabled:opacity-40"
              />
              <span className="w-10 text-right text-sm text-slate-300 tabular-nums">
                {Math.round(scale * 100)}%
              </span>
            </div>
          </div>

          {/* Channel A */}
          <ChannelSlider
            label="Channel A"
            value={following ? Math.round(coyote?.strengthA ?? 0) : a}
            reported={Math.round(coyote?.strengthA ?? 0)}
            max={maxStrength}
            disabled={!connected || following}
            onChange={(v) => setStrength({ a: v })}
          />

          {/* Channel B */}
          <ChannelSlider
            label="Channel B"
            value={following ? Math.round(coyote?.strengthB ?? 0) : b}
            reported={Math.round(coyote?.strengthB ?? 0)}
            max={maxStrength}
            disabled={!connected || following}
            onChange={(v) => setStrength({ b: v })}
          />

          {following && (
            <p className="text-xs text-cyan-400/80 -mt-2">
              Following the program — channels are driven automatically.
            </p>
          )}

          {/* STOP */}
          <button
            onClick={handleStop}
            disabled={!connected}
            className="flex items-center justify-center gap-2 py-3.5 bg-red-600 hover:bg-red-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-base font-bold rounded-xl transition-colors"
          >
            STOP (zero both)
          </button>

          <p className="text-xs text-slate-500 leading-relaxed">
            Strength is capped at {Math.round(maxStrength)} and ramps up gradually. Start low.
          </p>
        </div>
      </div>
    </div>
  )
}

interface ChannelSliderProps {
  label: string
  value: number
  reported: number
  max: number
  disabled: boolean
  onChange: (v: number) => void
}

function ChannelSlider({ label, value, reported, max, disabled, onChange }: ChannelSliderProps) {
  return (
    <div className="flex flex-col gap-2 p-4 bg-slate-800/50 rounded-xl border border-slate-700">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-semibold text-slate-200">{label}</h3>
        <span className="text-xs text-slate-500">e-stim strength</span>
      </div>
      <div className="flex items-center gap-3">
        <input
          type="range"
          min={0}
          max={max}
          step={1}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(Number(e.target.value))}
          className="flex-1 accent-cyan-400 disabled:opacity-40"
        />
        <span className="w-10 text-right text-lg font-semibold text-slate-100 tabular-nums">
          {Math.round(value)}
        </span>
      </div>
      <div className="text-xs text-slate-500">
        Device: <span className="text-slate-300 tabular-nums">{reported}</span>
      </div>
    </div>
  )
}
