import { $api } from './contract/client'
import { panelState } from './panel'
import {
  heardAgo,
  silenceReason,
  sinceArrival,
  useFollowTheRecord,
  useLiveStatus,
  whySilent,
} from './live/status'
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
  // **Never empty, and that is stated rather than left out.** `/v1/about`
  // answers with two roots or it fails; there is no collection here to be
  // empty of. Saying so costs one lambda and leaves no panel in this tree
  // deciding the order for itself.
  const state = panelState($api.useQuery('get', '/v1/about'), () => false)
  const data = state.kind === 'reading' || state.kind === 'refused' ? undefined : state.data
  // Configured and observed are different facts, and an empty listing is
  // consistent with both a clean record and a typo.
  const missing = [data?.archive, data?.tape].filter((r) => r && !r.readable) as Array<{
    path: string
    var: string
  }>
  if (state.kind === 'refused') return <Refusal what="The archive" error={state.error} />
  return (
    <header>
      <h1>galata-tower</h1>
      <p className="muted">
        watching <code>{data?.archive.path ?? '…'}</code>
        {data ? ' · tape ' : null}
        {data ? <code>{data.tape.path}</code> : null}
      </p>
      {/* **Said once, here, where the paths already are.** A tower pointed at
          a directory that does not exist answered every surface with a
          plausible empty result and none of them said "there is no here" — an
          operator with one typo got a calm, entirely empty dashboard. Eight
          panels each repeating a diagnosis would be eight statements of one
          fact; this is the one place the paths are printed. */}
      {missing.length > 0 ? (
        <div className="refusal">
          <strong>
            {missing.length === 1 ? 'A root is not there.' : 'Neither root is there.'}
          </strong>
          {missing.map((r) => (
            <p key={r.var}>
              <code>{r.path}</code> cannot be listed — set <code>{r.var}</code>
            </p>
          ))}
          <p className="muted">
            Every panel below will look empty, which is what an absent directory and a clean
            record have in common.
          </p>
        </div>
      ) : null}
      {/* **The tape can be wrong in a way that inflates every figure below.**
          Two segments whose sequence ranges overlap serve the overlap twice.
          Measured on the real tape, 2026-09-23: this tower reported 108,098
          rows where the rebuild had written 107,312, and said nothing —
          `galata-tape-rebuild` had been printing the same problem on every
          run since it was written, and the tower linked the same crate and
          never asked.

          Beside the missing-root warning because it is the same kind of fact:
          the numbers below are not what they appear. Reported, never repaired
          — the tape is `galata-tape-rebuild`'s to write, and a reader that
          deleted segments would be a second writer. */}
      {data && data.tape_problems.length > 0 ? (
        <div className="refusal">
          <strong>
            The tape is malformed, so the counts below are too high.
          </strong>
          {data.tape_problems.map((problem) => (
            <p key={problem}>{problem}</p>
          ))}
          <p className="muted">
            Rows in an overlapping range are read twice. <code>galata-tape-rebuild --replace</code>{' '}
            rewrites the affected partitions; nothing here will, because the tape is written by
            that and read by this.
          </p>
        </div>
      ) : null}
      <p className="muted">
        prunes on {data?.prune_on.map((c) => <code key={c}>{c}</code>) ?? null}
      </p>
    </header>
  )
}

/** The partitions the record holds. A fact on disk, not a claim by a process. */
function Partitions() {
  const state = panelState($api.useQuery('get', '/v1/partitions'), (d) => d.length === 0)
  if (state.kind === 'refused') return <Refusal what="The partitions" error={state.error} />
  const data = state.kind === 'reading' ? undefined : state.data
  return (
    <section>
      <h2>Partitions {data ? <span className="count">{data.length}</span> : null}</h2>
      {state.kind === 'reading' ? <p className="muted">reading the store…</p> : null}
      {state.kind === 'empty' ? <p className="muted">The archive holds none.</p> : null}
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
  // **This had no reading case.** `data?.length === 0` is false while the read
  // is in flight, so a pending panel drew its heading over nothing and said
  // why. It is the sixth hand-written copy, and the one that was missing a
  // branch — which is the argument for this function in one panel.
  const state = panelState($api.useQuery('get', '/v1/overdue'), (d) => d.length === 0)
  if (state.kind === 'refused') return <Refusal what="The overdue partitions" error={state.error} />
  const data = state.kind === 'reading' ? undefined : state.data
  return (
    <section>
      <h2>Closed, still holding {data ? <span className="count">{data.length}</span> : null}</h2>
      {state.kind === 'reading' ? <p className="muted">reading the record…</p> : null}
      {state.kind === 'empty' ? (
        <p className="muted">Nothing closed is still holding segments.</p>
      ) : null}
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
        {live.drops} {live.drops === 1 ? 'drop' : 'drops'} · {live.missed} snapshots missed
        {live.missed > 0 ? ' — the stream said so rather than dropping them quietly' : null}
      </p>
      {venues.length === 0 ? (
        <p className="muted">
          {silenceReason(whySilent(live))} The record above does not depend on the bus.
        </p>
      ) : live.broker && !live.broker.connected ? (
        /* **The bus, said whenever it is gone — not only when there is nothing
            else to show.**

            This branch used to be the only place the broker's state appeared,
            reached only when no venue had ever been seen. Once one has, it is
            never dropped — absence after presence is the statement an operator
            most needs — so a dead bus rendered as a venue quietly getting
            older. Found by running a real broker for the first time and then
            killing it: the screen said `Live connected · 0 drops` over
            `hyperliquid 53s ago`, every clause true and the conclusion wrong.

            `connected` above is the BROWSER's stream to this tower, which is
            genuinely up. The bus is a different fact and now has its own
            line.

            **`live.broker &&` is not defensive noise.** It is null until the
            first board frame arrives, and *not yet told* is not *down* — a
            screen that announced a lost bus for the moment before it had been
            told anything would be wrong every single load. */
        <p className="muted">
          <strong>The tower has lost the broker.</strong>{' '}
          {live.broker.refusal ?? 'it is not connected'}
          {live.broker.attempts > 0 ? ` — ${live.broker.attempts} attempts so far` : null}. Nothing
          can arrive while it is down, so the ages below are <em>not</em> how long each venue has
          been quiet — every one of them climbs at the same rate whatever the venue is doing. The
          record above does not depend on the bus.
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
  // **Mounted once, here.** When the record moves, everything derived from it
  // is stale; the alternative was a predicate inside each panel's hook, which
  // had already fallen out of step with five of them.
  useFollowTheRecord()
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
