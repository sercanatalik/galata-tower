import { $api } from './contract/client'
import { heardAgo, silenceReason, sinceArrival, useLiveStatus, whySilent } from './live/status'
import Boundary from './Boundary'
import Candles from './Candles'
import Coverage from './Coverage'
import Failures from './Failures'
import Gaps from './Gaps'
import Rates from './Rates'
import Instruments from './Instruments'
import Tape from './Tape'

/** A refusal, rendered. A blank page is the worst answer to a server that is not there. */
function Refusal({ what, error }: { what: string; error: unknown }) {
  const said = error instanceof Error ? error.message : String(error)
  return (
    <div className="refusal">
      <strong>{what} could not be read.</strong>
      <p>{said}</p>
      <p className="muted">
        The tower reads a directory. If nothing is there, nothing is claimed — this is the server
        saying so, not an empty screen pretending otherwise.
      </p>
    </div>
  )
}

/** Which archive is being watched, and what a reader may prune on. */
function Header() {
  const { data, error } = $api.useQuery('get', '/v1/about')
  if (error) return <Refusal what="The archive" error={error} />
  return (
    <header>
      <h1>galata-tower</h1>
      <p className="muted">
        watching <code>{data?.archive ?? '…'}</code>
      </p>
      <p className="muted">
        prunes on {data?.prune_on.map((c) => <code key={c}>{c}</code>) ?? null}
      </p>
    </header>
  )
}

/** The partitions the record holds. A fact on disk, not a claim by a process. */
function Partitions() {
  const { data, error, isPending } = $api.useQuery('get', '/v1/partitions')
  if (error) return <Refusal what="The partitions" error={error} />
  return (
    <section>
      <h2>Partitions {data ? <span className="count">{data.length}</span> : null}</h2>
      {isPending ? <p className="muted">reading the store…</p> : null}
      {data?.length === 0 ? <p className="muted">The archive holds none.</p> : null}
      <ul className="rows">
        {data?.map((p) => (
          <li key={p.path}>
            <code>{p.path}</code>
          </li>
        ))}
      </ul>
    </section>
  )
}

/**
 * Closed days still holding segments.
 *
 * **Reported, never judged.** There is no colour here and no threshold: what
 * counts as too many is the operator's, and the screen's job is to show the
 * number rather than to have an opinion about it.
 */
function Overdue() {
  const { data, error } = $api.useQuery('get', '/v1/overdue')
  if (error) return <Refusal what="The overdue partitions" error={error} />
  return (
    <section>
      <h2>Closed, still holding {data ? <span className="count">{data.length}</span> : null}</h2>
      {data?.length === 0 ? <p className="muted">Nothing closed is still holding segments.</p> : null}
      <ul className="rows">
        {data?.map((o) => (
          <li key={o.path}>
            <code>{o.path}</code> <span className="count">{o.segments}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

/**
 * The live status of every venue.
 *
 * Once seen, never dropped: a venue that stops publishing stays listed with its
 * age climbing, because absence after presence is the statement an operator
 * most needs rendered.
 */
function Status() {
  const live = useLiveStatus()
  const venues = [...live.venues.values()].sort((a, b) => a.venue.localeCompare(b.venue))
  return (
    <section>
      <h2>
        Live <span className="count">{live.connected ? 'connected' : 'not connected'}</span>
      </h2>
      <p className="muted">
        {live.reconnects} reconnects · {live.missed} snapshots missed
        {live.missed > 0 ? ' — the stream said so rather than dropping them quietly' : null}
      </p>
      {venues.length === 0 ? (
        <p className="muted">
          {silenceReason(whySilent(live))} The record above does not depend on the bus.
        </p>
      ) : null}
      <ul className="rows">
        {venues.map((v) => (
          <li key={v.venue}>
            <code>{v.venue}</code>
            <span className="count">
              {/* The capture's own measurement where it has one, and only the
                  locally elapsed time where it does not. */}
              {heardAgo(
                (v.body as { observed_at_micros?: number } | null)?.observed_at_micros,
                (v.body as { last_flush_micros?: number } | null)?.last_flush_micros,
                v.received_ms,
              ) ?? sinceArrival(v.received_ms)}
              s ago
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}

/**
 * The screen.
 *
 * **Each panel inside its own boundary**, because one throwing used to take
 * every other with it — planted, built and served, the page came back with no
 * text content at all. These panels read different routes and answer different
 * questions; there is no reason for a malformed decimal in the tape to remove
 * the partition listing.
 *
 * The header is outside one deliberately: it is the page's own title, and a
 * boundary around it would render a refusal where the name of the thing should
 * be.
 */
export default function App() {
  return (
    <main>
      <Header />
      <Boundary name="Live">
        <Status />
      </Boundary>
      <Boundary name="Instruments">
        <Instruments />
      </Boundary>
      <Boundary name="Partitions">
        <Partitions />
      </Boundary>
      <Boundary name="Closed, still holding">
        <Overdue />
      </Boundary>
      <Boundary name="Candles">
        <Candles />
      </Boundary>
      <Boundary name="Gaps">
        <Gaps />
      </Boundary>
      <Boundary name="Failures">
        <Failures />
      </Boundary>
      <Boundary name="Coverage">
        <Coverage />
      </Boundary>
      <Boundary name="Rates">
        <Rates />
      </Boundary>
      <Boundary name="Tape">
        <Tape />
      </Boundary>
    </main>
  )
}
