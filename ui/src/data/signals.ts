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

export type HistoryPoint = components['schemas']['HistoryPoint']
export type ChartPoint = { time: UTCTimestamp; value: number } | { time: UTCTimestamp }

/** The history as line points: a value where there is one, a whitespace point (a gap) where it is absent. */
export function chartPoints(points: readonly HistoryPoint[]): ChartPoint[] {
  return points.map((p) => {
    const time = Math.floor(p.asof_micros / 1_000_000) as UTCTimestamp
    return p.value != null ? { time, value: p.value } : { time }
  })
}
