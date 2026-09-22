import { useState } from 'react'

import { $api } from './contract/client'
import { dec, fmt } from './contract/money'

/** A venue-micros window ending now, so the view shows the latest of whatever is there. */
/** The windows offered, by name. No string is turned into a number anywhere. */
const WINDOWS = {
  hour: 3600,
  day: 24 * 3600,
  week: 7 * 24 * 3600,
} as const

type WindowName = keyof typeof WINDOWS

/** A venue-micros window ending now, so the view shows the latest of whatever is there. */
function windowOf(name: WindowName): { from: number; to: number } {
  const now = Date.now() * 1000
  return { from: now - WINDOWS[name] * 1_000_000, to: now }
}

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
  const [span, setSpan] = useState<WindowName>('hour')
  const window = windowOf(span)
  const { data, error, isPending } = $api.useQuery('get', '/v1/tape/{kind}', {
    params: { path: { kind: 'quotes' }, query: window },
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
  const rows = [...(data?.rows ?? [])].slice(-40).reverse()

  return (
    <section>
      <h2>
        Tape <span className="count">{data ? `${data.rows.length} rows` : '…'}</span>
      </h2>
      <p className="muted">
        quotes, last{' '}
        <select value={span} onChange={(e) => setSpan(e.target.value as WindowName)}>
          <option value="hour">hour</option>
          <option value="day">24 hours</option>
          <option value="week">week</option>
        </select>{' '}
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
