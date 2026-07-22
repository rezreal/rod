/**
 * BleTransport — connects to the Rod SSCP GATT service via Web Bluetooth.
 *
 * SSCP Service UUID:  7e400001-b5a3-f393-e0a9-e50e24dc4179
 * Characteristics:
 *   Telemetry (notify)  7e400002-b5a3-f393-e0a9-e50e24dc4179
 *   Command  (write)    7e400003-b5a3-f393-e0a9-e50e24dc4179
 *   Ack      (notify)   7e400004-b5a3-f393-e0a9-e50e24dc4179
 *   DevInfo  (read)     7e400005-b5a3-f393-e0a9-e50e24dc4179
 *
 * Wire encoding: JSON for v1 (proto binary in v2 once prost-wasm is integrated).
 * The Rust side encodes Telemetry/Ack as JSON bytes and decodes Command JSON bytes.
 */
import type {
  Command,
  CommandAck,
  ConnectionState,
  DeviceInfo,
  ITransport,
  Telemetry,
} from '../types/sscp'

// The Handy FW4 service UUID — what the device advertises.
// The SSCP characteristics live *inside* this same service (Handy clients ignore
// unknown UUIDs). We filter by name prefix so the picker shows "OHD_hw…" devices
// and declare the Handy UUID as an optionalService so Web Bluetooth allows access.
const HANDY_SERVICE_UUID = '77834d26-40f7-11ee-be56-0242ac120002'

// SSCP characteristic UUIDs — must match src/sscp/service.rs.
const CHAR_TELEMETRY   = '7e400002-b5a3-f393-e0a9-e50e24dc4179'
const CHAR_COMMAND     = '7e400003-b5a3-f393-e0a9-e50e24dc4179'
const CHAR_ACK         = '7e400004-b5a3-f393-e0a9-e50e24dc4179'
const CHAR_DEVICE_INFO = '7e400005-b5a3-f393-e0a9-e50e24dc4179'

const encoder = new TextEncoder()
const decoder = new TextDecoder()

export class BleTransport implements ITransport {
  connectionState: ConnectionState = 'disconnected'

  onTelemetry: ((t: Telemetry) => void) | null = null
  onAck: ((ack: CommandAck) => void) | null = null
  onConnectionChange: ((state: ConnectionState) => void) | null = null
  onDeviceInfo: ((info: DeviceInfo) => void) | null = null

  private _device: BluetoothDevice | null = null
  private _cmdChar: BluetoothRemoteGATTCharacteristic | null = null

  static isSupported(): boolean {
    return typeof navigator !== 'undefined' && 'bluetooth' in navigator
  }

  async connect(): Promise<void> {
    this._setState('connecting')
    try {
      const device = await navigator.bluetooth.requestDevice({
        // Two independent OR filters so the device appears whether the browser
        // relies on the advertised name or the service UUID in the advertisement:
        //   • namePrefix — matches "OHD_hw3_…" Handy-style local names.
        //   • services   — matches any device advertising the Handy FW4 UUID
        //                  (more reliable on macOS / some Android versions).
        filters: [
          { namePrefix: 'OHD_hw' },
          { services: [HANDY_SERVICE_UUID] },
        ],
        optionalServices: [HANDY_SERVICE_UUID],
      })
      this._device = device
      device.addEventListener('gattserverdisconnected', () => this._onDisconnect())

      const server = await device.gatt!.connect()
      const service = await server.getPrimaryService(HANDY_SERVICE_UUID)

      // Subscribe to telemetry.
      // Large JSON frames are split into multiple ATT notifications (each ≤ ATT_MTU−3).
      // We accumulate bytes and try JSON.parse after each chunk; reset on success.
      const telChar = await service.getCharacteristic(CHAR_TELEMETRY)
      await telChar.startNotifications()
      let telBuf = ''
      telChar.addEventListener('characteristicvaluechanged', (e) => {
        const val = (e.target as BluetoothRemoteGATTCharacteristic).value!
        telBuf += decoder.decode(val)
        try {
          const t = JSON.parse(telBuf) as Telemetry
          this.onTelemetry?.(t)
          telBuf = ''
        } catch {
          // Incomplete JSON — wait for next chunk.
          // Safety valve: if it grows without ever completing, something is wrong.
          if (telBuf.length > 8192) {
            console.warn('SSCP: telemetry buffer overflow — resetting')
            telBuf = ''
          }
        }
      })

      // Subscribe to acks (short frames, rarely fragmented).
      const ackChar = await service.getCharacteristic(CHAR_ACK)
      await ackChar.startNotifications()
      let ackBuf = ''
      ackChar.addEventListener('characteristicvaluechanged', (e) => {
        const val = (e.target as BluetoothRemoteGATTCharacteristic).value!
        ackBuf += decoder.decode(val)
        try {
          const ack = JSON.parse(ackBuf) as CommandAck
          this.onAck?.(ack)
          ackBuf = ''
        } catch {
          if (ackBuf.length > 512) ackBuf = ''
        }
      })

      // Store command characteristic
      this._cmdChar = await service.getCharacteristic(CHAR_COMMAND)

      // Read device info on first connect. Deliver it out-of-band — NOT via a
      // synthetic telemetry frame, which would overwrite the live status slice
      // (e.g. actuatorConnected) with stale defaults that then stick, since the
      // bridge dedups unchanged telemetry and won't re-send the real value.
      try {
        const devInfoChar = await service.getCharacteristic(CHAR_DEVICE_INFO)
        const val = await devInfoChar.readValue()
        const text = decoder.decode(val)
        const info = JSON.parse(text) as DeviceInfo
        this.onDeviceInfo?.(info)
      } catch { /* optional */ }

      this._setState('connected')
    } catch (err) {
      this._setState('disconnected')
      throw err
    }
  }

  disconnect(): void {
    this._device?.gatt?.disconnect()
    this._device = null
    this._cmdChar = null
    this._setState('disconnected')
  }

  send(cmd: Command, seq: number): void {
    if (!this._cmdChar) return
    const payload = JSON.stringify({ seq, ...cmd })
    const bytes = encoder.encode(payload)
    // Fire-and-forget; ack comes back on notify channel
    this._cmdChar.writeValueWithoutResponse(bytes).catch(console.error)
  }

  private _onDisconnect() {
    this._cmdChar = null
    this._setState('reconnecting')
    // Attempt reconnect after short delay
    setTimeout(() => this._reconnect(), 2000)
  }

  private async _reconnect() {
    if (!this._device) return
    try {
      await this._device.gatt!.connect()
      // Re-subscribe would need full setup; for now surface as disconnected
      this._setState('disconnected')
    } catch {
      this._setState('disconnected')
    }
  }

  private _setState(s: ConnectionState) {
    this.connectionState = s
    this.onConnectionChange?.(s)
  }
}
