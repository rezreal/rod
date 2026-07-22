import { useEffect, useRef, useState } from 'react'
import jsQR from 'jsqr'

interface Props {
  onResult: (text: string) => void
  onCancel: () => void
}

/**
 * Opens the rear camera and decodes a QR code from the live feed via jsQR on a
 * requestAnimationFrame loop. Calls `onResult` exactly once on success, then
 * stops the camera. If the camera is unavailable / denied, shows a friendly
 * message and relies on the parent's paste fallback.
 */
export function QrScanner({ onResult, onCancel }: Props) {
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let stream: MediaStream | null = null
    let rafId = 0
    let done = false
    const canvas = document.createElement('canvas')
    const ctx = canvas.getContext('2d', { willReadFrequently: true })

    function stop() {
      done = true
      cancelAnimationFrame(rafId)
      if (stream) {
        for (const track of stream.getTracks()) track.stop()
        stream = null
      }
    }

    function scan() {
      if (done) return
      const video = videoRef.current
      if (video && ctx && video.readyState === video.HAVE_ENOUGH_DATA) {
        canvas.width = video.videoWidth
        canvas.height = video.videoHeight
        ctx.drawImage(video, 0, 0, canvas.width, canvas.height)
        try {
          const img = ctx.getImageData(0, 0, canvas.width, canvas.height)
          const code = jsQR(img.data, img.width, img.height, {
            inversionAttempts: 'dontInvert',
          })
          if (code && code.data) {
            stop()
            onResult(code.data)
            return
          }
        } catch {
          // ignore transient frame errors and keep scanning
        }
      }
      rafId = requestAnimationFrame(scan)
    }

    async function start() {
      if (!navigator.mediaDevices?.getUserMedia) {
        setError('Camera not available on this device.')
        return
      }
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: 'environment' },
        })
        if (done) {
          for (const track of stream.getTracks()) track.stop()
          return
        }
        const video = videoRef.current
        if (video) {
          video.srcObject = stream
          await video.play().catch(() => {})
        }
        rafId = requestAnimationFrame(scan)
      } catch {
        setError('Could not access the camera. Paste the code instead.')
      }
    }

    void start()
    return stop
    // onResult is stable enough for this one-shot scanner; we only mount once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  if (error) {
    return (
      <div className="flex flex-col gap-3">
        <div className="px-4 py-3 bg-amber-500/10 border border-amber-500/30 rounded-xl text-sm text-amber-300">
          {error}
        </div>
        <button
          onClick={onCancel}
          className="py-2.5 bg-slate-700 hover:bg-slate-600 text-slate-100 text-sm font-semibold rounded-xl transition-colors"
        >
          Close camera
        </button>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="relative overflow-hidden rounded-2xl bg-black border border-slate-700">
        <video
          ref={videoRef}
          playsInline
          muted
          className="w-full aspect-square object-cover"
        />
        <div className="pointer-events-none absolute inset-6 border-2 border-cyan-400/70 rounded-xl" />
      </div>
      <p className="text-xs text-slate-500 text-center">
        Point the camera at the guest's answer QR code.
      </p>
      <button
        onClick={onCancel}
        className="py-2.5 bg-slate-700 hover:bg-slate-600 text-slate-100 text-sm font-semibold rounded-xl transition-colors"
      >
        Cancel
      </button>
    </div>
  )
}
