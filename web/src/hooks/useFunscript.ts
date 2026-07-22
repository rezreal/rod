import type { HspPoint } from '../types/sscp'

interface FunscriptAction {
  at: number   // ms
  pos: number  // 0–100
}

interface Funscript {
  actions: FunscriptAction[]
}

/** Parse a .funscript file (JSON) and convert to SSCP HspPoint array */
export function parseFunscript(json: string): HspPoint[] {
  const data = JSON.parse(json) as Funscript
  if (!Array.isArray(data.actions)) throw new Error('No actions array')
  return data.actions
    .sort((a, b) => a.at - b.at)
    .map((action) => ({
      timeMs: action.at,
      position: Math.round((action.pos / 100) * 255),
    }))
}

/** Split points into BLE-friendly chunks (max ~20 points per write) */
export function chunkPoints(points: HspPoint[], chunkSize = 20): HspPoint[][] {
  const chunks: HspPoint[][] = []
  for (let i = 0; i < points.length; i += chunkSize) {
    chunks.push(points.slice(i, i + chunkSize))
  }
  return chunks
}

/** Duration of a funscript in milliseconds */
export function scriptDurationMs(points: HspPoint[]): number {
  if (points.length === 0) return 0
  return points[points.length - 1]!.timeMs
}

/** Format a duration as "m:ss" */
export function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000)
  const m = Math.floor(s / 60)
  const sec = s % 60
  return `${m}:${sec.toString().padStart(2, '0')}`
}
