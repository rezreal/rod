import { describe, it, expect } from 'vitest'
import { encodeSignal, decodeSignal, buildRemoteLink, readRemoteOfferFromHash, isExpired } from '../src/transport/signaling'

describe('serverless signaling codec', () => {
  it('round-trips a session description', async () => {
    const desc = {
      type: 'offer' as const,
      sdp: 'v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=candidate:1 1 udp 2113 1.2.3.4 50000 typ srflx\r\n',
    }
    const blob = await encodeSignal(desc)
    expect(blob.length).toBeGreaterThan(1)
    const back = await decodeSignal(blob)
    expect(back.type).toBe('offer')
    expect(back.sdp).toBe(desc.sdp)
  })

  it('builds and reads a remote link', async () => {
    const blob = await encodeSignal({ type: 'offer', sdp: 'v=0\r\n' })
    const link = buildRemoteLink(blob)
    expect(link).toContain('#remote=')
    // Simulate opening the link.
    location.hash = `#remote=${blob}`
    expect(readRemoteOfferFromHash()).toBe(blob)
    location.hash = ''
    expect(readRemoteOfferFromHash()).toBeNull()
  })

  it('round-trips and enforces an offer expiry', async () => {
    const past = Date.now() - 10 * 60 * 1000
    const future = Date.now() + 10 * 60 * 1000

    const expiredBlob = await encodeSignal({ type: 'offer', sdp: 'v=0\r\n' }, past)
    const expiredSig = await decodeSignal(expiredBlob)
    expect(expiredSig.exp).toBe(past)
    expect(isExpired(expiredSig)).toBe(true)

    const freshBlob = await encodeSignal({ type: 'offer', sdp: 'v=0\r\n' }, future)
    expect(isExpired(await decodeSignal(freshBlob))).toBe(false)

    // No expiry stamp → never considered expired.
    expect(isExpired(await decodeSignal(await encodeSignal({ type: 'offer', sdp: 'v=0\r\n' })))).toBe(false)
  })
})
