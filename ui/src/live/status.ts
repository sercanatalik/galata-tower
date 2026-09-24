// The live status store, as the tower streams it.
//
// Three rules carried from the predecessor's `live/store.ts`, because they
// were paid for once and are still right:
//
//   Level-triggered. Every message is a WHOLE snapshot for one venue, never a
//   difference — so this store REPLACES and never patches, and a dropped
//   message costs one interval rather than information. It is also why the
//   server may drop at all.
//
//   Once seen, never dropped. A venue that stops publishing stays here with
//   its age climbing, because absence after presence is the statement an
//   operator most needs rendered. Removing it would make a dead venue look
//   like one that never existed.
//
//   Counted, never judged. Reconnects and missed snapshots are numbers the
//   screen shows. Whether either is acceptable is the operator's.

import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useSyncExternalStore } from 'react'

import type { components } from '../contract/api'

type Snapshot = components['schemas']['Snapshot']
type Board = components['schemas']['Board']
type BrokerState = components['schemas']['BrokerState']
type TapeMoved = components['schemas']['TapeMoved']

export interface LiveVenue {
  venue: string
  body: unknown
  /** Our wall clock when it arrived, so ages render between messages. */
  received_ms: number
}

export interface LiveState {
  connected: boolean
  /** Whether a board frame has arrived, which is how "none yet" is told from "not connected". */
  seenBoard: boolean
  /**
   * Whether the TOWER has a broker, which is not whether WE have the tower.
   *
   * `connected` above is this browser's `EventSource`. This is the tower's
   * subscription, and the two are independent: a reachable tower with no
   * broker is the case that used to render as "no venue has published status
   * yet" — true of the venues, and false about why.
   *
   * Null until a board frame arrives.
   */
  broker: BrokerState | null
  /**
   * Each kind's durable bound per venue, as the tower last read it.
   *
   * The tower watches this on everyone's behalf — asking whether the tape
   * moved is 124µs against 3ms to read it — so nothing here polls. A value
   * changing is the whole signal. **Per venue**, because each venue numbers
   * its own stream; the positions of two venues are not comparable.
   */
  bounds: Readonly<Record<string, Readonly<Record<string, number>>>>
  /**
   * Our wall clock when each kind last advanced.
   *
   * **Local, so it can be subtracted from a local clock.** Same rule as
   * `received_ms`: a duration measured here is the one thing this clock can
   * answer honestly.
   */
  advancedAt: Readonly<Record<string, number>>
  /**
   * How many times something has happened that means held data may be stale.
   *
   * **One number, because two can disagree.** This replaced a pair —
   * `advances` and `connections` — read only by `useFollowTheRecord`, once as
   * a guard and once as an effect dependency. Naming different members of that
   * pair in the two places is a guard that never runs, and neither line is
   * wrong read alone: it survived review and was found by planting a probe on
   * this store. One value cannot be mis-paired with itself.
   *
   * What increments it is [`isAReasonToResync`], which is where the reasoning
   * lives and where the tests reach.
   */
  resyncs: number
  /** Reconnects since load. A count, not a verdict. */
  /** How many times the stream DROPPED. Not reconnections: counted on the way
   * down, which is the event an operator cares about and the one that happens
   * first. During an outage a reconnect-counter would read 0 while the stream
   * was down — saying less, at the moment somebody is looking. */
  drops: number
  /** Snapshots the server told us we missed. A gap is an event, never an absence. */
  missed: number
  venues: ReadonlyMap<string, LiveVenue>
  /** Advances once a second so ages render without a message arriving. */
  tick: number
}

let state: LiveState = {
  connected: false,
  seenBoard: false,
  broker: null,
  bounds: {},
  advancedAt: {},
  resyncs: 0,
  drops: 0,
  missed: 0,
  venues: new Map(),
  tick: 0,
}

/** What just happened to the stream, or to the record behind it. */
export type StreamEvent = 'connected' | 'disconnected' | 'advanced'

/**
 * Does this mean what the screen holds may no longer be true?
 *
 * **Extracted so a test can reach it**, which is the shape this tree already
 * uses on the other side — `moves`, `union_micros`, `event_for` and `describe`
 * were each pulled out of the loop or handler that called them, for the stated
 * reason that *a branch that is only ever reasoned about is a branch that is
 * not held*. The same argument, in the other language: this needs no React, no
 * document and no `renderHook` to exercise three comparisons.
 *
 * The hook that used to decide this inline shipped three defects in two days,
 * each found by hand in a browser. The answers below are what those cost.
 */
export function isAReasonToResync(event: StreamEvent, connectionsBefore: number): boolean {
  switch (event) {
    // The record moved. The plainest reason there is.
    case 'advanced':
      return true

    // **A re-connection, yes — a first connection, no.** The first is the page
    // load, whose reads have just been done; refetching would undo them.
    case 'connected':
      return connectionsBefore > 0

    // **A disconnection, no — and this is the one that looks like a reason.**
    // The stream broke, so surely something changed? But the tower is usually
    // down at that moment: refetching fails, and with retries off the failure
    // is held. Measured at nine refusals across the screen. The reason to
    // refetch is the connection coming BACK.
    case 'disconnected':
      return false
  }
}

const listeners = new Set<() => void>()

function set(next: Partial<LiveState>) {
  state = { ...state, ...next }
  for (const l of listeners) l()
}

// How many times the stream has been established. Not in `LiveState`: nothing
// renders it, and the only question anyone asks of it is whether the next
// connection is the first — which `isAReasonToResync` answers.
let opened = 0

/**
 * A lag, ADDED to what was already missed.
 *
 * **It accumulates only because `state` is a module binding that `set`
 * reassigns.** Captured as a snapshot instead — which is the shape a refactor
 * to a closure or a hook produces naturally — every lag would overwrite the
 * last, and the screen would report the size of the most recent gap as the
 * session's total. Watched live on 2026-09-23: 4,000 snapshots published in
 * 166ms produced "3185 snapshots missed".
 *
 * A function rather than a line in a handler, because a line in a handler is
 * not reachable from a test that does not open a connection, and these tests
 * deliberately do not.
 */
/**
 * Where the record stands after a board frame, and which kinds that advanced.
 *
 * **A kind advanced when any one of its venues' positions did.** Each venue
 * numbers its own stream, so positions are compared venue by venue and never
 * across. A frame restating the same positions after a reconnect advances
 * nothing — treating it as an advance would show a stale table as fresh.
 */
export function afterBoard(
  live: LiveState,
  bounds: Record<string, Record<string, number>>,
  now: number,
): Pick<LiveState, 'bounds' | 'advancedAt'> {
  const advancedAt = { ...live.advancedAt }
  for (const [kind, venues] of Object.entries(bounds)) {
    const seen = live.bounds[kind] ?? {}
    if (Object.entries(venues).some(([venue, bound]) => seen[venue] !== bound)) {
      advancedAt[kind] = now
    }
  }
  return { bounds, advancedAt }
}

/** One venue's stream moved for one kind: record it, and stamp the kind. */
export function afterTapeMoved(
  live: LiveState,
  moved: TapeMoved,
  now: number,
): Pick<LiveState, 'bounds' | 'advancedAt'> {
  return {
    bounds: {
      ...live.bounds,
      [moved.kind]: { ...(live.bounds[moved.kind] ?? {}), [moved.venue]: moved.bound },
    },
    advancedAt: { ...live.advancedAt, [moved.kind]: now },
  }
}

export function afterLag(live: LiveState, missed: number): Partial<LiveState> {
  return { missed: live.missed + missed }
}

/**
 * A stream that went away, counted once however many times it is retried.
 *
 * **`EventSource` retries on its own, repeatedly, for as long as the stream is
 * down**, and every attempt raises `error`. Counting them all would turn a
 * half-minute outage into dozens of drops. The guard lives HERE rather than in
 * the caller so that a test can reach it: a guard in the caller is a guard no
 * test sees.
 *
 * Watched live: a tower killed for half a minute read "1 drop" and stayed at
 * one across every retry.
 */
export function afterDisconnect(live: LiveState): Partial<LiveState> {
  if (!live.connected) return { connected: false }
  return { connected: false, drops: live.drops + 1 }
}

let source: EventSource | null = null
let ticker: ReturnType<typeof setInterval> | null = null

function open() {
  if (source) return
  source = new EventSource('/v1/status')

  source.addEventListener('open', () => {
    // **Counted on the way UP.** `EventSource` reconnects on its own, and a
    // connection being established is the moment a refetch can succeed — the
    // disconnection is not, which is what `drops` records for the Live panel
    // to report. That counter was called `reconnects` until 2026-09-23, when
    // restarting the tower under a live browser showed the screen saying
    // "not connected · 1 reconnects" — one drop, zero reconnections. This
    // comment was already right; the name a reader saw was not.
    const again = isAReasonToResync('connected', opened)
    opened += 1
    set({ connected: true, resyncs: state.resyncs + (again ? 1 : 0) })
  })

  // **The answer to every gap.** The tower sends this on connect and again
  // whenever we fall behind, so connect, lag and reconnect all have one
  // answer: here is what is true now. Replace the whole map -- the same
  // replace-never-patch rule as a single snapshot, one level up.
  source.addEventListener('board', (event) => {
    const frame = JSON.parse((event as MessageEvent<string>).data) as Board
    const venues = new Map<string, LiveVenue>()
    const now = Date.now()
    for (const s of frame.venues) {
      venues.set(s.venue, { venue: s.venue, body: s.body, received_ms: now })
    }
    // The broker's state rides in the frame so that a browser connecting
    // DURING an outage learns it immediately, rather than waiting for a
    // change event that by definition will not come while nothing changes.
    // The bounds are where the record STANDS. Arrival is stamped only for a
    // kind we had not seen: a frame after a reconnect restates the same
    // position, and treating that as an advance would show a stale table as
    // fresh — which is the defect this whole change exists to fix.
    const { bounds, advancedAt } = afterBoard(state, frame.bounds ?? {}, now)
    set({ venues, connected: true, seenBoard: true, broker: frame.broker, bounds, advancedAt })
  })

  // The record moved. The rows are NOT here: the route that serves them caps
  // them already, and a browser not drawing this kind should not pay to
  // receive it. This says which kind, whose stream, and where it now stands.
  source.addEventListener('tape', (event) => {
    const moved = JSON.parse((event as MessageEvent<string>).data) as TapeMoved
    set({
      ...afterTapeMoved(state, moved, Date.now()),
      resyncs: state.resyncs + (isAReasonToResync('advanced', opened) ? 1 : 0),
      connected: true,
    })
  })

  // The tower's subscription came or went. A notification that the state
  // moved; the state itself always arrives in the board frame above.
  source.addEventListener('broker', (event) => {
    const broker = JSON.parse((event as MessageEvent<string>).data) as BrokerState
    set({ broker, connected: true })
  })

  source.addEventListener('status', (event) => {
    const snapshot = JSON.parse((event as MessageEvent<string>).data) as Snapshot
    // Replace, never patch. The message is the whole truth for that venue.
    const venues = new Map(state.venues)
    venues.set(snapshot.venue, {
      venue: snapshot.venue,
      body: snapshot.body,
      received_ms: Date.now(),
    })
    set({ venues, connected: true })
  })

  source.addEventListener('lagged', (event) => {
    const { missed } = JSON.parse((event as MessageEvent<string>).data) as { missed: number }
    set(afterLag(state, missed))
  })

  source.addEventListener('error', () => {
    // EventSource retries by itself. What is recorded is that it happened.
    set(afterDisconnect(state))
  })

  ticker ??= setInterval(() => set({ tick: state.tick + 1 }), 1000)
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  open()
  return () => {
    listeners.delete(listener)
  }
}

export function useLiveStatus(): LiveState {
  return useSyncExternalStore(subscribe, () => state, () => state)
}

/**
 * How long since a snapshot arrived here, in whole seconds.
 *
 * **A duration measured locally, which is the one thing this clock can answer.**
 * It is added to an age the capture measured; it is never subtracted from a
 * capture's timestamp, because a subtraction across two machines' clocks
 * reports their disagreement rather than the elapsed time.
 */
export function sinceArrival(received_ms: number): number {
  return Math.max(0, Math.round((Date.now() - received_ms) / 1000))
}

/**
 * How long a capture had gone without hearing anything, when it looked.
 *
 * `observed_at_micros` and `last_recv_micros` are both the CAPTURE's clock, so
 * subtracting them is meaningful. The locally elapsed time since the snapshot
 * arrived is added on top, so the number advances between messages without the
 * browser's clock ever being compared to the capture's.
 *
 * This is what the venue panel got wrong: it used `Date.now()` minus arrival
 * for the whole figure, which reports the viewer's network rather than the
 * venue's silence.
 */
export function heardAgo(
  observed_at_micros: number | null | undefined,
  last_recv_micros: number | null | undefined,
  received_ms: number,
): number | null {
  if (!observed_at_micros || !last_recv_micros) return null
  const measured = Math.round((observed_at_micros - last_recv_micros) / 1_000_000)
  return Math.max(0, measured) + sinceArrival(received_ms)
}

/**
 * How far the venue's own timestamp sits behind our receipt, in seconds.
 *
 * **Crosses clocks deliberately.** This is skew and transport together, and it
 * is the number `PairStatus` keeps both fields to produce — a surface holding
 * one of them cannot. It can be NEGATIVE, meaning the venue claims data from
 * the future, and that is shown rather than clamped: hiding it would hide the
 * finding the two fields exist for.
 */
export function venueLag(
  last_recv_micros: number | null | undefined,
  last_event_micros: number | null | undefined,
): number | null {
  if (!last_recv_micros || !last_event_micros) return null
  return Math.round((last_recv_micros - last_event_micros) / 1_000_000)
}

/**
 * Why nothing is listed — the question an empty panel used to answer wrongly.
 *
 * **Four states, and they are not interchangeable.** The screen showed one
 * sentence, "No venue has published status yet", for every one of them; it is
 * true only of the last, and during an outage it is a reassuring lie that
 * costs an afternoon of looking at the capture instead of the broker.
 */
export type Silence =
  /** Venues are publishing; there is nothing to explain. */
  | { case: 'publishing' }
  /** This browser has not had a board frame yet. Ours, not the tower's. */
  | { case: 'no-stream' }
  /** We had a stream and lost it. Everything the tower told us is now the
   * last thing it told us, not the current state — including whether it has a
   * broker. Ours, not the tower's. */
  | { case: 'stream-lost' }
  /** The tower is reachable and has no broker. It is still trying. */
  | { case: 'no-broker'; attempts: number; refusal: string | null }
  /** The tower is subscribed and no venue has published. */
  | { case: 'nothing-published' }

/** Which of the four, from the state the tower sent. */
export function whySilent(live: LiveState): Silence {
  if (live.venues.size > 0) return { case: 'publishing' }
  if (!live.seenBoard) return { case: 'no-stream' }
  // **Before anything is said about the broker.** This asked only whether a
  // board had EVER arrived, so after the stream dropped it went on reporting
  // the last board's broker state as though it were current — observed
  // 2026-09-23 as "Live not connected" directly above "The tower is
  // subscribed", which cannot both be known at once. What the tower is doing
  // while we cannot hear it is not something this browser knows.
  if (!live.connected) return { case: 'stream-lost' }
  if (live.broker && !live.broker.connected) {
    return {
      case: 'no-broker',
      attempts: live.broker.attempts,
      refusal: live.broker.refusal ?? null,
    }
  }
  return { case: 'nothing-published' }
}

/**
 * The reason as a sentence, shared so two panels cannot disagree about it.
 *
 * **Counted, never judged**, like the drops beside it: the attempt count
 * is a number and the refusal is the broker's own words. There is no threshold
 * here at which this turns red, because the tower does not know one.
 */
export function silenceReason(silence: Silence): string {
  switch (silence.case) {
    case 'publishing':
      return ''
    case 'no-stream':
      return 'Waiting for the stream.'
    case 'stream-lost':
      return 'The stream to this tower has dropped, so nothing here is current — what the tower is doing, and whether it still has a broker, are things this browser cannot see while it is disconnected.'
    case 'no-broker': {
      // Zero is not "it has not tried" — it is a connection that WAS
      // established and dropped, where the count belongs to the client doing
      // the reconnecting rather than to us. Saying "0 attempts" would be a
      // number that means something other than what it reads as.
      const tries = silence.attempts === 0
        ? ''
        : ` — ${silence.attempts === 1 ? '1 attempt' : `${silence.attempts} attempts`} so far`
      const said = silence.refusal ? `, ${silence.refusal}` : ''
      return `The tower has no broker${tries}${said}. It keeps trying.`
    }
    case 'nothing-published':
      return 'The tower is subscribed; no venue has published yet.'
  }
}

/**
 * How long since this kind's record last advanced, in whole seconds.
 *
 * **Reporting only.** This used to invalidate queries as well, with a
 * predicate that matched `/v1/tape/{kind}` — written for the two panels that
 * read the tape, and then inherited unchanged by five that did not. Refetching
 * is `useFollowTheRecord`'s, mounted once; this answers the question six of
 * its seven callers actually asked.
 *
 * `null` where the kind has never advanced, because a current table and one
 * from an hour ago are otherwise identical.
 */
export function useRecordAdvances(kind: string): number | null {
  const live = useLiveStatus()
  const at = live.advancedAt[kind]
  // `live.tick` is read so this recomputes once a second without a message.
  void live.tick
  return at === undefined ? null : sinceArrival(at)
}

/**
 * Refetch what the record feeds, when the record moves.
 *
 * **Mounted once, beside the panels — not once per panel.** This replaced a
 * predicate inside `useRecordAdvances` that matched only `/v1/tape/{kind}`.
 * It was written for the two panels that read the tape, and five later panels
 * inherited the hook, printed *"advanced 2s ago"*, and never refetched: a
 * claim about freshness the panel did not keep, which is the same defect as a
 * reassuring sentence during an outage.
 *
 * **Everything, rather than a list.** When the record moves, every figure
 * derived from it is stale — partitions, overdue days, instruments, coverage,
 * rates, failures, the tape and the chart. A predicate naming some of them is
 * a second list of what the record feeds, and the one that existed had already
 * fallen out of step with the panels. TanStack refetches only what is mounted.
 *
 * **Measured, and a cheaper shape deliberately not built.** Three routes fold
 * the whole tape on every request — `/v1/instruments` 168ms, `/v1/coverage`
 * 171ms, `/v1/rates` 162ms — so an advance costs about 500ms of decode.
 * (`/v1/gaps` is 1.5ms and `/v1/failures` 0.8ms; they read one dataset and a
 * directory walk.) That is affordable because the tape is written by
 * `galata-tape-rebuild` as a per-date backfill, so it advances when a rebuild
 * runs and not continuously.
 *
 * **When to revisit:** if the tape ever becomes a streaming projection rather
 * than a rebuild, those three folds become 500ms per advance per browser, and
 * the answer is one shared fold behind the tower's own bound watcher — which
 * already knows exactly when to invalidate it. Until then it would be a cache
 * with no invalidation problem to solve.
 */
export function useFollowTheRecord(): void {
  const live = useLiveStatus()
  const client = useQueryClient()

  // **A re-connection is a reason too, on the same footing as an advance.** Both
  // say *what you hold may no longer be true*: an advance because the record
  // moved, a reconnect because there was a period in which it could have moved
  // unobserved and the browser cannot tell which.
  //
  // This is the argument the tower already makes for the venue board, one
  // level out — *"Connect, lag, reconnect — all three are the same question,
  // what is true now"* — applied to the venues and not to the record. Measured
  // before it was: with the tower restarted and the record changed while it
  // was down, the screen reconnected and went on reporting 36 instruments
  // against a record holding 24.
  //
  // The tower cannot do this for us: a reconnecting browser and a new one are
  // indistinguishable to it, and the browser is the one holding the stale
  // figures.
  useEffect(() => {
    // Nothing has happened yet: a board frame is where the record STANDS
    // rather than an advance, and the FIRST connection is the page load, whose
    // reads have just been done.
    //
    // It is the connection that triggers this, never the disconnection. An
    // earlier draft keyed on the drop counter (then called `reconnects`, now
    // `drops`) and so refetched everything the
    // moment the stream dropped — while the tower was still down. With
    // retries off, all nine panels held that refusal permanently.
    if (live.resyncs === 0) return
    client.invalidateQueries()
    // **`connections`, matching the guard above.** An earlier draft guarded on
    // `connections` and depended on the drop counter (`drops` now): the effect
    // re-ran on the
    // DROP, where the guard correctly returned early, and never again on the
    // reopen — so nothing refetched and the screen kept its pre-outage
    // figures. A guard and a dependency that name different things is a guard
    // that does not run.
  }, [live.resyncs, client])
}

