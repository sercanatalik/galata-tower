// The live stream from the tower. Snapshots are whole, so the store replaces and never patches;
// a venue once seen is never dropped; drops and misses are counted, never judged.

import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useSyncExternalStore } from 'react'

import type { components } from '../contract/api'

type Snapshot = components['schemas']['Snapshot']
type Board = components['schemas']['Board']
type BrokerState = components['schemas']['BrokerState']
type TapeMoved = components['schemas']['TapeMoved']
type ArchiveMoved = components['schemas']['ArchiveMoved']

export interface LiveVenue {
  venue: string
  body: unknown
  /** Our wall clock when it arrived, so ages render between messages. */
  received_ms: number
}

export interface LiveState {
  /** This browser's stream to the tower. */
  connected: boolean
  /** Whether a board frame has arrived, which tells "none yet" from "not connected". */
  seenBoard: boolean
  /** The tower's own broker subscription; null until told. */
  broker: BrokerState | null
  /** Each kind's durable bound per venue. Per venue, because each numbers its own stream. */
  bounds: Readonly<Record<string, Readonly<Record<string, number>>>>
  /** Our wall clock when each kind last advanced. */
  advancedAt: Readonly<Record<string, number>>
  /** Each venue's archive frontier, as the tower last announced it (our clock, micros). */
  archive: Readonly<Record<string, number>>
  /** Our wall clock when any archive frontier last moved. */
  archiveAt: number | null
  /** Bumped by every event that means held tape figures may be stale. One value, so guard and effect cannot disagree. */
  resyncs: number
  /** Times the stream dropped. */
  drops: number
  /** Snapshots the server told us we missed. */
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
  archive: {},
  archiveAt: null,
  resyncs: 0,
  drops: 0,
  missed: 0,
  venues: new Map(),
  tick: 0,
}

/** What just happened to the stream, or to the record behind it. */
export type StreamEvent = 'connected' | 'disconnected' | 'advanced'

/** Does this mean what the screen holds may no longer be true? */
export function isAReasonToResync(event: StreamEvent, connectionsBefore: number): boolean {
  switch (event) {
    case 'advanced':
      return true
    // A reconnect cannot know what moved while it was away; the first connect has just read everything.
    case 'connected':
      return connectionsBefore > 0
    // The tower is likely unreachable; a failure taken now would be held.
    case 'disconnected':
      return false
  }
}

const listeners = new Set<() => void>()

function set(next: Partial<LiveState>) {
  state = { ...state, ...next }
  for (const l of listeners) l()
}

let opened = 0

/** A board frame's bounds, with the kinds that moved stamped now. */
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

export function afterArchiveMoved(
  live: LiveState,
  moved: ArchiveMoved,
  now: number,
): Pick<LiveState, 'archive' | 'archiveAt'> {
  return { archive: { ...live.archive, [moved.venue]: moved.frontier_micros }, archiveAt: now }
}

export function afterLag(live: LiveState, missed: number): Partial<LiveState> {
  return { missed: live.missed + missed }
}

/** Counted on the way down, so an outage reads as a drop while it lasts. */
export function afterDisconnect(live: LiveState): Partial<LiveState> {
  if (!live.connected) return { connected: false }
  return { connected: false, drops: live.drops + 1 }
}

let source: EventSource | null = null
let ticker: ReturnType<typeof setInterval> | null = null

function read<T>(event: Event): T {
  return JSON.parse((event as MessageEvent<string>).data) as T
}

function open() {
  if (source) return
  source = new EventSource('/v1/status')

  source.addEventListener('open', () => {
    const again = isAReasonToResync('connected', opened)
    opened += 1
    set({ connected: true, resyncs: state.resyncs + (again ? 1 : 0) })
  })

  source.addEventListener('board', (event) => {
    const frame = read<Board>(event)
    const venues = new Map<string, LiveVenue>()
    const now = Date.now()
    for (const s of frame.venues) {
      venues.set(s.venue, { venue: s.venue, body: s.body, received_ms: now })
    }
    const { bounds, advancedAt } = afterBoard(state, frame.bounds ?? {}, now)
    set({ venues, connected: true, seenBoard: true, broker: frame.broker, bounds, advancedAt })
  })

  source.addEventListener('tape', (event) => {
    set({
      ...afterTapeMoved(state, read<TapeMoved>(event), Date.now()),
      resyncs: state.resyncs + (isAReasonToResync('advanced', opened) ? 1 : 0),
      connected: true,
    })
  })

  // Not a resync: only the latest prices follow the archive.
  source.addEventListener('archive', (event) => {
    set({ ...afterArchiveMoved(state, read<ArchiveMoved>(event), Date.now()), connected: true })
  })

  source.addEventListener('broker', (event) => {
    set({ broker: read<BrokerState>(event), connected: true })
  })

  source.addEventListener('status', (event) => {
    const snapshot = read<Snapshot>(event)
    const venues = new Map(state.venues)
    venues.set(snapshot.venue, { venue: snapshot.venue, body: snapshot.body, received_ms: Date.now() })
    set({ venues, connected: true })
  })

  source.addEventListener('lagged', (event) => {
    set(afterLag(state, read<{ missed: number }>(event).missed))
  })

  source.addEventListener('error', () => {
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

/** Seconds since a local arrival, by the local clock alone. */
export function sinceArrival(received_ms: number): number {
  return Math.max(0, Math.round((Date.now() - received_ms) / 1000))
}

/** How long a capture had gone unheard: its own two stamps, plus local time since we heard it. */
export function heardAgo(
  observed_at_micros: number | null | undefined,
  last_recv_micros: number | null | undefined,
  received_ms: number,
): number | null {
  if (!observed_at_micros || !last_recv_micros) return null
  const measured = Math.round((observed_at_micros - last_recv_micros) / 1_000_000)
  return Math.max(0, measured) + sinceArrival(received_ms)
}

/** How far the venue's stamp sits from our receipt, by the capture's clock. */
export function venueLag(
  last_recv_micros: number | null | undefined,
  last_event_micros: number | null | undefined,
): number | null {
  if (!last_recv_micros || !last_event_micros) return null
  return Math.round((last_recv_micros - last_event_micros) / 1_000_000)
}

export type Silence =
  | { case: 'publishing' }
  | { case: 'no-stream' }
  | { case: 'stream-lost' }
  | { case: 'no-broker'; attempts: number; refusal: string | null }
  | { case: 'nothing-published' }

/** Why no venue is shown. Each case says its own cause. */
export function whySilent(live: LiveState): Silence {
  if (live.venues.size > 0) return { case: 'publishing' }
  if (!live.seenBoard) return { case: 'no-stream' }
  if (!live.connected) return { case: 'stream-lost' }
  if (live.broker && !live.broker.connected) {
    return { case: 'no-broker', attempts: live.broker.attempts, refusal: live.broker.refusal ?? null }
  }
  return { case: 'nothing-published' }
}

export function silenceReason(silence: Silence): string {
  switch (silence.case) {
    case 'publishing':
      return ''
    case 'no-stream':
      return 'Waiting for the stream.'
    case 'stream-lost':
      return 'The stream to this tower has dropped, so nothing here is current — what the tower is doing, and whether it still has a broker, are things this browser cannot see while it is disconnected.'
    case 'no-broker': {
      const tries =
        silence.attempts === 0
          ? ''
          : ` — ${silence.attempts === 1 ? '1 attempt' : `${silence.attempts} attempts`} so far`
      const said = silence.refusal ? `, ${silence.refusal}` : ''
      return `The tower has no broker${tries}${said}. It keeps trying.`
    }
    case 'nothing-published':
      return 'The tower is subscribed; no venue has published yet.'
  }
}

/** Seconds since this kind last advanced, or null if it has not since load. */
export function useRecordAdvances(kind: string): number | null {
  const live = useLiveStatus()
  const at = live.advancedAt[kind]
  void live.tick
  return at === undefined ? null : sinceArrival(at)
}

/** Mounted once: refetch everything derived from the tape whenever it may be stale. */
export function useFollowTheRecord(): void {
  const live = useLiveStatus()
  const client = useQueryClient()
  useEffect(() => {
    if (live.resyncs === 0) return
    client.invalidateQueries({ predicate: (q) => !isLatest(q.queryKey) })
  }, [live.resyncs, client])
}

/** Mounted once: refetch the latest prices when the archive moves, at most every two seconds. */
export function useFollowTheArchive(): void {
  const live = useLiveStatus()
  const client = useQueryClient()
  useEffect(() => {
    // One pending refetch at a time; clearing it on every event would starve it.
    if (live.archiveAt === null || pending !== null) return
    pending = setTimeout(() => {
      pending = null
      lastLatest = Date.now()
      client.invalidateQueries({ predicate: (q) => isLatest(q.queryKey) })
    }, Math.max(0, lastLatest + 2000 - Date.now()))
  }, [live.archiveAt, client])
}

let lastLatest = 0
let pending: ReturnType<typeof setTimeout> | null = null

function isLatest(key: readonly unknown[]): boolean {
  return key[1] === '/v1/latest'
}
