import { $api } from './contract/client'
import { ageSeconds, useLiveStatus } from './live/status'

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
          No venue has published status yet. The record above does not depend on the bus.
        </p>
      ) : null}
      <ul className="rows">
        {venues.map((v) => (
          <li key={v.venue}>
            <code>{v.venue}</code>
            <span className="count">{ageSeconds(v.received_ms)}s ago</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

export default function App() {
  return (
    <main>
      <Header />
      <Status />
      <Partitions />
      <Overdue />
    </main>
  )
}
