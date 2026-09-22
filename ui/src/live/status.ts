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
   * Each kind's durable bound, as the tower last read it.
   *
   * The tower watches this on everyone's behalf — asking whether the tape
   * moved is 53µs against 2.85ms to read it — so nothing here polls. A value
   * changing is the whole signal.
   */
  bounds: Readonly<Record<string, number>>
  /**
   * Our wall clock when each kind last advanced.
   *
   * **Local, so it can be subtracted from a local clock.** Same rule as
   * `received_ms`: a duration measured here is the one thing this clock can
   * answer honestly.
   */
  advancedAt: Readonly<Record<string, number>>
  /** Reconnects since load. A count, not a verdict. */
  reconnects: number
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
  reconnects: 0,
  missed: 0,
  venues: new Map(),
  tick: 0,
}

const listeners = new Set<() => void>()

function set(next: Partial<LiveState>) {
  state = { ...state, ...next }
  for (const l of listeners) l()
}

let source: EventSource | null = null
let ticker: ReturnType<typeof setInterval> | null = null

function open() {
  if (source) return
  source = new EventSource('/v1/status')

  source.addEventListener('open', () => {
    // Not the first open: EventSource reconnects on its own, and each one
    // after the first is worth counting.
    set({ connected: true, reconnects: state.connected ? state.reconnects : state.reconnects })
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
    const bounds = (frame.bounds ?? {}) as Record<string, number>
    const advancedAt = { ...state.advancedAt }
    for (const [kind, bound] of Object.entries(bounds)) {
      if (state.bounds[kind] !== bound) advancedAt[kind] = now
    }
    set({ venues, connected: true, seenBoard: true, broker: frame.broker, bounds, advancedAt })
  })

  // The record moved. The rows are NOT here: the route that serves them caps
  // them already, and a browser not drawing this kind should not pay to
  // receive it. This says which kind, and where it now stands.
  source.addEventListener('tape', (event) => {
    const moved = JSON.parse((event as MessageEvent<string>).data) as TapeMoved
    set({
      bounds: { ...state.bounds, [moved.kind]: moved.bound },
      advancedAt: { ...state.advancedAt, [moved.kind]: Date.now() },
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
    set({ missed: state.missed + missed })
  })

  source.addEventListener('error', () => {
    // EventSource retries by itself. What is recorded is that it happened.
    if (state.connected) set({ connected: false, reconnects: state.reconnects + 1 })
    else set({ connected: false })
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
  /** The tower is reachable and has no broker. It is still trying. */
  | { case: 'no-broker'; attempts: number; refusal: string | null }
  /** The tower is subscribed and no venue has published. */
  | { case: 'nothing-published' }

/** Which of the four, from the state the tower sent. */
export function whySilent(live: LiveState): Silence {
  if (live.venues.size > 0) return { case: 'publishing' }
  if (!live.seenBoard) return { case: 'no-stream' }
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
 * **Counted, never judged**, like the reconnects beside it: the attempt count
 * is a number and the refusal is the broker's own words. There is no threshold
 * here at which this turns red, because the tower does not know one.
 */
export function silenceReason(silence: Silence): string {
  switch (silence.case) {
    case 'publishing':
      return ''
    case 'no-stream':
      return 'Waiting for the stream.'
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
 * Refetch this kind's reads when the record advances, and not otherwise.
 *
 * **Told, never asking.** A `refetchInterval` would decode 39,231 rows on a
 * timer to usually learn nothing; a conditional request would still cost a
 * round trip per browser per interval to be told nothing happened. The tower
 * holds an open channel and watches the bound once for everybody, so the
 * browser reads only when there is something new to read.
 *
 * Returns how long since this kind last advanced, in whole seconds, or null if
 * it never has — because a current table and an hour-old one are otherwise
 * identical, which is the defect the whole change is about.
 */
export function useRecordAdvances(kind: string): number | null {
  const live = useLiveStatus()
  const client = useQueryClient()
  const bound = live.bounds[kind]

  useEffect(() => {
    if (bound === undefined) return
    // Matched by predicate rather than by rebuilding the key. The key
    // openapi-react-query makes carries the full params, so an equality test
    // would have to restate every query's window and limit — three places to
    // fall out of step instead of one predicate that reads the kind.
    client.invalidateQueries({
      predicate: (query) => {
        const key = query.queryKey as unknown[]
        if (key[1] !== '/v1/tape/{kind}') return false
        const params = key[2] as { params?: { path?: { kind?: string } } } | undefined
        return params?.params?.path?.kind === kind
      },
    })
  }, [bound, kind, client])

  const at = live.advancedAt[kind]
  // `live.tick` is read so this recomputes once a second without a message.
  void live.tick
  return at === undefined ? null : sinceArrival(at)
}
