import type { UTCTimestamp } from 'lightweight-charts'

import type { components } from '../contract/api'
import { forHumans, stamp } from '../kit/format'

export type HorizonFigures = components['schemas']['HorizonFigures']
export type SignalCell = components['schemas']['SignalCell']

/** Microseconds in the calendar year a market that never closes trades. */
const YEAR_MICROS = 365 * 86_400_000_000

/** σ a year from a covariance per bar, by the width the server stated: √(cov × year / width). */
export function annualisedSigma(covariance: number, widthMicros: number): number {
  return Math.sqrt((covariance * YEAR_MICROS) / widthMicros)
}

/** The stored cell for a pair and measure, in either order. */
export function cellOf(h: HorizonFigures, measure: string, a: string, b: string): SignalCell | undefined {
  return h.cells.find(
    (c) => c.measure === measure && ((c.ticker_i === a && c.ticker_j === b) || (c.ticker_i === b && c.ticker_j === a)),
  )
}

/**
 * What a stored figure is, in words: model, asof, age, and whether its next
 * bar has closed with nothing written. **Words, not colour**: the board is not
 * colour-judged.
 */
export function describe(h: HorizonFigures, nowMicros: number): string {
  const how = h.fitted ? h.model : `${h.model}, not fitted`
  const age = forHumans(nowMicros - h.asof_micros)
  const state = h.stale ? 'stale: its next bar has closed and no figure was written' : 'current'
  return `${how} · as of ${stamp(h.asof_micros)} (${age} ago) · ${state}`
}

/** A stored figure's value for (measure, ticker_i[, ticker_j]), or its reason when absent. */
export function figure(h: HorizonFigures | undefined, measure: string, a = '*', b: string | null = null): { value: number | null; absent: string | null } {
  const c = h?.cells.find((x) => x.measure === measure && x.ticker_i === a && (x.ticker_j ?? null) === b)
  if (!c) return { value: null, absent: h ? 'not written' : 'no figure for this horizon' }
  return { value: c.value ?? null, absent: c.absent ?? null }
}

/** β to BTC and the idiosyncratic share, as text; BTC against itself is 1 and —. */
export function betaOf(beta: HorizonFigures | undefined, ticker: string, reference = 'BTC'): [string, string] {
  if (ticker === reference) return ['1', '—']
  const b = figure(beta, 'beta', ticker, reference)
  const i = figure(beta, 'idiosyncratic_share', ticker, reference)
  return [b.value != null ? b.value.toFixed(2) : 'absent', i.value != null ? `${(i.value * 100).toFixed(0)}%` : 'absent']
}

const said = (f: { value: number | null; absent: string | null }, show: (v: number) => string) => (f.value != null ? show(f.value) : `absent (${f.absent})`)

/**
 * The whole universe in one line: absorption against its floor, the last bar's
 * surprise, and the historical turbulence. Every figure as stored.
 */
export function universe(absorption: HorizonFigures | undefined, surprise: HorizonFigures | undefined, turbulence: HorizonFigures | undefined, n: number): string {
  const pct = (v: number) => `${(v * 100).toFixed(0)}th pct`
  const parts = [
    `absorption ${said(figure(absorption, 'correlation_ar'), (v) => v.toFixed(2))} on ρ, ${said(figure(absorption, 'covariance_ar'), (v) => v.toFixed(2))} on Σ (floor 1/${n} = ${(1 / n).toFixed(2)})`,
    `last bar: Mahalanobis ${said(figure(surprise, 'mahalanobis'), (v) => v.toFixed(1))}, ${said(figure(surprise, 'chi2_percentile'), pct)} of χ²${n}, magnitude ${said(figure(surprise, 'magnitude_surprise'), (v) => v.toFixed(2))}, correlation surprise ${said(figure(surprise, 'correlation_surprise'), (v) => v.toFixed(2))}`,
    `turbulence ${said(figure(turbulence, 'turbulence'), (v) => v.toFixed(1))}, ${said(figure(turbulence, 'percentile'), pct)} of its sample`,
  ]
  return parts.join(' · ')
}

/**
 * Whether the horizon's correlation has stayed constant: Engle and Sheppard's
 * test as stored, its window and lags from the row's own params. `null` where
 * nothing was tested (an EWMA horizon writes no `constancy`). Flagged in words
 * only below 0.1%: a test repeated every half hour on overlapping windows
 * alarms far more often than its nominal level (Chu, Stinchcombe and White
 * 1996), about one false episode a month per horizon at 5% against one in
 * years at 0.1%. The p-value is always shown.
 */
export function constancy(h: HorizonFigures | undefined): string | null {
  if (!h) return null
  const p = figure(h, 'engle_sheppard_p')
  let window = ''
  try {
    const params = JSON.parse(h.params) as { days?: number; lags?: number }
    if (params.days != null && params.lags != null) window = ` over ${params.days} days, ${params.lags} lags`
  } catch {
    // The p-value stands without its window.
  }
  if (p.value == null) return `constant correlation: test absent (${p.absent})`
  const verdict = p.value < 0.001 ? 'below 0.1%: a correlation regime flag, the constant correlation is not holding' : 'no regime flag (flagged below 0.1%)'
  const shown = p.value < 0.001 ? p.value.toExponential(1) : p.value.toFixed(3)
  return `constant correlation (Engle–Sheppard${window}): p ${shown}, ${verdict}`
}

export type HistoryPoint = components['schemas']['HistoryPoint']
export type ChartPoint = { time: UTCTimestamp; value: number } | { time: UTCTimestamp }

/** The history as line points: a value where there is one, a whitespace point (a gap) where it is absent. */
export function chartPoints(points: readonly HistoryPoint[]): ChartPoint[] {
  return points.map((p) => {
    const time = Math.floor(p.asof_micros / 1_000_000) as UTCTimestamp
    return p.value != null ? { time, value: p.value } : { time }
  })
}

/** One column of the Instruments panel: which signal and measure, and how it is written. */
export type Column = { signal: 'carry' | 'jumps' | 'liquidity'; measure: string; label: string; show: (v: number) => string }

const pct = (v: number) => `${(v * 100).toFixed(2)}%`
const bps = (v: number) => v.toFixed(2)
const usdK = (v: number) => `$${(v / 1000).toFixed(v < 10_000 ? 2 : 0)}k`

export const INSTRUMENT_COLUMNS: Column[] = [
  { signal: 'carry', measure: 'carry_apr_7d', label: 'carry 7d', show: pct },
  { signal: 'carry', measure: 'excess_apr_7d', label: 'over floor', show: pct },
  { signal: 'carry', measure: 'nowcast_apr', label: 'nowcast', show: pct },
  { signal: 'jumps', measure: 'jumps_up_24h', label: 'jumps ↑ 24h', show: (v) => v.toFixed(0) },
  { signal: 'jumps', measure: 'jumps_down_24h', label: 'jumps ↓ 24h', show: (v) => v.toFixed(0) },
  { signal: 'jumps', measure: 'jump_share_24h', label: 'jump share', show: pct },
  { signal: 'jumps', measure: 'rj_z_24h', label: 'z', show: (v) => v.toFixed(1) },
  { signal: 'liquidity', measure: 'quoted_spread_bps', label: 'quoted bps', show: bps },
  { signal: 'liquidity', measure: 'effective_spread_bps_vw', label: 'effective bps', show: bps },
  { signal: 'liquidity', measure: 'depth_usd_median', label: 'depth median', show: usdK },
  { signal: 'liquidity', measure: 'depth_usd_p10', label: 'depth p10', show: usdK },
  { signal: 'liquidity', measure: 'impact_bps_5s_vw', label: 'impact 5s bps', show: bps },
]

export type Cell = { text: string; title?: string }

/** Rows by instrument: each column's stored figure as text, or "absent" with its reason as the title. */
export function instrumentRows(by: Partial<Record<Column['signal'], HorizonFigures | undefined>>): Array<{ ticker: string; cells: Cell[] }> {
  const tickers = new Set<string>()
  for (const h of Object.values(by)) for (const c of h?.cells ?? []) if (c.ticker_i !== '*') tickers.add(c.ticker_i)
  return [...tickers].sort().map((ticker) => ({
    ticker,
    cells: INSTRUMENT_COLUMNS.map((col) => {
      const h = by[col.signal]
      if (!h) return { text: '—', title: `no ${col.signal} stored` }
      const c = h.cells.find((x) => x.measure === col.measure && x.ticker_i === ticker)
      if (!c) return { text: '—', title: 'not written' }
      return c.value != null ? { text: col.show(c.value) } : { text: 'absent', title: c.absent ?? undefined }
    }),
  }))
}

/** A signal's asof and staleness, in words. */
export function groupNote(signal: string, h: HorizonFigures | undefined, nowMicros: number): string {
  if (!h) return `${signal}: none stored`
  return `${signal}: as of ${stamp(h.asof_micros)} (${forHumans(nowMicros - h.asof_micros)} ago)${h.stale ? ', stale' : ''}`
}
