import { describe, expect, it } from 'vitest'

import { model, shock, type Statistics } from './risk'

const v = (value: number) => ({ value: { value, n: 65, backfilled_share: 1 } })

const stats: Statistics = {
  bucket_secs: 1800,
  from_micros: 0,
  to_micros: 1,
  annualisation: Math.sqrt(17520),
  volatility: { BTC: v(0.5), ETH: v(0.58) },
  correlation: { 'BTC|ETH': { rho: v(0.7), interval: { low: 0.55, high: 0.81 } } },
  beta: {},
  reference: 'BTC',
}

describe('the modelled figures', () => {
  it('one position is its own one-day sigma, and all of the risk', () => {
    const m = model({ source: 'held', items: [{ ticker: 'BTC', usd: 100_000 }] }, stats)
    if (m.kind !== 'modelled') throw new Error(m.missing)
    expect(m.sigmaDay).toBeCloseTo((100_000 * 0.5) / Math.sqrt(365), 6)
    expect(m.var95).toBeCloseTo(1.645 * m.sigmaDay, 9)
    expect(m.shares[0].share).toBeCloseTo(1, 12)
  })

  it('hypothetical sizes are labelled', () => {
    const m = model({ source: 'hypothetical', items: [{ ticker: 'BTC', usd: 1 }] }, stats)
    expect(m.source).toBe('hypothetical')
  })

  it('an absent correlation makes VaR absent, naming the pair', () => {
    const thin: Statistics = {
      ...stats,
      correlation: { 'BTC|ETH': { rho: { absent: { instrument: 'ETH', count: 3, floor: 20 } }, interval: null } },
    }
    const m = model(
      { source: 'held', items: [{ ticker: 'BTC', usd: 1 }, { ticker: 'ETH', usd: 1 }] },
      thin,
    )
    expect(m).toEqual({ kind: 'absent', source: 'held', missing: 'ρ of BTC|ETH' })
  })

  it('a hedge has a negative share', () => {
    const m = model(
      { source: 'held', items: [{ ticker: 'BTC', usd: 100 }, { ticker: 'ETH', usd: -30 }] },
      stats,
    )
    if (m.kind !== 'modelled') throw new Error(m.missing)
    expect(m.shares.find((s) => s.ticker === 'ETH')!.share).toBeLessThan(0)
    expect(m.shares.reduce((a, s) => a + s.share, 0)).toBeCloseTo(1, 12)
  })
})

describe('shocks, through β', () => {
  it('moves each held leg by its β on the shocked one', () => {
    const s = shock(
      { source: 'held', items: [{ ticker: 'BTC', usd: 100 }, { ticker: 'ETH', usd: 200 }] },
      stats,
      'BTC',
      -0.1,
    )
    if (s.kind !== 'modelled') throw new Error(s.missing)
    expect(s.pnl).toBeCloseTo(100 * -0.1 + 200 * ((0.7 * 0.58) / 0.5) * -0.1, 12)
  })

  it('a leg not held moves nothing', () => {
    const s = shock({ source: 'hypothetical', items: [{ ticker: 'ETH', usd: 50 }] }, stats, 'BTC', 0.1)
    if (s.kind !== 'modelled') throw new Error(s.missing)
    expect(s.pnl).toBeCloseTo(50 * ((0.7 * 0.58) / 0.5) * 0.1, 12)
    expect(s.source).toBe('hypothetical')
  })

  it('an absent correlation makes the shock absent, naming the pair', () => {
    const thin: Statistics = {
      ...stats,
      correlation: { 'BTC|ETH': { rho: { absent: { instrument: 'ETH', count: 3, floor: 20 } }, interval: null } },
    }
    const s = shock({ source: 'held', items: [{ ticker: 'ETH', usd: 1 }] }, thin, 'BTC', -0.1)
    expect(s).toEqual({ kind: 'absent', source: 'held', missing: 'ρ of BTC|ETH' })
  })

  it('a shock on a leg with no σ is absent', () => {
    const s = shock({ source: 'held', items: [{ ticker: 'BTC', usd: 1 }] }, stats, 'GOLD', -0.05)
    expect(s).toEqual({ kind: 'absent', source: 'held', missing: 'σ of GOLD' })
  })
})
