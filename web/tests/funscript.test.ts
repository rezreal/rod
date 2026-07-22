import { describe, it, expect } from 'vitest'
import { parseFunscript, chunkPoints, scriptDurationMs, formatDuration } from '../src/hooks/useFunscript'

const SAMPLE = JSON.stringify({
  actions: [
    { at: 0,    pos: 0   },
    { at: 1000, pos: 100 },
    { at: 2000, pos: 50  },
    { at: 3000, pos: 0   },
  ],
})

describe('parseFunscript', () => {
  it('converts actions to HspPoints', () => {
    const pts = parseFunscript(SAMPLE)
    expect(pts).toHaveLength(4)
    expect(pts[0]).toEqual({ timeMs: 0,    position: 0   })
    expect(pts[1]).toEqual({ timeMs: 1000, position: 255 })
    expect(pts[2]).toEqual({ timeMs: 2000, position: 128 }) // round(50/100*255)
    expect(pts[3]).toEqual({ timeMs: 3000, position: 0   })
  })

  it('sorts actions by time', () => {
    const unsorted = JSON.stringify({
      actions: [{ at: 2000, pos: 50 }, { at: 0, pos: 0 }, { at: 1000, pos: 100 }],
    })
    const pts = parseFunscript(unsorted)
    expect(pts[0]!.timeMs).toBe(0)
    expect(pts[1]!.timeMs).toBe(1000)
    expect(pts[2]!.timeMs).toBe(2000)
  })
})

describe('chunkPoints', () => {
  it('splits into chunks of the given size', () => {
    const pts = parseFunscript(SAMPLE)
    const chunks = chunkPoints(pts, 2)
    expect(chunks).toHaveLength(2)
    expect(chunks[0]).toHaveLength(2)
    expect(chunks[1]).toHaveLength(2)
  })
})

describe('scriptDurationMs', () => {
  it('returns the last timestamp', () => {
    expect(scriptDurationMs(parseFunscript(SAMPLE))).toBe(3000)
  })

  it('returns 0 for empty array', () => {
    expect(scriptDurationMs([])).toBe(0)
  })
})

describe('formatDuration', () => {
  it('formats seconds as m:ss', () => {
    expect(formatDuration(134_000)).toBe('2:14')
    expect(formatDuration(60_000)).toBe('1:00')
    expect(formatDuration(5_000)).toBe('0:05')
  })
})
