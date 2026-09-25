import type { components } from '../contract/api'
import { lag } from '../kit/format'
import { heardAgo, type LiveState } from '../live/status'

type About = components['schemas']['About']

/** A lag shorter than this is the rebuild's ordinary cadence, not news. */
export const LAG_WORTH_SAYING_MICROS = 60_000_000

export type Tone = 'ok' | 'bad' | 'unknown'

export interface Fact {
  label: string
  value: string
  detail: string
  tone: Tone
}

/** What the capture says of itself, as the status stream carries it. */
export interface CaptureBody {
  observed_at_micros?: number
  last_flush_micros?: number
  session_age_secs?: number
  sink_dropped?: number
  subs_held?: number
  subs_declared?: number
  subs_refused?: number
  count_window_secs?: number
  pairs?: Array<{
    ticker?: string
    series?: string
    state?: string
    count?: number
    last_recv_micros?: number | null
    last_event_micros?: number | null
  }>
}

export const body = (v: { body: unknown } | undefined): CaptureBody => (v?.body ?? {}) as CaptureBody

/** The four facts, each with its evidence. Amber only where the tower cannot vouch. */
export function facts(about: About | undefined, live: LiveState): Fact[] {
  const root = (label: string, r: About['archive'] | undefined): Fact =>
    !r
      ? { label, value: 'reading…', detail: '', tone: 'unknown' }
      : r.readable
        ? { label, value: 'readable', detail: r.path, tone: 'ok' }
        : { label, value: 'not there', detail: `${r.path} · set ${r.var}`, tone: 'bad' }

  const tape: Fact = !about
    ? { label: 'Tape', value: 'reading…', detail: '', tone: 'unknown' }
    : !about.tape.readable
      ? root('Tape', about.tape)
      : about.tape_problems.length > 0
        ? { label: 'Tape', value: 'malformed, counts too high', detail: about.tape_problems[0], tone: 'bad' }
        : { label: 'Tape', value: 'well-formed', detail: 'no overlapping sequence ranges', tone: 'ok' }

  const broker = live.broker
  const bus: Fact = !live.connected
    ? { label: 'Bus', value: 'unknown', detail: 'this browser has lost the tower', tone: 'unknown' }
    : !broker
      ? { label: 'Bus', value: 'waiting…', detail: '', tone: 'unknown' }
      : broker.connected
        ? { label: 'Bus', value: 'connected', detail: 'status.> · 0 reconnect attempts', tone: 'ok' }
        : {
            label: 'Bus',
            value: 'lost',
            detail: `${broker.refusal ?? 'not connected'}${broker.attempts ? ` · ${broker.attempts} attempts` : ''}`,
            tone: 'bad',
          }

  return [root('Archive', about?.archive), tape, bus, ...captures(live)]
}

/** One fact per venue. While the bus is down an age would climb for every venue alike, so it is unknown. */
export function captures(live: LiveState): Fact[] {
  const venues = [...live.venues.values()].sort((a, b) => a.venue.localeCompare(b.venue))
  const busUp = live.connected && live.broker?.connected === true
  if (venues.length === 0) {
    return [{ label: 'Capture', value: busUp ? 'none publishing' : 'unknown', detail: 'no status.<venue> seen', tone: 'unknown' }]
  }
  return venues.map((v) => {
    const b = body(v)
    const label = `Capture · ${v.venue}`
    if (!busUp) return { label, value: 'unknown', detail: 'the bus is down, so no age is measured', tone: 'unknown' }
    const heard = heardAgo(b.observed_at_micros, b.last_flush_micros, v.received_ms)
    const held = b.subs_declared ? `${b.subs_held ?? 0} of ${b.subs_declared} held` : ''
    const drops = b.sink_dropped !== undefined ? `${b.sink_dropped} sink drops` : ''
    return {
      label,
      value: heard === null ? 'publishing' : `live, heard ${heard} s ago`,
      detail: [held, drops].filter(Boolean).join(' · '),
      tone: 'ok',
    }
  })
}

export interface Lag {
  venue: string
  micros: number
  archive: number
  tape: number | null
  text: string
}

/** Venues whose tape is behind their archive by more than a minute, on the one clock both are stamped by. */
export function lags(frontiers: About['frontiers'], archiveLive: Readonly<Record<string, number>>): Lag[] {
  return frontiers
    .map((f) => {
      const archive = Math.max(f.archive_micros ?? 0, archiveLive[f.venue] ?? 0)
      const tape = f.tape_micros ?? null
      const micros = archive - (tape ?? 0)
      return { venue: f.venue, micros, archive, tape, text: tape === null ? 'no tape at all' : lag(micros) }
    })
    .filter((l) => l.archive > 0 && (l.tape === null || l.micros > LAG_WORTH_SAYING_MICROS))
}
