/**
 * The Portfolio view's arithmetic, **modelled** and labelled so.
 *
 * Positions come from the ledger's fold report (held) or from sizes the
 * viewer typed on a flat account (hypothetical); volatility and correlation
 * come from `/v1/statistics`. What is computed here — parametric VaR, risk
 * share — is a model over those, never a record, and it inherits absence: a
 * figure built on a missing σ or ρ is itself missing, naming which.
 *
 * The shapes below mirror datawatch's `ledger::fold::FoldReport` and
 * `derive::tape::Derived`, which the tower passes through unchanged. They are
 * stated here because the contract types them as opaque objects.
 */

/** A statistic as derived: a value with its count and backfill, or why not. */
export type Cell =
  | { value: { value: number; n: number; backfilled_share: number } }
  | { absent: { instrument: string; count: number; floor: number } }

export interface Pair {
  rho: Cell
  interval: { low: number; high: number } | null
}

export interface Statistics {
  bucket_secs: number
  from_micros: number
  to_micros: number
  annualisation: number
  volatility: Record<string, Cell>
  correlation: Record<string, Pair>
  beta: Record<string, Cell>
  reference: string | null
}

export interface Derived {
  venue: string
  bound: number | null
  statistics: Statistics
}

export type Basis = { state: 'flat' } | { state: 'known'; price: string } | { state: 'unknown' }

export interface BookReport {
  dex: string | null
  ticker: string
  position: string | null
  basis: Basis
  realised: string
  realised_not_held: number
  fees: string | null
  funding: string
  poisoned_by: [number, number] | null
}

export interface AccountFold {
  books: BookReport[]
  breaks: unknown[]
  skews: unknown[]
  realised_agreements: number
  snapshot_differences: unknown[]
  snapshot_agreements: number
  cash: Record<string, { usdc: string; unknown: number }>
  equity_not_held: string[]
}

export interface FoldReport {
  venue: string
  at_micros: number
  accounts: Record<string, AccountFold>
}

/** Where a size came from: the ledger, or the viewer. */
export type Source = 'held' | 'hypothetical'

/** One exposure, in USD notional, signed. */
export interface Exposure {
  ticker: string
  usd: number
}

export interface Exposures {
  source: Source
  items: Exposure[]
}

export const value = (c: Cell | undefined): number | null => (c && 'value' in c ? c.value.value : null)

/** A cell's key in the correlation map: the two tickers, sorted. */
export const pairKey = (a: string, b: string): string => (a < b ? `${a}|${b}` : `${b}|${a}`)

/** The result of modelling: figures, or the input that was missing. */
export type Modelled =
  | {
      kind: 'modelled'
      source: Source
      /** One-day portfolio σ in USD. */
      sigmaDay: number
      var95: number
      var99: number
      /** 95 % VaR were every pair to move together (ρ = 1): `1.645·Σ|Nₖ|σₖ`. */
      undiversified95: number
      /** Sum of solo one-day σ over portfolio one-day σ; 1 when ρ is 1 throughout. */
      ratio: number
      /** Each exposure's share of portfolio variance; negative is a hedge. */
      shares: { ticker: string; share: number }[]
    }
  | { kind: 'absent'; source: Source; missing: string }

/**
 * Parametric one-day VaR at zero mean: `σ_day = √(eᵀ Σ e)`, Σ from annual σ
 * over √365 and the correlations, VaR = 1.645 σ and 2.326 σ.
 *
 * **Absent** when any held instrument lacks σ, or any held pair lacks ρ,
 * naming the first missing input. A figure is never computed around a hole.
 */
export function model(exposures: Exposures, stats: Statistics): Modelled {
  const held = exposures.items.filter((e) => e.usd !== 0)
  const source = exposures.source
  const day = Math.sqrt(365)
  const sigma: number[] = []
  for (const e of held) {
    const s = value(stats.volatility[e.ticker])
    if (s === null) return { kind: 'absent', source, missing: `σ of ${e.ticker}` }
    sigma.push(s / day)
  }
  const n = held.length
  const rho = (i: number, j: number): number | null =>
    i === j ? 1 : value(stats.correlation[pairKey(held[i].ticker, held[j].ticker)]?.rho)
  const marginal: number[] = new Array(n).fill(0)
  let variance = 0
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      const r = rho(i, j)
      if (r === null) {
        return { kind: 'absent', source, missing: `ρ of ${pairKey(held[i].ticker, held[j].ticker)}` }
      }
      const c = r * sigma[i] * sigma[j] * held[j].usd
      marginal[i] += c
      variance += held[i].usd * c
    }
  }
  const sigmaDay = Math.sqrt(Math.max(variance, 0))
  const soloSum = held.reduce((a, e, i) => a + Math.abs(e.usd) * sigma[i], 0)
  return {
    kind: 'modelled',
    source,
    sigmaDay,
    var95: 1.645 * sigmaDay,
    var99: 2.326 * sigmaDay,
    undiversified95: 1.645 * soloSum,
    ratio: sigmaDay > 0 ? soloSum / sigmaDay : 1,
    shares: held.map((e, i) => ({
      ticker: e.ticker,
      share: variance > 0 ? (e.usd * marginal[i]) / variance : 0,
    })),
  }
}

/** A shock's result: the book's P&L, or the input that was missing. */
export type Shocked =
  | { kind: 'modelled'; source: Source; pnl: number }
  | { kind: 'absent'; source: Source; missing: string }

/**
 * Move `ticker` by `move` (a fraction, −0.10 for −10 %) and every held leg by
 * its β on it: `β(m,k) = ρ(m,k)·σ(m)/σ(k)`, the expected move of `m` given
 * `k`'s under the same joint normal the VaR assumes, from the same σ and ρ.
 *
 * **Absent** when the shocked leg's σ, or a held leg's σ or ρ with it, is
 * missing, naming which. The shocked leg has β 1 and needs no ρ.
 */
export function shock(exposures: Exposures, stats: Statistics, ticker: string, move: number): Shocked {
  const source = exposures.source
  const held = exposures.items.filter((e) => e.usd !== 0)
  const sk = value(stats.volatility[ticker])
  if (sk === null) return { kind: 'absent', source, missing: `σ of ${ticker}` }
  let pnl = 0
  for (const e of held) {
    if (e.ticker === ticker) {
      pnl += e.usd * move
      continue
    }
    const sm = value(stats.volatility[e.ticker])
    if (sm === null) return { kind: 'absent', source, missing: `σ of ${e.ticker}` }
    const r = value(stats.correlation[pairKey(e.ticker, ticker)]?.rho)
    if (r === null) return { kind: 'absent', source, missing: `ρ of ${pairKey(e.ticker, ticker)}` }
    pnl += e.usd * ((r * sm) / sk) * move
  }
  return { kind: 'modelled', source, pnl }
}

/** The redesign board's five shocks: one leg moves, the others follow by β. */
export const SHOCKS: { ticker: string; move: number }[] = [
  { ticker: 'BTC', move: -0.1 },
  { ticker: 'BTC', move: 0.1 },
  { ticker: 'ETH', move: -0.15 },
  { ticker: 'CL', move: 0.1 },
  { ticker: 'GOLD', move: -0.05 },
]

/** A pair with a figure. */
export interface Named {
  pair: string
  rho: number
  n: number
}

/** A mean of ρ over the pairs that have a figure, and how many those were. */
export interface Averaged {
  mean: number | null
  over: number
  of: number
}

/** The pairs that matter: extremes among figures, and counted averages. */
export interface PairsSummary {
  alike: Named | null
  hedge: Named | null
  block: Averaged
  all: Averaged
}

/** The redesign board's crypto block. */
export const CRYPTO_BLOCK = ['BTC', 'ETH', 'HYPE']

/**
 * **Extremes are among figures only**, so a pair below its floor is never
 * named; **averages omit absent pairs** and say how many they averaged, so a
 * missing ρ is never read as zero.
 */
export function pairs(stats: Statistics): PairsSummary {
  const named: Named[] = []
  for (const [pair, p] of Object.entries(stats.correlation)) {
    if ('value' in p.rho) named.push({ pair, rho: p.rho.value.value, n: p.rho.value.n })
  }
  const average = (keys: string[]): Averaged => {
    const got = named.filter((x) => keys.includes(x.pair))
    return {
      mean: got.length ? got.reduce((a, x) => a + x.rho, 0) / got.length : null,
      over: got.length,
      of: keys.length,
    }
  }
  const blockKeys: string[] = []
  for (let i = 0; i < CRYPTO_BLOCK.length; i++)
    for (let j = i + 1; j < CRYPTO_BLOCK.length; j++) {
      const k = pairKey(CRYPTO_BLOCK[i], CRYPTO_BLOCK[j])
      if (k in stats.correlation) blockKeys.push(k)
    }
  const byRho = [...named].sort((a, b) => a.rho - b.rho)
  return {
    alike: byRho.at(-1) ?? null,
    hedge: byRho[0] ?? null,
    block: average(blockKeys),
    all: average(Object.keys(stats.correlation)),
  }
}
