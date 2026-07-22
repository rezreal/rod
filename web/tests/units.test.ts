import { describe, it, expect } from 'vitest'
import { clamp, fmtMm, fmtPct, estimateVelocity } from '../src/lib/units'

describe('units', () => {
  it('clamps values', () => {
    expect(clamp(0.5, 0, 1)).toBe(0.5)
    expect(clamp(-0.1, 0, 1)).toBe(0)
    expect(clamp(1.5, 0, 1)).toBe(1)
  })

  it('formats mm', () => {
    expect(fmtMm(127.4)).toBe('127.4 mm')
  })

  it('formats percentage', () => {
    expect(fmtPct(0.512)).toBe('51')
  })

  it('estimates velocity from position delta', () => {
    // 50mm move in 100ms on a 200mm stroke
    const v = estimateVelocity(0.0, 0.25, 100, 200)
    expect(v).toBe(500) // mm/s
  })
})
