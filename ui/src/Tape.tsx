import { useState } from 'react'

import { $api } from './contract/client'
import { useRecordAdvances, useRemembered } from './live/status'
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
  // Told when the record moves, and refetched then. Nothing polls: asking the
  // tower whether the tape grew is 53µs and it asks once for everybody, where
  // a refetchInterval here would decode 39,231 rows on a timer to usually
  // learn nothing.
  const advanced = useRecordAdvances('quotes')
  // **Every instrument, or one.** Six tickers were in the tape and the newest
  // forty rows were all the busiest one, so five of the six could not be
  // looked at from here at all. `null` is every, which is what a table of the
  // newest rows wants by default.
  const [ticker, setTicker] = useState<string | null>(null)
  const { data, error, isPending } = $api.useQuery('get', '/v1/tape/{kind}', {
    // Ask for what is drawn. The route caps anyway; asking is what stops
    // eleven megabytes crossing to render forty rows.
    params: {
      path: { kind: 'quotes' },
      // Asked of the ROUTE, not filtered here: filtering in the browser would
      // still ship every instrument's rows to drop most of them, which is
      // what the candle chart did.
      query: { ...WHOLE_TAPE, limit: COUNTS[count], ...(ticker ? { ticker } : {}) },
    },
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
  // **From the route, and remembered.** Deriving this from the returned rows
  // offered exactly one instrument — the newest forty quotes are all the
  // busiest ticker, so the other five stayed unreachable, which is the gap
  // this change is about, reintroduced by its own fix. The route reports what
  // the read matched, before the cap; once a ticker is chosen that is one
  // entry, so what an unfiltered read said is held on to.
  const tickers = useRemembered(data?.tickers)

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
        quotes{' '}
        <select value={ticker ?? ''} onChange={(e) => setTicker(e.target.value || null)}>
          <option value="">every instrument</option>
          {/* Whatever is already selected stays offered, even when this
              window's rows no longer include it — otherwise choosing a quiet
              ticker empties the table and the selector at once, and there is
              no way back to it. */}
          {[...new Set([...tickers, ...(ticker ? [ticker] : [])])].sort().map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        · durable to stream_seq {data?.bound ?? '…'}
        {/* **A stale table and a current one look identical**, which is the
            whole reason this is here. Measured locally against a local
            arrival, never by subtracting the record's clock from ours. */}
        {' · '}
        {advanced === null
          ? 'the record has not moved since this page loaded'
          : `advanced ${advanced}s ago`}
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
