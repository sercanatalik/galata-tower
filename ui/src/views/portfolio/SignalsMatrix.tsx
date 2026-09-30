import { useState } from 'react'

import { useSignalHistory } from '../../data/queries'
import {
  annualisedSigma,
  backtestOf,
  betaOf,
  cellOf,
  chartPoints,
  describe,
  tailOf,
  type HorizonFigures,
} from '../../data/signals'
import SignalChart from '../../charts/SignalChart'

export function SignalMatrix({ h, beta, tail, bt }: { h: HorizonFigures; beta?: HorizonFigures; tail?: HorizonFigures; bt?: HorizonFigures }) {
  const now = Date.now() * 1000
  return (
    <div>
      <p className="muted mono" style={{ fontSize: 12, margin: '0 0 8px' }}>{describe(h, now)}</p>
      <table className="grid mono">
        <thead>
          <tr>
            <th scope="col" />
            {h.tickers.map((t) => (
              <th key={t} scope="col">{t}</th>
            ))}
            <th scope="col">σ a year</th>
            <th scope="col">β to BTC</th>
            <th scope="col">not BTC</th>
            <th scope="col" title="one bar ahead, filtered historical simulation">VaR 99% bar</th>
            <th scope="col" title="one bar ahead, filtered historical simulation">ES 97.5% bar</th>
            <th scope="col" title="the last 90 days of stored tails against the bars they forecast">tail backtest 97.5%</th>
          </tr>
        </thead>
        <tbody>
          {h.tickers.map((a) => {
            const own = cellOf(h, 'covariance', a, a)
            return (
              <tr key={a}>
                <th scope="row">{a}</th>
                {h.tickers.map((b) => {
                  if (a === b) return <td key={b} className="muted">1</td>
                  const c = cellOf(h, 'correlation', a, b)
                  return (
                    <td key={b} title={c?.absent ?? (c?.n_eff != null ? `n_eff ${c.n_eff.toFixed(0)}` : undefined)}>
                      {c?.value != null ? c.value.toFixed(2) : <span className="muted">absent</span>}
                    </td>
                  )
                })}
                <td title={own?.absent ?? undefined}>
                  {own?.value != null && h.width_micros != null ? `${(annualisedSigma(own.value, h.width_micros) * 100).toFixed(1)}%` : <span className="muted">absent</span>}
                </td>
                {betaOf(beta, a).map((text, k) => (
                  <td key={k}>{text}</td>
                ))}
                {tailOf(tail, a).map((text, k) => (
                  <td key={`t${k}`} className={text === 'absent' ? 'muted' : undefined}>{text}</td>
                ))}
                <td className={backtestOf(bt, a) === 'absent' ? 'muted' : undefined}>{backtestOf(bt, a)}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

/** What the history chart can show: a pair's ρ or covariance, or an instrument's β to BTC. */
const CHARTABLE: Array<{ label: string; signal: string; measure: string; pair: boolean }> = [
  { label: 'ρ', signal: 'varcov', measure: 'correlation', pair: true },
  { label: 'covariance', signal: 'varcov', measure: 'covariance', pair: true },
  { label: 'β to BTC', signal: 'beta', measure: 'beta', pair: false },
]

export function SignalHistory({ h }: { h: HorizonFigures }) {
  const [which, setWhich] = useState(0)
  const [a, setA] = useState('BTC')
  const [b, setB] = useState('ETH')
  const pick = CHARTABLE[which]
  const tickers = h.tickers
  const history = useSignalHistory(pick.signal, h.horizon, pick.measure, pick.pair ? a : b, pick.pair ? b : 'BTC')
  const points = history.data ? chartPoints(history.data.points) : []
  const absent = history.data?.points.filter((p) => p.value == null).length ?? 0
  return (
    <div>
      <p className="mono" style={{ fontSize: 12, margin: '0 0 8px', display: 'flex', gap: 8, alignItems: 'center' }}>
        <select aria-label="measure" value={which} onChange={(e) => setWhich(e.currentTarget.selectedIndex)}>
          {CHARTABLE.map((c, i) => (
            <option key={c.label} value={i}>{c.label}</option>
          ))}
        </select>{' '}
        {pick.pair ? (
          <select aria-label="first" value={a} onChange={(e) => setA(e.currentTarget.value)}>
            {tickers.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        ) : null}{' '}
        <select aria-label="second" value={b} onChange={(e) => setB(e.currentTarget.value)}>
          {tickers.filter((t) => pick.pair || t !== 'BTC').map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>{' '}
        <span className="muted">
          {h.horizon} · {history.data ? `${history.data.points.length} asofs over ${history.data.days} days${absent ? `, ${absent} absent (gaps)` : ''}` : 'reading'}
        </span>
      </p>
      <SignalChart points={points} height={220} />
    </div>
  )
}
