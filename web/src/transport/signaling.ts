// Serverless WebRTC signaling helpers.
//
// We do *non-trickle* signaling: wait for ICE gathering to finish so the SDP
// carries all candidates, then pack the whole session description into one
// compact string that can travel in a URL fragment or a QR code. No signaling
// server, no TURN — just a public STUN server for the public (srflx) candidate.

/** Public STUN servers (free; only discover the public IP — they relay nothing). */
export const ICE_SERVERS: RTCIceServer[] = [
  { urls: 'stun:stun.l.google.com:19302' },
  { urls: 'stun:stun1.l.google.com:19302' },
]

/** URL-fragment key carrying a remote-control offer for the guest. */
export const REMOTE_HASH_KEY = 'remote'

/** How long a shared offer stays valid before the host tears it down. */
export const OFFER_TTL_MS = 5 * 60 * 1000

/** Clock-skew grace on the guest's expiry check (host-side teardown is the real
 *  enforcement; this is only for the friendly "expired" message). */
const EXP_SKEW_MS = 30 * 1000

/** A decoded signal — a session description plus an optional expiry (epoch ms). */
export type Signal = RTCSessionDescriptionInit & { exp?: number }

/** Thrown by the guest when the shared offer link has expired. */
export class OfferExpiredError extends Error {
  constructor() {
    super('This remote-control link has expired — ask for a new one.')
    this.name = 'OfferExpiredError'
  }
}

function bytesToBase64Url(bytes: Uint8Array): string {
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function base64UrlToBytes(s: string): Uint8Array {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/')
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

async function pump(bytes: Uint8Array, stream: TransformStream): Promise<Uint8Array> {
  const writer = stream.writable.getWriter()
  void writer.write(bytes)
  void writer.close()
  const chunks: Uint8Array[] = []
  const reader = stream.readable.getReader()
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    if (value) chunks.push(value)
  }
  const len = chunks.reduce((n, c) => n + c.length, 0)
  const out = new Uint8Array(len)
  let off = 0
  for (const c of chunks) {
    out.set(c, off)
    off += c.length
  }
  return out
}

async function deflate(bytes: Uint8Array): Promise<Uint8Array | null> {
  if (typeof CompressionStream === 'undefined') return null
  try {
    return await pump(bytes, new CompressionStream('deflate-raw'))
  } catch {
    return null // environment without working streams → fall back to raw
  }
}

async function inflate(bytes: Uint8Array): Promise<Uint8Array> {
  return pump(bytes, new DecompressionStream('deflate-raw'))
}

/**
 * Encode a session description to a compact string. Format: a 1-char codec
 * marker (`d` = deflated, `r` = raw) followed by base64url. The SDP is highly
 * compressible, so the deflated form fits a QR comfortably.
 */
export async function encodeSignal(desc: RTCSessionDescriptionInit, exp?: number): Promise<string> {
  const json = JSON.stringify({ t: desc.type, s: desc.sdp ?? '', ...(exp ? { e: exp } : {}) })
  const raw = new TextEncoder().encode(json)
  const deflated = await deflate(raw)
  if (deflated && deflated.length < raw.length) {
    return 'd' + bytesToBase64Url(deflated)
  }
  return 'r' + bytesToBase64Url(raw)
}

/** Decode a string produced by {@link encodeSignal}. */
export async function decodeSignal(str: string): Promise<Signal> {
  const codec = str[0]
  const bytes = base64UrlToBytes(str.slice(1))
  const raw = codec === 'd' ? await inflate(bytes) : bytes
  const obj = JSON.parse(new TextDecoder().decode(raw)) as { t: RTCSdpType; s: string; e?: number }
  return { type: obj.t, sdp: obj.s, exp: obj.e }
}

/** True if the offer's embedded expiry has passed (with skew grace). */
export function isExpired(sig: Signal): boolean {
  return sig.exp != null && Date.now() > sig.exp + EXP_SKEW_MS
}

/**
 * Resolve once ICE gathering is complete (so the local description contains all
 * candidates), or after `timeoutMs` — whichever comes first. The timeout means a
 * slow/blocked STUN round-trip can't stall the handshake forever; we just send
 * whatever candidates we have.
 */
export function waitIceComplete(pc: RTCPeerConnection, timeoutMs = 3000): Promise<void> {
  if (pc.iceGatheringState === 'complete') return Promise.resolve()
  return new Promise((resolve) => {
    let done = false
    const finish = () => {
      if (done) return
      done = true
      pc.removeEventListener('icegatheringstatechange', check)
      resolve()
    }
    const check = () => {
      if (pc.iceGatheringState === 'complete') finish()
    }
    pc.addEventListener('icegatheringstatechange', check)
    setTimeout(finish, timeoutMs)
  })
}

/** Build the shareable guest link carrying an offer in the URL fragment. */
export function buildRemoteLink(offerBlob: string): string {
  const base = location.origin + location.pathname
  return `${base}#${REMOTE_HASH_KEY}=${offerBlob}`
}

/** Extract an offer blob from the current URL fragment, if present. */
export function readRemoteOfferFromHash(): string | null {
  const hash = location.hash.startsWith('#') ? location.hash.slice(1) : location.hash
  for (const part of hash.split('&')) {
    const [k, v] = part.split('=')
    if (k === REMOTE_HASH_KEY && v) return v
  }
  return null
}
