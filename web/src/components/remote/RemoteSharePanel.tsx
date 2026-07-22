import { useEffect, useRef, useState } from 'react'
import { useTransport } from '../../transport/TransportProvider'
import { buildRemoteLink } from '../../transport/signaling'
import { QrCode } from './QrCode'
import { QrScanner } from './QrScanner'

interface Props {
  onClose: () => void
  /** Reopened after a reload interrupted a previous share. */
  recovered?: boolean
}

/**
 * Host-side modal for handing remote control to a guest over WebRTC.
 *  1. Show the shareable link (+ QR) built from our offer.
 *  2. Take the guest's answer code (camera scan or paste) and complete it.
 *  3. Once the guest is connected, offer a prominent take-back control.
 */
export function RemoteSharePanel({ onClose, recovered = false }: Props) {
  const {
    startShare,
    acceptGuestAnswer,
    stopShare,
    guestConnected,
    offerExpired,
    offerExpiresAt,
    regenerateShare,
  } = useTransport()

  const [link, setLink] = useState<string | null>(null)
  const [startError, setStartError] = useState<string | null>(null)
  const [linkCopied, setLinkCopied] = useState(false)

  const [scanning, setScanning] = useState(false)
  const [pasted, setPasted] = useState('')
  const [accepting, setAccepting] = useState(false)
  const [acceptError, setAcceptError] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const startedRef = useRef(false)

  // Begin sharing once on open.
  useEffect(() => {
    if (startedRef.current) return
    startedRef.current = true
    startShare()
      .then((offer) => setLink(buildRemoteLink(offer)))
      .catch((e) => setStartError(e instanceof Error ? e.message : String(e)))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Tick a 1s clock while an unexpired offer is outstanding (for the countdown).
  useEffect(() => {
    if (!offerExpiresAt || guestConnected) return
    const id = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [offerExpiresAt, guestConnected])

  async function handleRegenerate() {
    setLink(null)
    setStartError(null)
    setAcceptError(null)
    setLinkCopied(false)
    try {
      const offer = await regenerateShare()
      setLink(buildRemoteLink(offer))
    } catch (e) {
      setStartError(e instanceof Error ? e.message : String(e))
    }
  }

  const remainingMs = offerExpiresAt ? Math.max(0, offerExpiresAt - now) : 0
  const remaining = `${Math.floor(remainingMs / 60000)}:${String(
    Math.floor((remainingMs % 60000) / 1000),
  ).padStart(2, '0')}`

  async function handleCopyLink() {
    if (!link) return
    try {
      await navigator.clipboard.writeText(link)
      setLinkCopied(true)
      setTimeout(() => setLinkCopied(false), 2000)
    } catch {
      // ignore — textarea is selectable
    }
  }

  async function accept(blob: string) {
    const trimmed = blob.trim()
    if (!trimmed) return
    setAccepting(true)
    setAcceptError(null)
    try {
      await acceptGuestAnswer(trimmed)
    } catch (e) {
      setAcceptError(e instanceof Error ? e.message : String(e))
    } finally {
      setAccepting(false)
    }
  }

  function handleScanResult(text: string) {
    setScanning(false)
    void accept(text)
  }

  function handleStop() {
    stopShare()
    onClose()
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="w-full sm:w-96 max-h-[90vh] bg-slate-900 border border-slate-800 rounded-t-3xl sm:rounded-2xl overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-slate-800">
          <h2 className="font-semibold text-slate-100">Share remote control</h2>
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
          {guestConnected ? (
            /* ── Connected: take-back control ── */
            <div className="flex flex-col gap-4">
              <div className="flex items-start gap-3 p-4 bg-amber-900/30 border border-amber-700/50 rounded-xl">
                <svg viewBox="0 0 24 24" className="w-6 h-6 text-amber-400 shrink-0" fill="none" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M18 18.72a9.094 9.094 0 0 0 3.741-.479 3 3 0 0 0-4.682-2.72m.94 3.198.001.031c0 .225-.012.447-.037.666A11.944 11.944 0 0 1 12 21c-2.17 0-4.207-.576-5.963-1.584A6.062 6.062 0 0 1 6 18.719m12 0a5.971 5.971 0 0 0-.941-3.197m0 0A5.995 5.995 0 0 0 12 12.75a5.995 5.995 0 0 0-5.058 2.772m0 0a3 3 0 0 0-4.681 2.72 8.986 8.986 0 0 0 3.74.477m.94-3.197a5.971 5.971 0 0 0-.94 3.197M15 6.75a3 3 0 1 1-6 0 3 3 0 0 1 6 0z" />
                </svg>
                <div>
                  <p className="text-sm font-semibold text-amber-200">Guest connected</p>
                  <p className="text-xs text-amber-300/80 mt-1">
                    They now control the device. Take back control at any time.
                  </p>
                </div>
              </div>
              <button
                onClick={handleStop}
                className="flex items-center justify-center gap-2 py-3.5 bg-red-600 hover:bg-red-500 text-white font-semibold rounded-2xl transition-colors min-h-[56px]"
              >
                <svg viewBox="0 0 24 24" className="w-5 h-5" fill="currentColor">
                  <rect x="6" y="6" width="12" height="12" rx="1" />
                </svg>
                Stop &amp; take back control
              </button>
            </div>
          ) : (
            <>
              {recovered && (
                <div className="px-4 py-3 bg-amber-500/10 border border-amber-500/30 rounded-xl text-sm text-amber-300">
                  Your previous share was interrupted by a reload, so the old link no
                  longer works. Share this new link instead.
                </div>
              )}

              {/* ── Step 1: share link ── */}
              <div className="flex flex-col gap-3">
                <div>
                  <h3 className="text-sm font-semibold text-slate-200">1. Share this link</h3>
                  <p className="text-xs text-slate-500 mt-1">
                    Send this link to the person you want to give control to.
                  </p>
                </div>

                {offerExpired ? (
                  <div className="flex flex-col gap-3">
                    <div className="px-4 py-3 bg-amber-500/10 border border-amber-500/30 rounded-xl text-sm text-amber-300">
                      This link expired and was disabled. Generate a new one to share again.
                    </div>
                    <button
                      onClick={handleRegenerate}
                      className="w-full py-2.5 bg-cyan-600 hover:bg-cyan-500 text-white text-sm font-semibold rounded-xl transition-colors"
                    >
                      Generate new link
                    </button>
                  </div>
                ) : startError ? (
                  <div className="px-4 py-3 bg-red-500/10 border border-red-500/30 rounded-xl text-sm text-red-400 select-text">
                    {startError}
                  </div>
                ) : !link ? (
                  <div className="flex items-center gap-3 text-slate-400 text-sm py-2">
                    <span className="w-5 h-5 border-2 border-cyan-400/30 border-t-cyan-400 rounded-full animate-spin" />
                    Generating share link…
                  </div>
                ) : (
                  <div className="flex flex-col items-center gap-3">
                    <QrCode value={link} />
                    <textarea
                      readOnly
                      value={link}
                      onFocus={(e) => e.currentTarget.select()}
                      className="w-full h-16 px-3 py-2 bg-slate-950 border border-slate-700 rounded-xl text-xs font-mono text-slate-300 resize-none select-text"
                    />
                    <button
                      onClick={handleCopyLink}
                      className="w-full py-2.5 bg-slate-700 hover:bg-slate-600 text-slate-100 text-sm font-semibold rounded-xl transition-colors"
                    >
                      {linkCopied ? 'Copied!' : 'Copy link'}
                    </button>
                    <div className="flex items-center justify-between w-full text-xs text-slate-500">
                      <span>Expires in {remaining}</span>
                      <button
                        onClick={handleRegenerate}
                        className="text-cyan-400 hover:text-cyan-300 font-medium"
                      >
                        Regenerate
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* ── Step 2: enter their answer ── */}
              {!offerExpired && (
              <div className="flex flex-col gap-3 pt-1 border-t border-slate-800">
                <div className="pt-3">
                  <h3 className="text-sm font-semibold text-slate-200">2. Enter their answer code</h3>
                  <p className="text-xs text-slate-500 mt-1">
                    Scan the QR they show you, or paste their answer code.
                  </p>
                </div>

                {acceptError && (
                  <div className="px-4 py-3 bg-red-500/10 border border-red-500/30 rounded-xl text-sm text-red-400 select-text">
                    {acceptError}
                  </div>
                )}

                {scanning ? (
                  <QrScanner onResult={handleScanResult} onCancel={() => setScanning(false)} />
                ) : (
                  <button
                    onClick={() => { setAcceptError(null); setScanning(true) }}
                    disabled={accepting}
                    className="flex items-center justify-center gap-2 py-2.5 bg-cyan-600 hover:bg-cyan-500 disabled:opacity-50 text-white text-sm font-semibold rounded-xl transition-colors"
                  >
                    <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M6.827 6.175A2.31 2.31 0 0 1 5.186 7.23c-.38.054-.757.112-1.134.175C2.999 7.58 2.25 8.507 2.25 9.574V18a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9.574c0-1.067-.75-1.994-1.802-2.169a47.865 47.865 0 0 0-1.134-.175 2.31 2.31 0 0 1-1.64-1.055l-.822-1.316a2.192 2.192 0 0 0-1.736-1.039 48.774 48.774 0 0 0-5.232 0 2.192 2.192 0 0 0-1.736 1.039l-.821 1.316z" />
                      <path strokeLinecap="round" strokeLinejoin="round" d="M16.5 12.75a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0zM18.75 10.5h.008v.008h-.008V10.5z" />
                    </svg>
                    Scan answer with camera
                  </button>
                )}

                <div className="flex flex-col gap-2">
                  <textarea
                    value={pasted}
                    onChange={(e) => setPasted(e.target.value)}
                    placeholder="Paste answer code here…"
                    className="w-full h-20 px-3 py-2 bg-slate-950 border border-slate-700 rounded-xl text-xs font-mono text-slate-300 resize-none placeholder:text-slate-600"
                  />
                  <button
                    onClick={() => void accept(pasted)}
                    disabled={accepting || !pasted.trim()}
                    className="flex items-center justify-center gap-2 py-2.5 bg-cyan-600 hover:bg-cyan-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-sm font-semibold rounded-xl transition-colors"
                  >
                    {accepting ? (
                      <>
                        <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                        Connecting…
                      </>
                    ) : 'Connect'}
                  </button>
                </div>
              </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  )
}
