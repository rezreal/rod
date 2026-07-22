import { useRef, useState } from 'react'
import { useSendCommand } from '../../hooks/useSendCommand'
import { useStatus } from '../../hooks/useDeviceState'
import { parseFunscript, chunkPoints, scriptDurationMs, formatDuration } from '../../hooks/useFunscript'
import type { HspPoint } from '../../types/sscp'

export function HspControls() {
  const { hsp } = useStatus()
  const send = useSendCommand()
  const fileRef = useRef<HTMLInputElement>(null)

  const [points, setPoints] = useState<HspPoint[]>([])
  const [fileName, setFileName] = useState<string | null>(null)
  const [loop, setLoop] = useState(false)
  const [rate, setRate] = useState(1.0)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)

  const playState    = hsp?.state        ?? 'stopped'
  const bufferPoints = hsp?.bufferPoints ?? 0
  const isPlaying    = playState === 'playing'
  const isPaused     = playState === 'paused'
  const isStarving   = playState === 'starving'

  async function loadFile(file: File) {
    setLoadError(null)
    try {
      const text = await file.text()
      const parsed = parseFunscript(text)
      setPoints(parsed)
      setFileName(file.name)
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Invalid funscript file')
    }
  }

  async function handlePlay() {
    if (points.length === 0) return
    // Load chunks then play
    const chunks = chunkPoints(points)
    send({ type: 'hsp_load', points: chunks[0]!, append: false })
    for (let i = 1; i < chunks.length; i++) {
      send({ type: 'hsp_load', points: chunks[i]!, append: true })
    }
    send({ type: 'hsp_play', loop, rate })
  }

  function drop(e: React.DragEvent) {
    e.preventDefault()
    setDragging(false)
    const file = e.dataTransfer.files[0]
    if (file) loadFile(file)
  }

  return (
    <div className="flex flex-col gap-5 p-4">
      {/* Drop zone */}
      <div
        onDragOver={(e) => { e.preventDefault(); setDragging(true) }}
        onDragLeave={() => setDragging(false)}
        onDrop={drop}
        onClick={() => fileRef.current?.click()}
        className={`relative flex flex-col items-center justify-center gap-2 h-24 border-2 border-dashed rounded-2xl cursor-pointer transition-colors
          ${dragging
            ? 'border-emerald-500 bg-emerald-500/10'
            : fileName
            ? 'border-slate-700 bg-slate-800/50 hover:border-slate-600'
            : 'border-slate-700 bg-slate-800/30 hover:border-slate-600'
          }`}
      >
        <input
          ref={fileRef}
          type="file"
          accept=".funscript,.json"
          className="hidden"
          onChange={(e) => { const f = e.target.files?.[0]; if (f) loadFile(f) }}
        />
        {fileName ? (
          <>
            <svg viewBox="0 0 24 24" className="w-5 h-5 text-emerald-400" fill="none" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M9 12l2 2 4-4m6 2a9 9 0 1 1-18 0 9 9 0 0 1 18 0z" />
            </svg>
            <span className="text-sm text-slate-300 truncate max-w-[90%]">{fileName}</span>
            <span className="text-xs text-slate-500">
              {formatDuration(scriptDurationMs(points))} · {points.length} pts
            </span>
          </>
        ) : (
          <>
            <svg viewBox="0 0 24 24" className="w-6 h-6 text-slate-500" fill="none" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75V16.5m-13.5-9L12 3m0 0 4.5 4.5M12 3v13.5" />
            </svg>
            <span className="text-sm text-slate-500">Drop .funscript or tap to browse</span>
          </>
        )}
      </div>

      {loadError && (
        <p className="text-xs text-red-400 px-1">{loadError}</p>
      )}

      {/* Buffer */}
      {(isPlaying || isPaused || isStarving) && (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between text-xs text-slate-500">
            <span>Buffer</span>
            <span className={isStarving ? 'text-red-400' : ''}>{bufferPoints} pts</span>
          </div>
          <div className="h-2 bg-slate-700 rounded-full overflow-hidden">
            <div
              className={`h-full rounded-full transition-all ${isStarving ? 'bg-red-500' : 'bg-emerald-500'}`}
              style={{ width: `${Math.min(100, (bufferPoints / 500) * 100)}%` }}
            />
          </div>
        </div>
      )}

      {/* Rate */}
      <div className="flex items-center justify-between gap-4">
        <span className="text-sm text-slate-400 shrink-0">Rate</span>
        <div className="relative flex-1 h-10 flex items-center">
          <div className="absolute inset-x-0 h-2 bg-slate-700 rounded-full">
            <div className="h-full bg-emerald-500/50 rounded-full" style={{ width: `${((rate - 0.25) / 1.75) * 100}%` }} />
          </div>
          <input
            type="range" min={0.25} max={2} step={0.05}
            value={rate}
            onChange={(e) => setRate(parseFloat(e.target.value))}
            onPointerUp={(e) => {
              const v = parseFloat((e.target as HTMLInputElement).value)
              if (isPlaying) send({ type: 'hsp_rate', rate: v })
            }}
            className="absolute inset-0 w-full opacity-0 cursor-pointer"
          />
          <div
            className="absolute w-6 h-6 rounded-full bg-emerald-400 border-2 border-emerald-300 shadow-lg pointer-events-none"
            style={{ left: `calc(${((rate - 0.25) / 1.75) * 100}% - 12px)` }}
          />
        </div>
        <span className="text-sm font-mono font-semibold text-emerald-400 shrink-0 w-12 text-right">
          {rate.toFixed(2)}×
        </span>
      </div>

      {/* Loop toggle */}
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-400">Loop</span>
        <button
          onClick={() => setLoop(!loop)}
          className={`relative w-12 h-6 rounded-full transition-colors ${loop ? 'bg-emerald-600' : 'bg-slate-700'}`}
        >
          <span
            className={`absolute top-1 w-4 h-4 bg-white rounded-full shadow transition-transform ${loop ? 'translate-x-7' : 'translate-x-1'}`}
          />
        </button>
      </div>

      {/* Playback controls */}
      <div className="flex gap-2">
        {!isPlaying && !isPaused ? (
          <button
            onClick={handlePlay}
            disabled={points.length === 0}
            className="flex-1 flex items-center justify-center gap-2 py-4 bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 disabled:text-slate-500 text-white font-semibold rounded-2xl transition-colors min-h-[56px]"
          >
            <svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3" /></svg>
            Play
          </button>
        ) : (
          <>
            <button
              onClick={() => send(isPaused ? { type: 'hsp_play', loop, rate } : { type: 'hsp_pause' })}
              className="flex-1 flex items-center justify-center gap-2 py-4 bg-slate-700 hover:bg-slate-600 text-white font-semibold rounded-2xl transition-colors min-h-[56px]"
            >
              {isPaused ? (
                <><svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3" /></svg>Resume</>
              ) : (
                <><svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor"><rect x="6" y="6" width="4" height="12" rx="1" /><rect x="14" y="6" width="4" height="12" rx="1" /></svg>Pause</>
              )}
            </button>
            <button
              onClick={() => send({ type: 'hsp_stop' })}
              className="px-5 py-4 bg-slate-700 hover:bg-slate-600 text-slate-300 font-semibold rounded-2xl transition-colors"
            >
              <svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1" /></svg>
            </button>
          </>
        )}
      </div>
    </div>
  )
}
