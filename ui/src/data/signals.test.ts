import { describe as group, expect, it } from 'vitest'

import { annualisedSigma, betaOf, cellOf, chartPoints, constancy, describe, monitorOf, groupNote, type HorizonFigures, instrumentRows, universe } from './signals'

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

group('the constancy test', () => {
  const tested = (p: number | null, absent: string | null = null) => ({ ...market([['engle_sheppard_p', p, absent]]), params: '{"a":0.0,"b":0.0,"days":30,"lags":5}' })
  it('a p-value, its window, and whether it rejects', () => {
    expect(constancy(tested(0.574))).toBe('constant correlation (Engle–Sheppard over 30 days, 5 lags): p 0.574, no regime flag (flagged below 0.1%)')
    expect(constancy(tested(0.004))).toContain('p 0.004, no regime flag')
    expect(constancy(tested(0.0004))).toContain('p 4.0e-4, below 0.1%: a correlation regime flag')
  })
  it('an absent test keeps its reason, and an untested horizon says nothing', () => {
    expect(constancy(tested(null, '264 returns, under min_obs=500'))).toBe('constant correlation: test absent (264 returns, under min_obs=500)')
    expect(constancy(undefined)).toBeNull()
  })
})

group('the sequential monitor', () => {
  const params = (extra: object) => JSON.stringify({ epoch_start: '2026-09-24T00:00:00+00:00', m: 830, k: 743, ...extra })
  it('no alarm, with how close it came', () => {
    const h = { ...market([['wied_galeano_alarm', 0, null], ['wied_galeano_ratio', 0.98, null]]), params: params({ pair: null, change_at: null }) }
    expect(monitorOf(h)).toBe('sequential monitor (Wied–Galeano since 2026-09-24, 743 returns against a baseline of 830): no alarm, nearest pair at 0.98 of its boundary')
  })
  it('an alarm names its pair and when the correlation changed', () => {
    const h = { ...market([['wied_galeano_alarm', 1, null], ['wied_galeano_ratio', 1.34, null]]), params: params({ pair: ['BTC', 'GOLD'], change_at: '2026-09-26T14:05:00+00:00' }) }
    expect(monitorOf(h)).toContain('alarm on BTC|GOLD, its correlation changed around')
  })
  it('an absent monitor keeps its reason, and an unmonitored horizon says nothing', () => {
    expect(monitorOf(market([['wied_galeano_alarm', null, 'a baseline of 360 returns is too short']]))).toBe('sequential monitor: absent (a baseline of 360 returns is too short)')
    expect(monitorOf(undefined)).toBeNull()
  })
})

group('the history chart', () => {
  it('a gap is not a zero', () => {
    const pts = chartPoints([
      { asof_micros: 1_790_553_600_000_000, computed_micros: 1, value: 0.87, absent: null },
      { asof_micros: 1_790_568_000_000_000, computed_micros: 2, value: null, absent: 'under min_obs' },
    ])
    expect(pts[0]).toEqual({ time: 1_790_553_600, value: 0.87 })
    expect(pts[1]).toEqual({ time: 1_790_568_000 })
  })
})

group('the instruments panel', () => {
  const one = (signal: string, cells: Array<[string, string, number | null, string | null]>): HorizonFigures =>
    figures({ horizon: signal === 'jumps' ? '5m' : '1h', cells: cells.map(([t, m, v, a]) => ({ measure: m, ticker_i: t, ticker_j: null, value: v, absent: a, n_eff: null })) })

  it('one row per instrument, every signals columns', () => {
    const rows = instrumentRows({
      carry: one('carry', [['BTC', 'carry_apr_7d', 0.1095, null], ['GOLD', 'carry_apr_7d', 0.0548, null]]),
      jumps: one('jumps', [['BTC', 'jumps_up_24h', 1, null]]),
      liquidity: one('liquidity', [['GOLD', 'quoted_spread_bps', 0.2485, null], ['BTC', 'depth_usd_median', 125462, null]]),
    })
    expect(rows.map((r) => r.ticker)).toEqual(['BTC', 'GOLD'])
    const btc = rows[0].cells.map((c) => c.text)
    expect(btc[0]).toBe('10.95%')
    expect(btc[3]).toBe('1')
    expect(btc[9]).toBe('$125k')
    expect(rows[1].cells[7].text).toBe('0.25')
  })

  it('the basis: premium and mark in bps, open interest as a percent change and in dollars', () => {
    const rows = instrumentRows({
      basis: one('basis', [['CL', 'premium_twa_bps', -4.5309, null], ['CL', 'mark_oracle_bps', -3.6928, null], ['CL', 'open_interest_log_change', 0.172446, null], ['CL', 'open_interest_usd', 1.346e8, null]]),
    })
    expect(rows[0].cells.slice(12, 16).map((c) => c.text)).toEqual(['-4.53', '-3.69', '+18.8%', '$135M'])
  })

  it('the order flow: its fit and both imbalances', () => {
    const rows = instrumentRows({ flow: one('flow', [['BTC', 'ofi_r2', 0.5556, null], ['BTC', 'trade_imbalance_1h', 0.2467, null], ['BTC', 'queue_imbalance_twa', 0.325, null]]) })
    expect(rows[0].cells.slice(16, 19).map((c) => c.text)).toEqual(['0.56', '0.25', '0.33'])
  })

  it('the realized moments', () => {
    const rows = instrumentRows({ moments: one('moments', [['XYZ100', 'realized_skew_1d', -3.952, null], ['XYZ100', 'realized_kurt_1d', 44.89, null], ['XYZ100', 'realized_skew_7d', -1.2918, null]]) })
    expect(rows[0].cells.slice(19, 22).map((c) => c.text)).toEqual(['-3.95', '44.9', '-1.29'])
  })

  it('the inferred liquidations', () => {
    const rows = instrumentRows({ cascade: one('cascade', [['CL', 'liq_intensity', 0.0412, null], ['CL', 'cascade_events', 1, null]]) })
    expect(rows[0].cells.slice(22, 24).map((c) => c.text)).toEqual(['4.12%', '1'])
  })

  it('the abnormal activity', () => {
    const rows = instrumentRows({ activity: one('activity', [['BTC', 'volume_z', 2.34, null], ['BTC', 'large_share', 0.587, null]]) })
    expect(rows[0].cells.slice(24).map((c) => c.text)).toEqual(['2.3', '59%'])
  })

  it('an absent figure keeps its reason', () => {
    const rows = instrumentRows({ carry: one('carry', [['GOLD', 'carry_apr_7d', null, 'settled funding covers 0 of 168 hours']]) })
    expect(rows[0].cells[0]).toEqual({ text: 'absent', title: 'settled funding covers 0 of 168 hours' })
    expect(rows[0].cells[3]).toEqual({ text: '—', title: 'no jumps stored' })
    expect(groupNote('jumps', undefined, 0)).toBe('jumps: none stored')
  })
})
