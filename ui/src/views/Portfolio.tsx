import { useState } from 'react'

import { dec, fmt, plot } from '../contract/money'
import { PORTFOLIO_STATISTICS, useLatest, usePortfolio, useStatistics } from '../data/queries'
import {
  type Cell,
  type Derived,
  type Exposures,
  type FoldReport,
  model,
  pairKey,
  type Statistics,
  value,
} from '../data/risk'
import { forHumans, stamp } from '../kit/format'
import { Head, Panel } from '../kit/Panel'
import { shown } from './Overview'

const VENUE = 'hyperliquid'
const HYPOTHETICAL_KEY = 'galata.portfolio.hypothetical'

/** Sizes the viewer typed, in this browser only. Never sent anywhere. */
function useHypothetical(): [Record<string, string>, (t: string, v: string) => void] {
  const [sizes, setSizes] = useState<Record<string, string>>(() => {
    try {
      return JSON.parse(window.localStorage.getItem(HYPOTHETICAL_KEY) ?? '{}')
    } catch {
      return {}
    }
  })
  const set = (ticker: string, usd: string) => {
    const next = { ...sizes, [ticker]: usd }
    setSizes(next)
    try {
      window.localStorage.setItem(HYPOTHETICAL_KEY, JSON.stringify(next))
    } catch {
      // A browser that keeps nothing still shows the figures for this visit.
    }
  }
  return [sizes, set]
}

function CellText({ cell, dp = 2 }: { cell: Cell | undefined; dp?: number }) {
  if (!cell) return <span className="muted">—</span>
  if ('absent' in cell) {
    const a = cell.absent
    return (
      <span className="muted" title={`${a.instrument} has ${a.count} returns, below the floor of ${a.floor}`}>
        — <small>n {a.count}</small>
      </span>
    )
  }
  return (
    <span title={`n ${cell.value.n} · backfilled ${(cell.value.backfilled_share * 100).toFixed(0)}%`}>
      {cell.value.value.toFixed(dp)} <small className="muted">n {cell.value.n}</small>
    </span>
  )
}

function Matrix({ stats }: { stats: Statistics }) {
  const tickers = Object.keys(stats.volatility)
  return (
    <table className="grid mono">
      <thead>
        <tr>
          <th scope="col" />
          {tickers.map((t) => (
            <th key={t} scope="col">{t}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {tickers.map((a) => (
          <tr key={a}>
            <th scope="row">{a}</th>
            {tickers.map((b) => {
              if (a === b) return <td key={b} className="muted">1</td>
              const pair = stats.correlation[pairKey(a, b)]
              return (
                <td
                  key={b}
                  title={pair?.interval ? `interval [${pair.interval.low.toFixed(2)}, ${pair.interval.high.toFixed(2)}]` : undefined}
                >
                  <CellText cell={pair?.rho} />
                </td>
              )
            })}
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function Risk({ exposures, stats }: { exposures: Exposures; stats: Statistics }) {
  const m = model(exposures, stats)
  const label = `modelled · ${exposures.source === 'held' ? 'held positions' : 'hypothetical sizes'}`
  if (exposures.items.every((e) => e.usd === 0)) {
    return <p className="muted">No size to model. {exposures.source === 'hypothetical' ? 'Enter a notional above.' : ''}</p>
  }
  if (m.kind === 'absent') {
    return (
      <p>
        One-day VaR is not computed: <strong>{m.missing}</strong> is below its floor. <span className="muted">{label}</span>
      </p>
    )
  }
  return (
    <div>
      <p className="muted">{label} · parametric, zero mean, one day</p>
      <dl className="facts mono">
        <dt>σ, one day</dt>
        <dd>${m.sigmaDay.toFixed(0)}</dd>
        <dt>VaR 95%</dt>
        <dd>${m.var95.toFixed(0)}</dd>
        <dt>VaR 99%</dt>
        <dd>${m.var99.toFixed(0)}</dd>
      </dl>
      <table className="grid mono">
        <thead>
          <tr>
            <th scope="col">instrument</th>
            <th scope="col">share of risk</th>
          </tr>
        </thead>
        <tbody>
          {m.shares.map((s) => (
            <tr key={s.ticker}>
              <td>{s.ticker}</td>
              <td>{(s.share * 100).toFixed(1)}%{s.share < 0 ? ' · a hedge' : ''}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export default function Portfolio() {
  const portfolio = usePortfolio(VENUE)
  const statistics = useStatistics(VENUE)
  const latest = useLatest()
  const [sizes, setSize] = useHypothetical()
  const prices = latest.data?.venues.find((v) => v.venue === VENUE)?.prices ?? []
  const mark = (ticker: string) => {
    const p = prices.find((x) => x.ticker === ticker)
    return p ? shown(p) : null
  }
  const { horizon, hours, min_observations, z, reference } = PORTFOLIO_STATISTICS

  return (
    <div className="page">
      <section aria-labelledby="pos">
        <Head title="Positions" id="pos">
          {VENUE} · from the ledger's fold, marked at the archive's latest price
        </Head>
        <Panel
          what="The fold report"
          read={portfolio}
          isEmpty={(d) => d.report == null}
          empty={portfolio.data?.reason ?? 'No fold report yet.'}
        >
          {(d) => {
            const report = d.report as unknown as FoldReport
            return (
              <div>
                <p className="muted mono">
                  folded {d.written_micros ? forHumans(Date.now() * 1000 - d.written_micros) : '?'} ago · {stamp(report.at_micros)}
                </p>
                {Object.entries(report.accounts).map(([alias, account]) => {
                  const open = account.books.filter((b) => b.position && !dec(b.position)?.isZero())
                  return (
                    <div key={alias}>
                      <h3>{alias}</h3>
                      {open.length === 0 ? (
                        <p>No positions held. {account.equity_not_held.length ? <span className="muted">Equity not held: {account.equity_not_held.join('; ')}.</span> : null}</p>
                      ) : (
                        <table className="grid mono">
                          <thead>
                            <tr>
                              <th scope="col">instrument</th>
                              <th scope="col">position</th>
                              <th scope="col">basis</th>
                              <th scope="col">mark</th>
                              <th scope="col">notional</th>
                            </tr>
                          </thead>
                          <tbody>
                            {open.map((b) => {
                              const m = mark(b.ticker)
                              const size = dec(b.position)
                              return (
                                <tr key={`${b.dex}:${b.ticker}`}>
                                  <td>{b.dex ? `${b.dex}:` : ''}{b.ticker}</td>
                                  <td>{fmt(size, 4)}</td>
                                  <td>{b.basis.state === 'known' ? fmt(dec(b.basis.price)) : b.basis.state === 'unknown' ? 'unknown' : '—'}</td>
                                  <td>{m ? fmt(m) : 'no price'}</td>
                                  <td>{m && size ? fmt(size.times(m), 0) : '—'}</td>
                                </tr>
                              )
                            })}
                          </tbody>
                        </table>
                      )}
                      <p className="muted mono">
                        checks: {account.breaks.length} breaks · {account.skews.length} skews ({account.realised_agreements} agree) ·{' '}
                        {account.snapshot_differences.length} snapshot differences ({account.snapshot_agreements} agree)
                      </p>
                    </div>
                  )
                })}
              </div>
            )
          }}
        </Panel>
      </section>

      <section aria-labelledby="corr">
        <Head title="Correlation" id="corr">
          ρ of {horizon} log returns over the last {hours} h · floor {min_observations} · interval ±{z} se · hover a cell for n, backfill and interval
        </Head>
        <Panel
          what="The statistics"
          read={statistics}
          isEmpty={(d) => Object.keys((d.derived as unknown as Derived).statistics.volatility).length === 0}
          empty="The tape holds no candles in this window."
        >
          {(d) => <Matrix stats={(d.derived as unknown as Derived).statistics} />}
        </Panel>
      </section>

      <section aria-labelledby="vol">
        <Head title="Volatility and beta" id="vol">
          annualised by √365 per day of a market that never closes · β on {reference}
        </Head>
        <Panel
          what="The statistics"
          read={statistics}
          isEmpty={(d) => Object.keys((d.derived as unknown as Derived).statistics.volatility).length === 0}
          empty="The tape holds no candles in this window."
        >
          {(d) => {
            const s = (d.derived as unknown as Derived).statistics
            return (
              <table className="grid mono">
                <thead>
                  <tr>
                    <th scope="col">instrument</th>
                    <th scope="col">σ, annual</th>
                    <th scope="col">β on {reference}</th>
                  </tr>
                </thead>
                <tbody>
                  {Object.keys(s.volatility).map((t) => (
                    <tr key={t}>
                      <td>{t}</td>
                      <td><CellText cell={s.volatility[t]} /></td>
                      <td>{t === reference ? '1' : <CellText cell={s.beta[t]} />}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          }}
        </Panel>
      </section>

      <section aria-labelledby="risk">
        <Head title="Risk" id="risk">
          modelled from the positions and the statistics above; nothing here is a record
        </Head>
        <Panel
          what="The statistics"
          read={statistics}
          isEmpty={() => false}
          empty={null}
        >
          {(d) => {
            const s = (d.derived as unknown as Derived).statistics
            const report = portfolio.data?.report as unknown as FoldReport | null | undefined
            const held = Object.values(report?.accounts ?? {})
              .flatMap((a) => a.books)
              .filter((b) => b.position && !dec(b.position)?.isZero())
            const exposures: Exposures =
              held.length > 0
                ? {
                    source: 'held',
                    items: held.map((b) => {
                      const m = mark(b.ticker)
                      const size = dec(b.position)
                      return { ticker: b.ticker, usd: m && size ? (plot(size.times(m)) ?? 0) : 0 }
                    }),
                  }
                : {
                    source: 'hypothetical',
                    items: Object.keys(s.volatility).map((t) => ({ ticker: t, usd: plot(dec(sizes[t] || '0')) ?? 0 })),
                  }
            return (
              <div>
                {exposures.source === 'hypothetical' ? (
                  <fieldset>
                    <legend>Hypothetical notional, USD, signed — no positions are held; kept in this browser only</legend>
                    {Object.keys(s.volatility).map((t) => (
                      <label key={t} className="mono">
                        {t}{' '}
                        <input
                          inputMode="decimal"
                          value={sizes[t] ?? ''}
                          placeholder="0"
                          onChange={(e) => setSize(t, e.target.value)}
                        />
                      </label>
                    ))}
                  </fieldset>
                ) : null}
                <Risk exposures={exposures} stats={s} />
                <p className="muted">
                  {value(s.volatility[reference]) === null ? `σ of ${reference} is absent over this window.` : null}
                </p>
              </div>
            )
          }}
        </Panel>
      </section>
    </div>
  )
}
