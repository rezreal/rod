import { useEffect, useRef } from 'react'
import { usePosition } from '../../hooks/useDeviceState'

interface Props {
  className?: string
}

const CYAN = '#22d3ee'
const CYAN_DIM = 'rgba(34,211,238,0.12)'

export function WaveformChart({ className = '' }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const { waveform } = usePosition()

  useEffect(() => {
    // Drive canvas drawing via rAF so it never executes during a React commit
    // and can be coalesced with the browser's own paint cycle.
    let raf: number
    raf = requestAnimationFrame(() => { draw() })
    return () => cancelAnimationFrame(raf)

    function draw() {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const dpr = window.devicePixelRatio ?? 1
    const w = canvas.clientWidth
    const h = canvas.clientHeight

    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr
      canvas.height = h * dpr
      ctx.scale(dpr, dpr)
    }

    ctx.clearRect(0, 0, w, h)

    if (waveform.length < 2) {
      // Empty state
      ctx.strokeStyle = 'rgba(100,116,139,0.2)'
      ctx.setLineDash([4, 6])
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(0, h / 2)
      ctx.lineTo(w, h / 2)
      ctx.stroke()
      ctx.setLineDash([])
      return
    }

    const pts = waveform
    const step = w / (pts.length - 1)

    // Gradient fill
    const grad = ctx.createLinearGradient(0, 0, 0, h)
    grad.addColorStop(0, 'rgba(34,211,238,0.25)')
    grad.addColorStop(1, 'rgba(34,211,238,0)')

    ctx.beginPath()
    pts.forEach((v, i) => {
      const x = i * step
      const y = h - v * h
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    })
    // Close path downward for fill
    ctx.lineTo((pts.length - 1) * step, h)
    ctx.lineTo(0, h)
    ctx.closePath()
    ctx.fillStyle = grad
    ctx.fill()

    // Line
    ctx.beginPath()
    pts.forEach((v, i) => {
      const x = i * step
      const y = h - v * h
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    })
    ctx.strokeStyle = CYAN
    ctx.lineWidth = 1.5
    ctx.lineJoin = 'round'
    ctx.stroke()

    // Leading dot
    const lastV = pts[pts.length - 1]!
    const lx = (pts.length - 1) * step
    const ly = h - lastV * h
    ctx.beginPath()
    ctx.arc(lx, ly, 3, 0, Math.PI * 2)
    ctx.fillStyle = CYAN
    ctx.fill()
    ctx.beginPath()
    ctx.arc(lx, ly, 6, 0, Math.PI * 2)
    ctx.fillStyle = CYAN_DIM
    ctx.fill()
    } // end draw()
  }, [waveform])

  return (
    <div className={`relative ${className}`}>
      <canvas
        ref={canvasRef}
        className="w-full h-full"
        aria-hidden="true"
      />
      {waveform.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center text-xs text-slate-600">
          waiting for data…
        </div>
      )}
    </div>
  )
}
