import { $api } from './contract/client'
import { panelState } from './panel'
import { useRecordAdvances } from './live/status'

/** An hour, in UTC, on the calendar the partitions use. */
function hour(micros: number): string {
  return new Date(micros / 1000).toISOString().replace('T', ' ').slice(0, 13) + ':00'
}

/**
 * How many rows the record holds, by hour.
 *
 * **The failure nothing else here sees.** A socket that stays open and
 * delivers a trickle records no gap, leaves coverage at its full window, and
 * keeps every instrument's last-seen current — *"the TCP connection remains
 * nominally established, nothing arrives."* The partial case is worse than the
 * total one, because the total one is a gap.
 *
 * **No baseline, no threshold, no colour.** The published answer to this is a
 * learned expected count and an alert on deviation; what a normal hour holds
 * for a venue is the operator's knowledge and not this tower's. An hour beside
 * its neighbours is a shape anybody can read — 39,231 next to 1,380 needs no
 * help from a threshold.
 */
export default function Rates() {
  const advanced = useRecordAdvances('quotes')
  const read = $api.useQuery('get', '/v1/rates')
  const state = panelState(read, (d) => d.buckets.length === 0)

  if (state.kind === 'refused') {
    return (
      <section>
        <h2>Rates</h2>
        <div className="refusal">
          <strong>The rates could not be read.</strong>
          <p>{String(state.error)}</p>
        </div>
      </section>
    )
  }

  const data = state.kind === 'reading' ? undefined : state.data
  const buckets = data?.buckets ?? []
  // One row per hour, one column per dataset — the shape is the point, and a
  // dataset that received nothing in an hour is blank rather than zero,
  // because nothing arriving and zero arriving are not the same claim.
  const kinds = [...new Set(buckets.map((b) => b.kind))].sort()
  const hours = [...new Set(buckets.map((b) => b.hour_micros))].sort((a, b) => b - a)
  const at = new Map(buckets.map((b) => [`${b.hour_micros}/${b.kind}`, b.rows]))

  return (
    <section>
      <h2>
        Rates <span className="count">{data ? `${data.hours} hours` : '…'}</span>
      </h2>
      <p className="muted">
        rows per hour — a feed that stays connected and delivers a fraction of its usual volume
        records no gap and leaves coverage whole, so this is the only place it shows. No baseline
        and no threshold: what a normal hour holds is yours to know
        {data?.capped ? ' · capped to the newest hours' : ''}
        {' · '}
        {advanced === null ? 'the record has not moved since this page loaded' : `advanced ${advanced}s ago`}
      </p>
      {state.kind === 'reading' ? <p className="muted">counting…</p> : null}
      {state.kind === 'empty' ? (
        <p className="muted">The record holds no rows yet.</p>
      ) : null}
      {buckets.length > 0 ? (
        <table className="tape">
          <thead>
            <tr>
              <th>hour (UTC)</th>
              {kinds.map((k) => (
                <th key={k} className="num">
                  {k}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {hours.map((h) => (
              <tr key={h}>
                <td>
                  <code>{hour(h)}</code>
                </td>
                {kinds.map((k) => {
                  const rows = at.get(`${h}/${k}`)
                  return (
                    <td key={k} className="num">
                      {rows === undefined ? '' : rows.toLocaleString()}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  )
}
