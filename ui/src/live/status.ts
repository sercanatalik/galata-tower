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

import { useSyncExternalStore } from 'react'

import type { components } from '../contract/api'

type Snapshot = components['schemas']['Snapshot']
type Board = components['schemas']['Board']
type BrokerState = components['schemas']['BrokerState']

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
    set({ venues, connected: true, seenBoard: true, broker: frame.broker })
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
