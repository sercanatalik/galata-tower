import { $api } from './contract/client'
import { useRecordAdvances } from './live/status'

/** The whole tape; the summary is bounded by the number of causes, not rows. */
const WHOLE_TAPE = { from: 0, to: 9_000_000_000_000_000 }

/**
 * A duration in whole units, largest that fits.
 *
 * Not a money path: these are microseconds the record measured, so rendering
 * them is arithmetic on an integer count, not a decimal that must not round.
 * The browser-side money guard is textual and refuses the coercions by name
 * anywhere outside `money.ts` — including in a comment that merely mentions
 * one, which it did to an earlier draft of this paragraph. Nothing here needs
 * one either way.
 */
export function forHumans(micros: number): string {
  const seconds = Math.round(micros / 1_000_000)
  // **A sub-second gap is still a gap.** Rounding one to `0s` reads as "no
  // gap", which is the single thing this panel must never say when the record
  // says otherwise — and it is what this returned until a test was written
  // whose name contradicted its own assertion.
  if (micros > 0 && seconds === 0) return '<1s'
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`
  if (seconds < 86_400) return `${(seconds / 3600).toFixed(1)}h`
  return `${(seconds / 86_400).toFixed(1)}d`
}

/**
 * What the record says is missing, and why.
 *
 * **A gap is never inferred from silence.** Every interval here was written
 * down by the capture with a cause; absent rows are not a gap and nothing in
 * this panel turns them into one.
 *
 * The duration is the UNION of a cause's intervals, folded in the tower. The
 * tape writes one row per affected instrument and series, so one outage
 * arrives two dozen times — summing those rows reported 32.6 days missing from
 * a 32.6-hour window on this very archive. The row count is shown beside the
 * duration as BREADTH, which is what it actually measures.
 */
export default function Gaps() {
  const advanced = useRecordAdvances('gaps')
  const { data, error, isPending } = $api.useQuery('get', '/v1/gaps', {
    params: { query: WHOLE_TAPE },
  })

  if (error) {
    return (
      <section>
        <h2>Gaps</h2>
        <div className="refusal">
          <strong>The gaps could not be read.</strong>
          <p>{String(error)}</p>
        </div>
      </section>
    )
  }

  const causes = data?.causes ?? []

  return (
    <section>
      <h2>
        Gaps <span className="count">{data ? `${data.rows} rows` : '…'}</span>
      </h2>
      <p className="muted">
        what the record says is missing, and why — never inferred from absent rows
        {' · '}
        {advanced === null
          ? 'the record has not moved since this page loaded'
          : `advanced ${advanced}s ago`}
      </p>
      {isPending ? <p className="muted">reading the gaps…</p> : null}
      {data && causes.length === 0 ? (
        <p className="muted">Nothing is recorded missing in this window.</p>
      ) : null}
      <table className="tape">
        <thead>
          <tr>
            <th>cause</th>
            <th className="num">missing</th>
            <th className="num">intervals</th>
            <th className="num">rows</th>
            <th>series</th>
            <th className="num">tickers</th>
            <th>bounded by</th>
          </tr>
        </thead>
        <tbody>
          {causes.map((c) => (
            <tr key={c.cause}>
              <td>
                <code>{c.cause}</code>
              </td>
              <td className="num">{forHumans(c.missing_micros)}</td>
              <td className="num">{c.intervals}</td>
              {/* Breadth, not a second duration: how many instrument-series
                  the outage touched. */}
              <td className="num">{c.rows}</td>
              <td>{c.series.join(', ')}</td>
              <td className="num">{c.tickers}</td>
              {/* **How loose the bound is**, carried through and uncoloured.
                  A gap bounded by two observed sequence numbers is a tighter
                  statement than one bounded by a restart, and the schema keeps
                  this field so a consumer can tell. */}
              <td>
                {Object.entries(c.clipped)
                  .map(([how, n]) => `${how} ×${n}`)
                  .join(', ')}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  )
}
