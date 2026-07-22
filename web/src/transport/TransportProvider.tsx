import React, { createContext, useContext, useEffect, useRef, useState } from 'react'
import { useDeviceStore } from '../store/deviceStore'
import { BleTransport } from './BleTransport'
import { WebRtcTransport } from './WebRtcTransport'
import { WebRtcHost } from './WebRtcHost'
import { OFFER_TTL_MS, readRemoteOfferFromHash } from './signaling'
import type { Command, ConnectionState, ITransport } from '../types/sscp'

type Role = 'host' | 'guest'

interface TransportContextValue {
  connectionState: ConnectionState
  isBleSupported: boolean
  connectBle(): Promise<void>
  disconnect(): void
  send(cmd: Command): void

  /** 'guest' if the app was opened from a remote-control share link. */
  role: Role
  // ── Guest role ──
  /** Process the shared offer and return the answer blob to hand back. */
  connectGuest(): Promise<string>
  // ── Host role (sharing remote control) ──
  /** A guest's data channel is currently open. */
  guestConnected: boolean
  /** Begin sharing: returns the offer blob (for the link / QR). */
  startShare(): Promise<string>
  /** Complete the handshake with the guest's answer blob. */
  acceptGuestAnswer(blob: string): Promise<void>
  /** Revoke the guest and tear the channel down. */
  stopShare(): void
  /** The current offer's expiry has passed and it was torn down. */
  offerExpired: boolean
  /** Epoch ms when the current offer expires (null if none / connected). */
  offerExpiresAt: number | null
  /** Discard the old offer and create a fresh one; returns the new blob. */
  regenerateShare(): Promise<string>
  /** A host share was active when the page was reloaded (the old link is dead). */
  shareInterrupted: boolean
  /** Acknowledge the interruption (after offering a fresh share). */
  clearShareInterrupted(): void
}

const TransportContext = createContext<TransportContextValue | null>(null)

/** sessionStorage flag: a host share was live in this tab. Survives a reload
 *  (same tab) but not a tab close — exactly the accidental-reload case. */
const SHARE_FLAG = 'rod.sharing'

export function TransportProvider({ children }: { children: React.ReactNode }) {
  const [connectionState, setConnectionState] = useState<ConnectionState>('disconnected')
  const [guestConnected, setGuestConnected] = useState(false)
  const [offerExpired, setOfferExpired] = useState(false)
  const [offerExpiresAt, setOfferExpiresAt] = useState<number | null>(null)
  const transportRef = useRef<ITransport | null>(null)
  const hostRef = useRef<WebRtcHost | null>(null)
  const expiryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const seqRef = useRef(0)
  const { setTelemetry, setConnectionState: storeSetState, setDeviceInfo } =
    useDeviceStore.getState()

  // Role is fixed for the lifetime of the page: a remote share link → guest.
  const [role] = useState<Role>(() => (readRemoteOfferFromHash() ? 'guest' : 'host'))
  const remoteOfferRef = useRef<string | null>(readRemoteOfferFromHash())

  // Captured at first render (before the flag-managing effect can clear it): was
  // a host share live when this tab was last reloaded?
  const [shareInterrupted, setShareInterrupted] = useState(
    () => !readRemoteOfferFromHash() && sessionStorage.getItem(SHARE_FLAG) === '1',
  )
  function clearShareInterrupted() {
    setShareInterrupted(false)
  }

  const isBleSupported = BleTransport.isSupported()

  function _mount(t: ITransport) {
    t.onConnectionChange = (s) => {
      setConnectionState(s)
      storeSetState(s)
    }
    t.onTelemetry = (tel) => {
      setTelemetry(tel)
      hostRef.current?.forwardTelemetry(tel) // mirror to a remote guest, if any
    }
    t.onDeviceInfo = (info) => {
      setDeviceInfo(info)
      hostRef.current?.forwardDeviceInfo(info) // mirror to a remote guest, if any
    }
    t.onAck = (ack) => {
      hostRef.current?.forwardAck(ack)
    }
    transportRef.current = t
  }

  async function connectBle() {
    const t = new BleTransport()
    _mount(t)
    await t.connect()
  }

  function clearExpiryTimer() {
    if (expiryTimerRef.current) {
      clearTimeout(expiryTimerRef.current)
      expiryTimerRef.current = null
    }
  }

  function disconnect() {
    clearExpiryTimer()
    hostRef.current?.close()
    hostRef.current = null
    setGuestConnected(false)
    setOfferExpired(false)
    setOfferExpiresAt(null)
    transportRef.current?.disconnect()
    transportRef.current = null
  }

  function send(cmd: Command) {
    transportRef.current?.send(cmd, seqRef.current++)
  }

  // ── Guest role ──
  async function connectGuest(): Promise<string> {
    const offer = remoteOfferRef.current
    if (!offer) throw new Error('no remote offer in link')
    const t = new WebRtcTransport()
    _mount(t)
    return t.acceptOffer(offer)
  }

  // ── Host role ──
  async function startShare(): Promise<string> {
    clearExpiryTimer()
    const host = new WebRtcHost()
    host.onGuestState = (c) => {
      setGuestConnected(c)
      if (c) {
        // Guest is in — the offer is spent; stop the expiry clock.
        clearExpiryTimer()
        setOfferExpiresAt(null)
      }
    }
    host.onCommand = (cmd, seq) => transportRef.current?.send(cmd, seq)
    hostRef.current = host
    setOfferExpired(false)
    const exp = Date.now() + OFFER_TTL_MS
    setOfferExpiresAt(exp)
    // Host-side enforcement: if no guest connects in time, tear the offer down
    // so a leaked link can no longer complete a handshake.
    expiryTimerRef.current = setTimeout(() => {
      if (hostRef.current && !hostRef.current.connected) {
        hostRef.current.close()
        hostRef.current = null
        setOfferExpired(true)
        setOfferExpiresAt(null)
      }
    }, OFFER_TTL_MS)
    return host.createOffer(exp)
  }

  async function regenerateShare(): Promise<string> {
    clearExpiryTimer()
    hostRef.current?.close()
    hostRef.current = null
    setGuestConnected(false)
    return startShare()
  }

  async function acceptGuestAnswer(blob: string): Promise<void> {
    if (!hostRef.current) {
      throw new Error('The share link has expired — regenerate it and reshare.')
    }
    await hostRef.current.acceptAnswer(blob)
  }

  function stopShare() {
    clearExpiryTimer()
    hostRef.current?.close()
    hostRef.current = null
    setGuestConnected(false)
    setOfferExpired(false)
    setOfferExpiresAt(null)
  }

  useEffect(() => {
    return () => {
      clearExpiryTimer()
      hostRef.current?.close()
      transportRef.current?.disconnect()
    }
  }, [])

  // A WebRTC handshake/session can't survive a reload (the RTCPeerConnection
  // isn't serializable and the shared link is bound to it). The best we can do
  // is warn before an accidental reload/close while a share is pending or live.
  const sharePending = offerExpiresAt != null && !offerExpired
  const guestActive = role === 'guest' && connectionState === 'connected'
  const shareActive = (role === 'host' && (guestConnected || sharePending)) || guestActive
  useEffect(() => {
    if (!shareActive) return
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      e.preventDefault()
      e.returnValue = '' // triggers the browser's "Leave site?" confirmation
    }
    window.addEventListener('beforeunload', onBeforeUnload)
    return () => window.removeEventListener('beforeunload', onBeforeUnload)
  }, [shareActive])

  // Mark/unmark the persistent "sharing" flag so a reload mid-share is detected
  // on the next load. A reload skips the false→clear path, so the flag survives.
  useEffect(() => {
    if (role !== 'host') return
    if (role === 'host' && (guestConnected || sharePending)) {
      sessionStorage.setItem(SHARE_FLAG, '1')
    } else {
      sessionStorage.removeItem(SHARE_FLAG)
    }
  }, [role, guestConnected, sharePending])

  return (
    <TransportContext.Provider
      value={{
        connectionState,
        isBleSupported,
        connectBle,
        disconnect,
        send,
        role,
        connectGuest,
        guestConnected,
        startShare,
        acceptGuestAnswer,
        stopShare,
        offerExpired,
        offerExpiresAt,
        regenerateShare,
        shareInterrupted,
        clearShareInterrupted,
      }}
    >
      {children}
    </TransportContext.Provider>
  )
}

export function useTransport(): TransportContextValue {
  const ctx = useContext(TransportContext)
  if (!ctx) throw new Error('useTransport must be used inside TransportProvider')
  return ctx
}
