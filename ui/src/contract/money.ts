// Money never touches a float. Every decimal the contract carries arrives
// as a string and is parsed here, once, into a Decimal — and the guard
// (`scripts/check-no-float-money.sh`) refuses `parseFloat(` and `Number(`
// anywhere else under src/, so the one place a float can be made is the
// one place that says why.

import Decimal from 'decimal.js'

Decimal.set({ precision: 40, toExpNeg: -20, toExpPos: 40 })

export type Money = Decimal

/** Parse a contract decimal. `null` stays `null`: absence is a statement. */
export function dec(s: string | null | undefined): Money | null {
  if (s === null || s === undefined || s === '') return null
  return new Decimal(s)
}

/** A decimal that must be present. */
export function must(s: string | null | undefined, what: string): Money {
  const d = dec(s)
  if (d === null) throw new Error(`${what}: absent where a figure was required`)
  return d
}

/** Render for the eye: grouped thousands, a bounded number of decimals. */
export function fmt(m: Money | null | undefined, dp = 2): string {
  if (m === null || m === undefined) return '—'
  const fixed = m.toFixed(dp)
  const [whole, frac] = fixed.split('.')
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return frac ? `${grouped}.${frac}` : grouped
}

/** Signed, with the sign shown: +12.40 / −3.10 / 0.00. */
export function signed(m: Money | null | undefined, dp = 2): string {
  if (m === null || m === undefined) return '—'
  if (m.isZero()) return fmt(m, dp)
  return (m.isNegative() ? '−' : '+') + fmt(m.abs(), dp)
}

/** A fraction as a percentage. */
export function pct(m: Money | null | undefined, dp = 1): string {
  if (m === null || m === undefined) return '—'
  return `${m.mul(100).toFixed(dp)}%`
}

/**
 * The one float. A chart plots pixels, not money: lightweight-charts takes
 * a JS number per point, and the conversion happens here — display only,
 * never arithmetic, never written back.
 */
export function plot(m: Money | null | undefined): number | null {
  if (m === null || m === undefined) return null
  // eslint-disable-next-line no-restricted-syntax
  return Number(m.toString())
}
