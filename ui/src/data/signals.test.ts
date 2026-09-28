import { describe as group, expect, it } from 'vitest'

import { annualisedSigma, betaOf, cellOf, describe, type HorizonFigures, universe } from './signals'

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

const market = (cells: Array<[string, number | null, string | null]>): HorizonFigures =>
  figures({ cells: cells.map(([measure, value, absent]) => ({ measure, ticker_i: '*', ticker_j: null, value, absent, n_eff: null })) })

group('the derived figures', () => {
  it('btc against itself', () => {
    const beta = figures({ cells: [{ measure: 'beta', ticker_i: 'ETH', ticker_j: 'BTC', value: 1.05, absent: null, n_eff: null }, { measure: 'idiosyncratic_share', ticker_i: 'ETH', ticker_j: 'BTC', value: 0.22, absent: null, n_eff: null }] })
    expect(betaOf(beta, 'BTC')).toEqual(['1', '—'])
    expect(betaOf(beta, 'ETH')).toEqual(['1.05', '22%'])
  })

  it('the universe in one line', () => {
    const line = universe(
      market([['correlation_ar', 0.61, null], ['covariance_ar', 0.7, null]]),
      market([['mahalanobis', 4.99, null], ['chi2_percentile', 0.45, null], ['magnitude_surprise', 0.73, null], ['correlation_surprise', 1.14, null]]),
      market([['turbulence', 1.4, null], ['percentile', 0.29, null]]),
      6,
    )
    expect(line).toContain('absorption 0.61 on ρ, 0.70 on Σ (floor 1/6 = 0.17)')
    expect(line).toContain('Mahalanobis 5.0, 45th pct of χ²6')
    expect(line).toContain('turbulence 1.4, 29th pct of its sample')
  })

  it('an absent figure keeps its reason', () => {
    const line = universe(undefined, market([['mahalanobis', null, 'the covariance matrix is not positive definite']]), undefined, 6)
    expect(line).toContain('Mahalanobis absent (the covariance matrix is not positive definite)')
    expect(line).toContain('absorption absent (no figure for this horizon)')
  })
})
