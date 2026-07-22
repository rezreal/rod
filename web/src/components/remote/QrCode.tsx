import { useEffect, useRef, useState } from 'react'
import QRCode from 'qrcode'

interface Props {
  value: string
  className?: string
}

/**
 * Renders `value` as a QR code on a light rounded card (so it scans well in a
 * dark UI). Uses low error-correction and a generous width to fit long blobs.
 * If the value is too large for any QR version, shows a fallback note instead.
 */
export function QrCode({ value, className }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const [error, setError] = useState(false)

  useEffect(() => {
    let cancelled = false
    setError(false)
    const canvas = canvasRef.current
    if (!canvas) return

    QRCode.toCanvas(canvas, value, {
      errorCorrectionLevel: 'L',
      width: 256,
      margin: 2,
      color: { dark: '#0a0f1a', light: '#ffffff' },
    }).catch(() => {
      if (!cancelled) setError(true)
    })

    return () => {
      cancelled = true
    }
  }, [value])

  if (error) {
    return (
      <div
        className={`flex items-center justify-center text-center text-xs text-amber-400 bg-slate-800/60 border border-slate-700 rounded-2xl p-4 ${className ?? ''}`}
        style={{ minHeight: 120 }}
      >
        Too long for a QR code — use the link or copy the text instead.
      </div>
    )
  }

  return (
    <div className={`inline-flex bg-white rounded-2xl p-3 ${className ?? ''}`}>
      <canvas ref={canvasRef} className="block" />
    </div>
  )
}
