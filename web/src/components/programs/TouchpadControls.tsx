import { useEffect, useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { usePosition, useStatus } from '../../hooks/useDeviceState'

// How often a new target is pushed to the actuator while dragging (ms).
const SEND_INTERVAL_MS = 40

// Responsiveness at dead center of the pad ("snappy": the actuator chases
// the finger almost instantly) vs. at the left/right edges ("soft": it eases
// toward the finger with a slow, gentle catch-up). xOffset (0 = center,
// 1 = edge) interpolates between the two pairs below.
const SNAPPY_VELOCITY = 0.95
const SOFT_VELOCITY = 0.15
const SNAPPY_CHASE = 1 // fraction of remaining distance closed per animation frame; 1 = instant
const SOFT_CHASE = 0.08

const PRESETS = [
  { label: 'Bottom', value: 0.0 },
  { label: 'Mid', value: 0.5 },
  { label: 'Top', value: 1.0 },
]

function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t
}

export function TouchpadControls() {
  const { positionPct: livePct } = usePosition()
  const { hdsp } = useStatus()
  const send = useSendCommand()

  const padRef = useRef<HTMLDivElement>(null)
  const [dragging, setDragging] = useState(false)
  const [displayPos, setDisplayPos] = useState(livePct)

  // Raw touch reading (updated synchronously on pointer events) and the
  // smoothed position the animation loop chases it with.
  const targetYRef = useRef(livePct)
  const xOffsetRef = useRef(0)
  const rawXRef = useRef(0.5)
  const smoothedYRef = useRef(livePct)

  const sendRef = useRef(send)
  sendRef.current = send

  const rafRef = useRef<number | null>(null)
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Track the live actuator position while idle, so a fresh touch starts
  // from wherever the device actually is.
  useEffect(() => {
    if (!dragging) {
      setDisplayPos(livePct)
      smoothedYRef.current = livePct
      targetYRef.current = livePct
    }
  }, [livePct, dragging])

  function readPointer(e: React.PointerEvent) {
    const rect = padRef.current!.getBoundingClientRect()
    const relX = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
    const relY = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height))
    targetYRef.current = 1 - relY // top of pad = fully extended, bottom = retracted
    rawXRef.current = relX
    xOffsetRef.current = Math.abs(relX - 0.5) * 2 // 0 at center, 1 at either edge
  }

  function tick() {
    const chase = lerp(SNAPPY_CHASE, SOFT_CHASE, xOffsetRef.current)
    smoothedYRef.current += (targetYRef.current - smoothedYRef.current) * chase
    setDisplayPos(smoothedYRef.current)
    rafRef.current = requestAnimationFrame(tick)
  }

  function startDrag(e: React.PointerEvent) {
    e.currentTarget.setPointerCapture(e.pointerId)
    readPointer(e)
    smoothedYRef.current = targetYRef.current
    setDragging(true)

    rafRef.current = requestAnimationFrame(tick)
    intervalRef.current = setInterval(() => {
      const velocityPct = lerp(SNAPPY_VELOCITY, SOFT_VELOCITY, xOffsetRef.current)
      sendRef.current({ type: 'hdsp_move', positionPct: smoothedYRef.current, velocityPct })
    }, SEND_INTERVAL_MS)
  }

  function endDrag() {
    setDragging(false)
    if (rafRef.current !== null) { cancelAnimationFrame(rafRef.current); rafRef.current = null }
    if (intervalRef.current !== null) { clearInterval(intervalRef.current); intervalRef.current = null }
  }

  function stop() {
    endDrag()
    send({ type: 'hdsp_stop' })
  }

  // Clean up on unmount (e.g. switching to another program mid-drag).
  useEffect(() => () => {
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
    if (intervalRef.current !== null) clearInterval(intervalRef.current)
  }, [])

  const moveState = hdsp?.state ?? 'idle'
  const isMoving = moveState === 'moving'
  const knobX = dragging ? rawXRef.current : 0.5
  const knobY = 1 - displayPos
  const softness = xOffsetRef.current

  return (
    <div className="flex flex-col gap-4 p-4">
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-400">Position</span>
        <span className="text-sm font-mono font-semibold text-violet-400">
          {Math.round(displayPos * 100)}%
        </span>
      </div>

      <div
        ref={padRef}
        onPointerDown={startDrag}
        onPointerMove={(e) => { if (dragging) readPointer(e) }}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        className="relative w-full aspect-[4/5] rounded-3xl bg-slate-800 border border-slate-700 overflow-hidden select-none touch-none"
        style={{ touchAction: 'none' }}
      >
        {/* Center band — the "snappy" response zone */}
        <div className="absolute inset-y-0 left-1/2 -translate-x-1/2 w-1/5 bg-violet-500/10 border-x border-violet-500/20" />

        {/* Current position guide line */}
        <div
          className="absolute inset-x-0 h-px bg-violet-400/40"
          style={{ top: `${knobY * 100}%` }}
        />

        {/* Finger indicator */}
        <div
          className={`absolute w-8 h-8 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 shadow-lg transition-colors
            ${dragging ? 'bg-violet-400 border-violet-200' : 'bg-slate-600 border-slate-500'}`}
          style={{ left: `${knobX * 100}%`, top: `${knobY * 100}%` }}
        />

        {!dragging && (
          <div className="absolute inset-0 flex items-center justify-center pointer-events-none px-6 text-center">
            <span className="text-xs text-slate-500">
              Drag up/down to move · center = snappy, edges = soft
            </span>
          </div>
        )}
      </div>

      <div className="flex items-center justify-between text-xs text-slate-500 font-mono">
        <span>{isMoving ? 'Moving…' : 'Idle'}</span>
        <span>{dragging ? (softness < 0.35 ? 'Snappy' : softness > 0.7 ? 'Soft' : 'Blending') : ''}</span>
      </div>

      {/* Presets / Stop */}
      <div className="flex gap-2">
        {PRESETS.map((p) => (
          <button
            key={p.label}
            onClick={() => send({ type: 'hdsp_move', positionPct: p.value, velocityPct: 0.5 })}
            className="flex-1 py-3 bg-slate-800 hover:bg-slate-700 border border-slate-700 text-slate-400 hover:text-slate-200 text-sm rounded-xl transition-colors"
          >
            {p.label}
          </button>
        ))}
        <button
          onClick={stop}
          className="px-5 py-3 bg-slate-700 hover:bg-slate-600 text-slate-300 font-semibold text-sm rounded-xl transition-colors"
        >
          Stop
        </button>
      </div>
    </div>
  )
}
