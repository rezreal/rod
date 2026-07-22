// IAI PCON-C ALMC alarm code descriptions
const ALARM_CODES: Record<number, string> = {
  0:  'No alarm',
  1:  'Flash memory error',
  2:  'Encoder communication error',
  3:  'Encoder count error',
  6:  'Origin return timeout',
  7:  'Driver initialisation error',
  0x0A: 'Overload',
  0x0B: 'Overspeed',
  0x0C: 'Motor current overload',
  0x0D: 'Position deviation overflow',
  0x10: 'Power circuit over-temperature',
  0x11: 'Motor over-temperature',
  0x19: 'Drive voltage low',
  0x1A: 'Drive voltage high',
  0x1B: 'Regenerative overload',
  0x26: 'Overload (continuous)',
  0x28: 'Emergency stop',
  0x30: 'Software limit reached',
  0x34: 'Home-return error',
  0x35: 'Push-motion timeout',
  0xA0: 'Control flag (CTLF) error',
  0xA3: 'Motion profile not supported',
}

export function describeAlarm(code: number): string {
  return ALARM_CODES[code] ?? `Alarm 0x${code.toString(16).toUpperCase().padStart(2, '0')}`
}

export function isAlarmCritical(code: number): boolean {
  // Major alarms that require homing after reset
  return [0x34, 0x0D, 0x0B, 0x0A, 0x28].includes(code)
}
