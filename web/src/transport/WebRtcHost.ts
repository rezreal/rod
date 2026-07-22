// Host-side relay: lets a remote guest drive this (BLE-connected) app over a
// WebRTC data channel. The host keeps its own BLE transport; this object only
// bridges the channel — guest commands are handed to `onCommand` (wired to the
// BLE send path), and telemetry/acks are pushed to the guest via forward*().

import type { Command, CommandAck, DeviceInfo, Telemetry } from '../types/sscp'
import { decodeSignal, encodeSignal, ICE_SERVERS, waitIceComplete } from './signaling'
import type { WireMessage } from './webrtcProtocol'

export class WebRtcHost {
  /** Whether a guest's data channel is currently open. */
  connected = false
  onGuestState: ((connected: boolean) => void) | null = null
  /** A command arrived from the guest — feed it to the device. */
  onCommand: ((cmd: Command, seq: number) => void) | null = null

  private pc: RTCPeerConnection | null = null
  private ch: RTCDataChannel | null = null
  /** Last device info seen on the host, replayed to each guest as it joins. */
  private lastDeviceInfo: DeviceInfo | null = null

  static isAvailable(): boolean {
    return typeof RTCPeerConnection !== 'undefined'
  }

  /**
   * Create the offer blob to share with the guest (link / QR). `exp` (epoch ms)
   * is embedded so the guest can detect an expired link; the host enforces it by
   * tearing this connection down (see TransportProvider).
   */
  async createOffer(exp?: number): Promise<string> {
    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS })
    this.pc = pc
    this.bind(pc.createDataChannel('ctl', { ordered: true }))
    pc.onconnectionstatechange = () => {
      const s = pc.connectionState
      if (s === 'failed' || s === 'disconnected' || s === 'closed') this.setGuest(false)
    }
    const offer = await pc.createOffer()
    await pc.setLocalDescription(offer)
    await waitIceComplete(pc)
    return encodeSignal(pc.localDescription!, exp)
  }

  /** Accept the guest's answer blob to complete the handshake. */
  async acceptAnswer(answerBlob: string): Promise<void> {
    if (!this.pc) throw new Error('createOffer() must run first')
    const answer = await decodeSignal(answerBlob)
    await this.pc.setRemoteDescription(answer)
  }

  forwardTelemetry(tel: Telemetry): void {
    this.sendWire({ t: 'tel', tel })
  }

  forwardAck(ack: CommandAck): void {
    this.sendWire({ t: 'ack', ack })
  }

  /** Cache device info and push it to the guest (now, and again on each join). */
  forwardDeviceInfo(info: DeviceInfo): void {
    this.lastDeviceInfo = info
    this.sendWire({ t: 'info', info })
  }

  /** Revoke the guest and tear the connection down. */
  close(): void {
    this.sendWire({ t: 'bye' })
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
    this.setGuest(false)
  }

  private bind(ch: RTCDataChannel) {
    this.ch = ch
    ch.onopen = () => {
      this.setGuest(true)
      // A guest typically joins after the host already read device info, so
      // replay the cached copy — there will be no fresh read to forward.
      if (this.lastDeviceInfo) this.sendWire({ t: 'info', info: this.lastDeviceInfo })
    }
    ch.onclose = () => this.setGuest(false)
    ch.onmessage = (e) => {
      let m: WireMessage
      try {
        m = JSON.parse(e.data as string)
      } catch {
        return
      }
      if (m.t === 'cmd') this.onCommand?.(m.cmd, m.seq)
    }
  }

  private sendWire(m: WireMessage) {
    if (this.ch?.readyState === 'open') this.ch.send(JSON.stringify(m))
  }

  private setGuest(c: boolean) {
    if (this.connected !== c) {
      this.connected = c
      this.onGuestState?.(c)
    }
  }
}
