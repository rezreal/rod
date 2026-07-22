import { useEffect, useState } from 'react'
import { useTransport } from '../../transport/TransportProvider'
import { OfferExpiredError, readRemoteOfferFromHash } from '../../transport/signaling'
import { QrCode } from './QrCode'

export function GuestConnectScreen() {
  const { connectGuest, connectionState } = useTransport()
  const [answer, setAnswer] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [expired, setExpired] = useState(false)
  const [copied, setCopied] = useState(false)
  const [attempt, setAttempt] = useState(0)

  const hasOffer = readRemoteOfferFromHash() !== null
  const isConnecting = connectionState === 'connecting'

  // Runs once on mount and again whenever the user taps "Try again" (attempt++).
  useEffect(() => {
    if (!hasOffer) return
    let cancelled = false
    setError(null)
    setAnswer(null)
    setExpired(false)
    connectGuest()
      .then((blob) => {
        if (!cancelled) setAnswer(blob)
      })
      .catch((e) => {
        if (!cancelled) {
          if (e instanceof OfferExpiredError) setExpired(true)
          const msg = e instanceof Error ? e.message : String(e)
          setError(msg)
        }
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [attempt])

  async function handleCopy() {
    if (!answer) return
    try {
      await navigator.clipboard.writeText(answer)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // clipboard unavailable — the textarea is still selectable
    }
  }

  return (
    <div className="flex flex-col items-center justify-center flex-1 px-6 py-8 gap-6 overflow-y-auto">
      {/* Header */}
      <div className="flex flex-col items-center gap-3">
        <div className="w-16 h-16 rounded-2xl bg-cyan-500/10 border border-cyan-500/30 flex items-center justify-center">
          <svg viewBox="0 0 24 24" className="w-8 h-8 text-cyan-400" fill="none" stroke="currentColor" strokeWidth={1.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M9.348 14.652a3.75 3.75 0 0 1 0-5.304m5.304 0a3.75 3.75 0 0 1 0 5.304m-7.425 2.121a6.75 6.75 0 0 1 0-9.546m9.546 0a6.75 6.75 0 0 1 0 9.546M5.106 18.894c-3.808-3.807-3.808-9.98 0-13.788m13.788 0c3.808 3.807 3.808 9.98 0 13.788M12 12h.008v.008H12V12z" />
          </svg>
        </div>
        <div className="text-center">
          <h1 className="text-2xl font-bold text-slate-100">Remote control</h1>
          <p className="text-sm text-slate-400 mt-2 max-w-sm leading-relaxed">
            You're connecting to someone's device. Show this answer code back to
            them to finish connecting.
          </p>
        </div>
      </div>

      {/* No offer in link, or an expired offer */}
      {(!hasOffer || expired) && (
        <div className="w-full max-w-sm px-4 py-3 bg-amber-500/10 border border-amber-500/30 rounded-xl text-sm text-amber-300 text-center">
          {expired
            ? 'This link has expired. Ask the host to share a new one.'
            : 'This link is invalid or expired.'}
        </div>
      )}

      {/* Error + retry (non-expiry failures only) */}
      {hasOffer && error && !expired && (
        <div className="w-full max-w-sm flex flex-col gap-3">
          <div className="px-4 py-3 bg-red-500/10 border border-red-500/30 rounded-xl text-sm text-red-400 select-text cursor-text">
            {error}
          </div>
          <button
            onClick={() => setAttempt((n) => n + 1)}
            className="py-3 bg-cyan-600 hover:bg-cyan-500 text-white font-semibold rounded-2xl transition-colors"
          >
            Try again
          </button>
        </div>
      )}

      {/* Generating answer */}
      {hasOffer && !error && !answer && (
        <div className="flex items-center gap-3 text-slate-400 text-sm">
          <span className="w-5 h-5 border-2 border-cyan-400/30 border-t-cyan-400 rounded-full animate-spin" />
          Preparing your answer code…
        </div>
      )}

      {/* Answer */}
      {hasOffer && !error && answer && (
        <div className="w-full max-w-sm flex flex-col items-center gap-4">
          <QrCode value={answer} />

          <div className="w-full flex flex-col gap-2">
            <label className="text-xs text-slate-500 uppercase tracking-wider">
              Answer code
            </label>
            <textarea
              readOnly
              value={answer}
              onFocus={(e) => e.currentTarget.select()}
              className="w-full h-24 px-3 py-2 bg-slate-900 border border-slate-700 rounded-xl text-xs font-mono text-slate-300 resize-none select-text"
            />
            <button
              onClick={handleCopy}
              className="py-2.5 bg-slate-700 hover:bg-slate-600 text-slate-100 text-sm font-semibold rounded-xl transition-colors"
            >
              {copied ? 'Copied!' : 'Copy answer code'}
            </button>
          </div>

          {/* Status */}
          <div className="flex items-center gap-2 text-sm text-slate-400">
            <span className="w-2.5 h-2.5 rounded-full bg-amber-400 animate-pulse" />
            {isConnecting ? 'Connecting…' : 'Waiting for host to accept…'}
          </div>
        </div>
      )}

      <p className="text-xs text-slate-600 text-center max-w-sm">
        You will be controlling a real, physical device.
      </p>
    </div>
  )
}
