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
