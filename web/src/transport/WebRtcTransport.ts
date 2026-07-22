// Guest-side transport: drives the device remotely over a WebRTC data channel.
// Implements ITransport, so the rest of the app (TransportProvider, every
// program's controls) works unchanged — commands go out over the channel and
// telemetry/acks come back, instead of BLE.

import type { Command, CommandAck, ConnectionState, DeviceInfo, ITransport, Telemetry } from '../types/sscp'
import {
  decodeSignal,
  encodeSignal,
  ICE_SERVERS,
  isExpired,
  OfferExpiredError,
  waitIceComplete,
} from './signaling'
import type { WireMessage } from './webrtcProtocol'

export class WebRtcTransport implements ITransport {
  connectionState: ConnectionState = 'disconnected'
  onTelemetry: ((t: Telemetry) => void) | null = null
  onAck: ((ack: CommandAck) => void) | null = null
  onConnectionChange: ((state: ConnectionState) => void) | null = null
  onDeviceInfo: ((info: DeviceInfo) => void) | null = null

  private pc: RTCPeerConnection | null = null
  private ch: RTCDataChannel | null = null

  static isAvailable(): boolean {
    return typeof RTCPeerConnection !== 'undefined'
  }

  /** The guest joins via {@link acceptOffer}, not connect(). */
  async connect(): Promise<void> {
    throw new Error('WebRtcTransport: use acceptOffer(offer) for the guest role')
  }

  /**
   * Process the host's offer and produce an answer blob (to send back to the
   * host). The connection completes — and `onConnectionChange('connected')`
   * fires — once the host accepts that answer and the data channel opens.
   */
  async acceptOffer(offerBlob: string): Promise<string> {
    this.setState('connecting')
    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS })
    this.pc = pc
    // The host creates the data channel; the guest receives it here.
    pc.ondatachannel = (e) => this.bind(e.channel)
    pc.onconnectionstatechange = () => {
      const s = pc.connectionState
      if (s === 'failed' || s === 'disconnected' || s === 'closed') {
        this.setState('disconnected')
      }
    }

    const offer = await decodeSignal(offerBlob)
    if (isExpired(offer)) {
      this.disconnect()
      throw new OfferExpiredError()
    }
    await pc.setRemoteDescription(offer)
    const answer = await pc.createAnswer()
    await pc.setLocalDescription(answer)
    await waitIceComplete(pc)
    return encodeSignal(pc.localDescription!)
  }

  private bind(ch: RTCDataChannel) {
    this.ch = ch
    ch.onopen = () => this.setState('connected')
    ch.onclose = () => this.setState('disconnected')
    ch.onmessage = (e) => {
      let m: WireMessage
      try {
        m = JSON.parse(e.data as string)
      } catch {
        return
      }
      if (m.t === 'tel') this.onTelemetry?.(m.tel)
      else if (m.t === 'ack') this.onAck?.(m.ack)
      else if (m.t === 'info') this.onDeviceInfo?.(m.info)
      else if (m.t === 'bye') this.disconnect()
    }
  }

  send(cmd: Command, seq: number): void {
    if (this.ch?.readyState === 'open') {
      this.ch.send(JSON.stringify({ t: 'cmd', cmd, seq } satisfies WireMessage))
    }
  }

  disconnect(): void {
    try {
      this.ch?.close()
    } catch {
      /* ignore */
    }
    try {
      this.pc?.close()
    } catch {
      /* ignore */
    }
    this.ch = null
    this.pc = null
    this.setState('disconnected')
  }

  private setState(s: ConnectionState) {
    this.connectionState = s
    this.onConnectionChange?.(s)
  }
}
