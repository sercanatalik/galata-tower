import { describe, expect, it } from 'vitest'

import type { LiveState } from '../live/status'
import { captures, lags } from './truth'

const HOUR = 3_600_000_000

function live(over: Partial<LiveState>): LiveState {
  return {
    connected: true,
    seenBoard: true,
    broker: { connected: true, attempts: 0, refusal: null },
    bounds: {},
    advancedAt: {},
    archive: {},
    archiveAt: null,
    resyncs: 0,
    drops: 0,
    missed: 0,
    venues: new Map(),
    tick: 0,
    ...over,
  }
}

describe('lags', () => {
  it('states a tape days behind its archive', () => {
    const [l] = lags([{ venue: 'hyperliquid', archive_micros: 75 * HOUR, tape_micros: 0.5 * HOUR }], {})
    expect(l.text).toBe('3 d 2 h')
  })

  it('says nothing when the tape is within a minute', () => {
    expect(lags([{ venue: 'hyperliquid', archive_micros: HOUR, tape_micros: HOUR - 30_000_000 }], {})).toEqual([])
  })

  it('takes the newer of the served and the streamed archive frontier', () => {
    const [l] = lags([{ venue: 'v', archive_micros: HOUR, tape_micros: HOUR }], { v: 2 * HOUR })
    expect(l.micros).toBe(HOUR)
  })

  it('names a venue the tape holds nothing for', () => {
    const [l] = lags([{ venue: 'v', archive_micros: HOUR, tape_micros: null }], {})
    expect(l.text).toBe('no tape at all')
  })
})

describe('captures', () => {
  const venues = new Map([['hyperliquid', { venue: 'hyperliquid', body: { subs_held: 24, subs_declared: 24 }, received_ms: Date.now() }]])

  it('reports a publishing venue', () => {
    expect(captures(live({ venues }))[0].tone).toBe('ok')
  })

  it('makes every age unknown while the bus is down', () => {
    const [c] = captures(live({ venues, broker: { connected: false, attempts: 3, refusal: 'refused' } }))
    expect(c.value).toBe('unknown')
  })
})
