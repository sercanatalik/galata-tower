import { type Cell, type Statistics, pairKey, pairs } from '../../data/risk'

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

/** The matrix's extremes and averages, among pairs with a figure. */
function PairsThatMatter({ stats }: { stats: Statistics }) {
  const p = pairs(stats)
  const avg = (a: { mean: number | null; over: number; of: number }) =>
    a.mean === null ? '— none has a figure' : `avg ${a.mean.toFixed(2)} of ${a.over}${a.over < a.of ? ` (${a.of - a.over} below floor)` : ''}`
  return (
    <dl className="facts mono">
      <dt>most alike</dt>
      <dd>{p.alike ? `${p.alike.pair.replace('|', ' / ')} · ${p.alike.rho.toFixed(2)}, n ${p.alike.n}` : '—'}</dd>
      <dt>strongest hedge</dt>
      <dd>{p.hedge ? `${p.hedge.pair.replace('|', ' / ')} · ${p.hedge.rho.toFixed(2)}, n ${p.hedge.n}` : '—'}</dd>
      <dt>crypto block · BTC ETH HYPE</dt>
      <dd>{avg(p.block)}</dd>
      <dt>all pairs</dt>
      <dd>{avg(p.all)}</dd>
    </dl>
  )
}

export function CorrelationMatrix({ stats }: { stats: Statistics }) {
  const tickers = Object.keys(stats.volatility)
  return (
    <div>
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
      <PairsThatMatter stats={stats} />
    </div>
  )
}

export { CellText }
