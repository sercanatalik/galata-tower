import { heardAgo, useLiveStatus, venueLag } from './live/status'

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

  // Keyed by venue, ticker AND series: one instrument has several series and
  // each can go quiet on its own, so collapsing them would hide a dead
  // `trades` behind a live `quotes`.
  const rows: Array<{
    key: string
    venue: string
    pair: Pair
    observed: number | null
    received_ms: number
  }> = []
  for (const v of live.venues.values()) {
    const body = v.body as { observed_at_micros?: number; pairs?: Pair[] } | null
    for (const pair of body?.pairs ?? []) {
      rows.push({
        key: `${v.venue}/${pair.ticker}/${pair.series}`,
        venue: v.venue,
        pair,
        observed: body?.observed_at_micros ?? null,
        received_ms: v.received_ms,
      })
    }
  }
  rows.sort((a, b) => a.key.localeCompare(b.key))

  return (
    <section>
      <h2>
        Instruments <span className="count">{rows.length}</span>
      </h2>
      {rows.length === 0 ? (
        <p className="muted">
          No venue has reported a pair yet. A capture publishes them with its status; the record
          above does not depend on it.
        </p>
      ) : (
        <p className="muted">
          heard — how long the capture had gone without hearing anything, by its own clock · behind
          — how far the venue&rsquo;s timestamp sits behind our receipt, negative if it claims the
          future
        </p>
      )}
      <table className="tape">
        <thead>
          <tr>
            <th>venue</th>
            <th>instrument</th>
            <th>series</th>
            <th>state</th>
            <th className="num">heard</th>
            <th className="num">behind</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ key, venue, pair, observed, received_ms }) => {
            const heard = heardAgo(observed, pair.last_recv_micros, received_ms)
            const lag = venueLag(pair.last_recv_micros, pair.last_event_micros)
            return (
              <tr key={key}>
                <td>
                  <code>{venue}</code>
                </td>
                <td>
                  <code>{pair.ticker ?? '—'}</code>
                </td>
                <td className="muted">{pair.series ?? '—'}</td>
                <td className="muted">{pair.state ?? '—'}</td>
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
