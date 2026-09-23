import { $api } from './contract/client'
import { forHumans } from './Gaps'
import { panelState } from './panel'
import { useRecordAdvances } from './live/status'

/** A day as the partitions spell it, from our clock. */
function day(micros: number): string {
  if (micros <= 0) return '—'
  return new Date(micros / 1000).toISOString().slice(11, 19)
}

/**
 * How much of each day the record holds.
 *
 * **The question nothing else answered**: *is this day usable?* That a day
 * exists, that it wants compacting, and that some time is missing across the
 * whole record are three different facts, and none of them is this one.
 *
 * Every figure is in OUR clock. `recv_micros` is what the partitions are dated
 * by, what a gap's bounds are written in, and what *did we have this data*
 * means — the predecessor puts it in one line: *"recv_micros is what coverage,
 * gaps and latency are measured in."*
 *
 * **Accounted for, never assumed.** The window is the observed rows plus the
 * day's stated gaps; it is not midnight to midnight. A day whose capture began
 * at noon says nothing about its morning, and claiming the morning either way
 * would invent an absence or hide one.
 */
export default function Coverage() {
  const advanced = useRecordAdvances('quotes')
  const read = $api.useQuery('get', '/v1/coverage')
  const state = panelState(read, (d) => d.days.length === 0)

  if (state.kind === 'refused') {
    return (
      <section>
        <h2>Coverage</h2>
        <div className="refusal">
          <strong>Coverage could not be read.</strong>
          <p>{String(state.error)}</p>
        </div>
      </section>
    )
  }

  const data = state.kind === 'reading' ? undefined : state.data
  const days = data?.days ?? []

  return (
    <section>
      <h2>
        Coverage <span className="count">{data ? `${days.length} day-datasets` : '…'}</span>
      </h2>
      <p className="muted">
        how much of what the record accounts for it actually holds — the window is the rows it has
        plus the gaps it states, never midnight to midnight, because a day whose capture began at
        noon says nothing about its morning
        {' · '}
        {advanced === null ? 'the record has not moved since this page loaded' : `advanced ${advanced}s ago`}
      </p>
      {state.kind === 'reading' ? <p className="muted">reading the record…</p> : null}
      {state.kind === 'empty' ? (
        <p className="muted">The record accounts for no day yet.</p>
      ) : null}
      {days.length > 0 ? (
        <table className="tape">
          <thead>
            <tr>
              <th>day</th>
              <th>dataset</th>
              <th>window</th>
              <th className="num">accounted</th>
              <th className="num">missing</th>
              <th className="num">covered</th>
              <th className="num">rows</th>
            </tr>
          </thead>
          <tbody>
            {days.map((d) => (
              <tr key={`${d.venue}/${d.kind}/${d.date}`}>
                <td>
                  <code>{d.date}</code>
                </td>
                <td className="muted">{d.kind}</td>
                {/* Shown, never omitted: one minute observed and one minute
                    covered is 100% of a minute, and a reader must be able to
                    see that the window is not the day. */}
                <td className="muted">
                  {day(d.window_from_micros)}–{day(d.window_to_micros)}
                </td>
                <td className="num">{forHumans(d.window_micros)}</td>
                <td className="num">{forHumans(d.missing_micros)}</td>
                {/* No colour, no threshold. Whether 22.9% is enough belongs to
                    whoever is running the backtest. */}
                <td className="num">
                  {d.window_micros > 0
                    ? `${((100 * d.covered_micros) / d.window_micros).toFixed(1)}%`
                    : '—'}
                </td>
                <td className="num">{d.rows.toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  )
}
