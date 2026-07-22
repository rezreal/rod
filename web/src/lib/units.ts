/** Format a position percentage as a 0–100 integer string */
export function fmtPct(pct: number): string {
  return `${Math.round(pct * 100)}`
}

/** Format mm with one decimal place */
export function fmtMm(mm: number): string {
  return `${mm.toFixed(1)} mm`
}

/** Format velocity mm/s */
export function fmtVelocity(mmPerSec: number): string {
  return `${Math.round(mmPerSec)} mm/s`
}

/** Clamp a value between min and max */
export function clamp(val: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, val))
}

/** Linear interpolate */
export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

/** Convert a 0–1 zone position to mm given stroke */
export function pctToMm(pct: number, strokeMm: number): number {
  return pct * strokeMm
}

/** Map a 0–1 percentage to a CSS percentage string */
export function toCssPct(pct: number): string {
  return `${(pct * 100).toFixed(1)}%`
}

/** Estimate velocity from two position samples */
export function estimateVelocity(
  pos1: number,
  pos2: number,
  dtMs: number,
  strokeMm: number,
): number {
  if (dtMs === 0) return 0
  return (Math.abs(pos2 - pos1) * strokeMm) / (dtMs / 1000)
}
