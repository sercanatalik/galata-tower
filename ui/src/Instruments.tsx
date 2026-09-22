import { $api } from './contract/client'
import { heardAgo, useLiveStatus, useRecordAdvances, venueLag } from './live/status'

/**
 * A venue timestamp as a readable instant, in UTC.
 *
 * **Not an age.** `at_micros` is the VENUE's clock; subtracting it from ours
 * would report the two clocks' disagreement rather than elapsed time, which is
 * the mistake `heardAgo` exists to avoid and which this panel made before.
 * Shown as the instant the record holds, on the calendar the partitions use.
 */
function stamp(micros: number): string {
  if (micros <= 0) return '—'
  return new Date(micros / 1000).toISOString().replace('T', ' ').slice(0, 19)
}

/** What a capture reports about one instrument's series. */
type Pair = {
  ticker?: string
  series?: string
  state?: string
  last_recv_micros?: number | null
  last_event_micros?: number | null
}

/**
 * Every instrument the venues report, and two ages for each.
 *
 * **Two, because one cannot produce the number that matters.** `PairStatus`
 * says so in its own documentation, and the published guidance agrees: with one
 * timestamp you cannot tell whether the producer is slow or the venue is. With
 * two, the failure shows — a venue claiming data from the future gives a
 * negative lag, and a venue falling behind its peers spikes asymmetrically.
 *
 * **Nothing here is judged.** No colour, no threshold. What counts as too old
 * belongs to the operator; this reports.
 */
export default function Instruments() {
  const live = useLiveStatus()
  // **The record leads.** Until 2026-09-22 this panel read `live.venues` and
  // nothing else, so the one surface naming the instruments was the only one
  // that could not answer without a broker — in a binary whose whole sentence
  // is that it watches the record, not the worker. The roadmap asks this tier
  // for "six instruments, their ages"; against a tape holding all six it
  // showed zero.
  useRecordAdvances('quotes')
  const { data } = $api.useQuery('get', '/v1/instruments')
  const held = data?.instruments ?? []

  // A venue's live account of one instrument, where one has arrived. Keyed by
  // venue and ticker: the record calls the dataset a `kind` and the status
  // calls it a `series`, and where those spellings agree the pair is matched
  // on all three. A translation table between them would be a third statement
  // of a naming both sides already make.
  const livePairs = new Map<string, { pair: Pair; observed: number | null; received_ms: number }>()
  for (const v of live.venues.values()) {
    const body = v.body as { observed_at_micros?: number; pairs?: Pair[] } | null
    for (const pair of body?.pairs ?? []) {
      livePairs.set(`${v.venue}/${pair.ticker}/${pair.series}`, {
        pair,
        observed: body?.observed_at_micros ?? null,
        received_ms: v.received_ms,
      })
    }
  }

  // **Rows are the record's**, one per (venue, ticker, kind) it holds. Each
  // can go quiet on its own, so they are not collapsed: that would hide a dead
  // `trades` behind a live `quotes`.
  const rows = held.map((i) => ({
    key: `${i.venue}/${i.ticker}/${i.kind}`,
    ...i,
    live: livePairs.get(`${i.venue}/${i.ticker}/${i.kind}`),
  }))

  return (
    <section>
      <h2>
        Instruments <span className="count">{rows.length}</span>
      </h2>
      {rows.length === 0 ? (
        <p className="muted">
          The record holds no instrument. This reads the tape, so it does not
          need a broker — an empty table here means an empty or unreadable tape
          root, not a capture that is not running.
        </p>
      ) : (
        <p className="muted">
          <strong>last</strong> and <strong>rows</strong> are the record&rsquo;s, and need no
          broker · <strong>heard</strong> and <strong>behind</strong> are the venue&rsquo;s own
          account of itself and appear only while one is publishing — heard is how long a capture
          had gone without hearing anything by its own clock, behind is how far the venue&rsquo;s
          timestamp sits from our receipt, negative if it claims the future
        </p>
      )}
      <table className="tape">
        <thead>
          <tr>
            <th>venue</th>
            <th>instrument</th>
            <th>dataset</th>
            <th className="num">last</th>
            <th className="num">rows</th>
            <th>state</th>
            <th className="num">heard</th>
            <th className="num">behind</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const heard = row.live
              ? heardAgo(row.live.observed, row.live.pair.last_recv_micros, row.live.received_ms)
              : null
            const lag = row.live
              ? venueLag(row.live.pair.last_recv_micros, row.live.pair.last_event_micros)
              : null
            return (
              <tr key={row.key}>
                <td>
                  <code>{row.venue}</code>
                </td>
                <td>
                  <code>{row.ticker}</code>
                </td>
                <td className="muted">{row.kind}</td>
                {/* The record's own clock, rendered as a date rather than an
                    age: this is venue time, and subtracting it from ours
                    would report the two clocks' disagreement instead of
                    elapsed time — the mistake `heardAgo` exists to avoid. */}
                <td className="num">{stamp(row.last_micros)}</td>
                <td className="num">{row.rows.toLocaleString()}</td>
                <td className="muted">{row.live?.pair.state ?? '—'}</td>
                <td className="num">{heard === null ? '—' : `${heard}s`}</td>
                <td className="num">{lag === null ? '—' : `${lag}s`}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </section>
  )
}
