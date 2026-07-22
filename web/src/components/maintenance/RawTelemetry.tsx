import { useDeviceStore } from '../../store/deviceStore'
import { usePosition, useStatus } from '../../hooks/useDeviceState'

export function RawTelemetry() {
  const connectionState = useDeviceStore((s) => s.connectionState)
  const deviceInfo      = useDeviceStore((s) => s.deviceInfo)
  const pos    = usePosition()
  const status = useStatus()

  if (connectionState !== 'connected' && connectionState !== 'reconnecting') {
    return <p className="text-sm text-slate-500 px-4 py-4">No telemetry yet.</p>
  }

  const rows: [string, string][] = [
    ['position_mm',      pos.positionMm.toFixed(2)],
    ['position_pct',     pos.positionPct.toFixed(4)],
    ['direction',        pos.direction],
    ['moving',           String(pos.moving)],
    ['mode',             status.mode],
    ['servo_on',         String(status.servoOn)],
    ['controller_ready', String(status.controllerReady)],
    ['homed',            String(status.homed)],
    ['pos_done',         String(status.positioningDone)],
    ['push_active',      String(status.pushActive)],
    ['brake_released',   String(status.brakeReleased)],
    ['alarm_code',       `0x${status.alarmCode.toString(16).toUpperCase().padStart(2,'0')}`],
    ['alarm_minor',      String(status.alarmMinor)],
    ['alarm_major',      String(status.alarmMajor)],
    ['e_stop',           String(status.emergencyStop)],
    ['volt_low',         String(status.motorVoltageLow)],
    ['safety_speed',     String(status.safetySpeed)],
  ]

  if (status.hamp) {
    rows.push(
      ['hamp.running',  String(status.hamp.running)],
      ['hamp.velocity', status.hamp.velocity.toFixed(3)],
      ['hamp.zone_min', status.hamp.zoneMin.toFixed(3)],
      ['hamp.zone_max', status.hamp.zoneMax.toFixed(3)],
      ['hamp.softness', status.hamp.softness.toFixed(3)],
    )
  }

  if (status.hdsp) {
    rows.push(['hdsp.state', status.hdsp.state])
  }

  if (status.hsp) {
    rows.push(
      ['hsp.state',         status.hsp.state],
      ['hsp.buffer_points', String(status.hsp.bufferPoints)],
      ['hsp.rate',          status.hsp.playbackRate.toFixed(2)],
    )
  }

  if (deviceInfo) {
    rows.push(
      ['device.name',   deviceInfo.deviceName],
      ['device.stroke', `${deviceInfo.strokeMm} mm`],
      ['device.fw',     deviceInfo.firmwareVersion],
      ['sscp.version',  String(deviceInfo.sscpVersion)],
    )
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs font-mono">
        <tbody>
          {rows.map(([key, val]) => (
            <tr key={key} className="border-b border-slate-800 hover:bg-slate-800/50">
              <td className="py-1.5 px-4 text-slate-500 w-40">{key}</td>
              <td className="py-1.5 px-4 text-slate-300">{val}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
