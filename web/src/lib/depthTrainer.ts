import type { DepthPhase } from '../types/sscp'

/** Seconds-before-event at which the soft Depth Trainer audio cue fires. */
export const CUE_SOON_THRESHOLD_S = 2

/**
 * True when the bridge's `cueSoonS` (null, or seconds to the next advance /
 * planned retract when <= 3 s) crosses down through the cue threshold. Fires
 * once per crossing: the previous sample must be null or still above it.
 */
export function crossedCueSoon(prevS: number | null | undefined, nowS: number | null | undefined): boolean {
  if (nowS === null || nowS === undefined || nowS > CUE_SOON_THRESHOLD_S) return false
  return prevS === null || prevS === undefined || prevS > CUE_SOON_THRESHOLD_S
}

/** Per-excursion cap rule (mirrors the bridge's ABS_MAX_S = 20 s; display/manual only). */
export function submergedCapS(breathHoldS: number): number {
  return Math.min(0.6 * breathHoldS, 20)
}

/** Mandatory rest rule (mirrors the bridge; display/manual only). */
export function mandatoryRestS(submergedS: number): number {
  return Math.max(3 * submergedS, 20)
}

const PHASE_LABELS: Record<DepthPhase, string> = {
  arming: 'Armed — waiting for hand switch',
  warmup: 'Warm-up',
  oscillation: 'Oscillation set',
  hold: 'Hold set',
  rest: 'Rest',
  cooldown: 'Cool-down',
  retracting: 'Retracting to origin',
  rating: 'Rate this session',
  ended: 'Session ended',
}

/** Human label for a phase; falls back to the raw string for unknown phases. */
export function phaseLabel(phase: string): string {
  return (PHASE_LABELS as Record<string, string>)[phase] ?? phase
}

/** Format seconds as `m:ss` (rounded up). */
export function fmtClock(s: number): string {
  const total = Math.max(0, Math.ceil(s))
  const m = Math.floor(total / 60)
  return `${m}:${(total % 60).toString().padStart(2, '0')}`
}

/** Format a long countdown (seconds) as `Xh Ym`, or "now" when elapsed. */
export function fmtLongCountdown(s: number): string {
  if (s <= 0) return 'now'
  const h = Math.floor(s / 3600)
  const m = Math.ceil((s % 3600) / 60)
  if (h <= 0) return `${m} min`
  return m === 60 ? `${h + 1}h 0m` : `${h}h ${m}m`
}
