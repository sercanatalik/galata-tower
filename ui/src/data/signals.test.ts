import { describe as group, expect, it } from 'vitest'

import { annualisedSigma, cellOf, describe, type HorizonFigures } from './signals'

const HOUR = 3_600_000_000
const figures = (over: Partial<HorizonFigures> = {}): HorizonFigures => ({
  horizon: '4h',
  width_micros: 4 * HOUR,
  asof_micros: 1_790_553_600_000_000,
  computed_micros: 1_790_553_600_000_001,
  stale: false,
  model: 'gjr-t/dcc',
  params: '{}',
  fitted: true,
  tickers: ['BTC', 'ETH'],
  cells: [{ measure: 'correlation', ticker_i: 'BTC', ticker_j: 'ETH', value: 0.87, absent: null, n_eff: 122 }],
  ...over,
})

group('a stored signal', () => {
  it('annualises a daily variance by 365 days', () => {
    expect(annualisedSigma(0.0004, 24 * HOUR)).toBeCloseTo(0.02 * Math.sqrt(365), 12)
  })

  it('finds a pair in either order', () => {
    expect(cellOf(figures(), 'correlation', 'ETH', 'BTC')?.value).toBe(0.87)
  })

  it('says stale in words', () => {
    const text = describe(figures({ stale: true }), 1_790_553_600_000_000 + 5 * HOUR)
    expect(text).toContain('stale: its next bar has closed')
    expect(text).toContain('(5.0h ago)')
  })

  it('says an unfitted figure was not fitted', () => {
    expect(describe(figures({ fitted: false, model: 'ewma' }), 1_790_553_600_000_000)).toContain('ewma, not fitted')
  })
})
