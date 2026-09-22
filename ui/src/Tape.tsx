import { useState } from 'react'

import { $api } from './contract/client'
import { dec, fmt } from './contract/money'

/**
 * How many of the newest rows to show. A COUNT, not a period.
 *
 * This offered `hour`, `day` and `week` ending at the wall clock, and against a
 * tape thirty-three hours old every one of them was empty. The table shows the
 * latest rows, and *latest N is not a time window* — a count answers "which
 * rows are newest", which is the question a table of the latest rows asks. The
 * route returns the newest when the cap bites, so the count IS the cap.
 *
 * A window remains the right control for a question about a period; this is not
 * one, and `/v1/tape` still takes a window for callers that are.
 */
const COUNTS = { '40': 40, '200': 200, '1000': 1000 } as const

type CountName = keyof typeof COUNTS

/** The whole tape; the cap selects the newest. */
const WHOLE_TAPE = { from: 0, to: 9_000_000_000_000_000 }

/**
 * The tape, read through `money.ts`.
 *
 * **Every price arrives as a string** and is parsed here with `decimal.js`.
 * The tower serialises decimals quoted for exactly this: sent as JSON numbers
 * they would be doubles before this component could decline to round them, and
 * the browser-side money guard could not see it, because nothing here would
 * have converted anything. (That guard is textual and refused this very
 * comment for naming the call it forbids, which is the rule working.)
 */
export default function Tape() {
  // **The narrowest window by default, and this was measured rather than
  // assumed.** A day of this tape is 39,231 rows and 11 MiB to render forty of
  // them. An hour bounds it in a live deployment; what would bound it properly
  // is a row limit on the read, which is a change to the route rather than to
  // this component and is named rather than smuggled in here.
  const [count, setCount] = useState<CountName>('40')
  const { data, error, isPending } = $api.useQuery('get', '/v1/tape/{kind}', {
    // Ask for what is drawn. The route caps anyway; asking is what stops
    // eleven megabytes crossing to render forty rows.
    params: { path: { kind: 'quotes' }, query: { ...WHOLE_TAPE, limit: COUNTS[count] } },
  })

  if (error) {
    return (
      <section>
        <h2>Tape</h2>
        <div className="refusal">
          <strong>The tape could not be read.</strong>
          <p>{String(error)}</p>
          <p className="muted">
            A tape is built from the archive by <code>galata-tape-rebuild</code>; a tower pointed at
            a root that has none says so rather than showing an empty table.
          </p>
        </div>
      </section>
    )
  }

  // Newest first, and bounded for the eye: the window bounds the read, this
  // bounds the render.
  // The route returns the newest already; this only reverses them for the eye.
  const rows = [...(data?.rows ?? [])].reverse()

  return (
    <section>
      <h2>
        Tape <span className="count">{data ? `${data.rows.length} rows` : '…'}</span>
      </h2>
      <p className="muted">
        the newest{' '}
        <select value={count} onChange={(e) => setCount(e.target.value as CountName)}>
          <option value="40">40</option>
          <option value="200">200</option>
          <option value="1000">1000</option>
        </select>{' '}
        quotes
        · durable to stream_seq {data?.bound ?? '…'}
      </p>
      {isPending ? <p className="muted">reading the tape…</p> : null}
      {data && data.rows.length === 0 ? (
        <p className="muted">Nothing in that window.</p>
      ) : null}
      <table className="tape">
        <thead>
          <tr>
            <th>ticker</th>
            <th className="num">bid</th>
            <th className="num">ask</th>
            <th className="num">bid size</th>
            <th className="num">ask size</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => {
            const r = row as Record<string, unknown>
            return (
              <tr key={`${r.stream_seq}-${i}`}>
                <td>
                  <code>{String(r.ticker ?? '')}</code>
                </td>
                <td className="num">{fmt(dec(r.bid_px as string | null))}</td>
                <td className="num">{fmt(dec(r.ask_px as string | null))}</td>
                <td className="num">{fmt(dec(r.bid_sz as string | null), 4)}</td>
                <td className="num">{fmt(dec(r.ask_sz as string | null), 4)}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </section>
  )
}
