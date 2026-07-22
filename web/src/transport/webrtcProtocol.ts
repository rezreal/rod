import type { Command, CommandAck, DeviceInfo, Telemetry } from '../types/sscp'

/** Messages tunneled over the WebRTC data channel between guest and host. */
export type WireMessage =
  | { t: 'cmd'; cmd: Command; seq: number } // guest → host
  | { t: 'tel'; tel: Telemetry } // host → guest
  | { t: 'ack'; ack: CommandAck } // host → guest
  | { t: 'info'; info: DeviceInfo } // host → guest (sent on guest join)
  | { t: 'bye' } // either side, on revoke/close
