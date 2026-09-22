import { describe, expect, it } from 'vitest'

import { heardAgo, silenceReason, sinceArrival, venueLag, whySilent } from './status'
import type { LiveState } from './status'

/** A LiveState with only what a test cares about set. */
function state(over: Partial<LiveState>): LiveState {
  return {
    connected: true,
    seenBoard: true,
    broker: null,
    bounds: {},
    advancedAt: {},
    reconnects: 0,
    missed: 0,
    venues: new Map(),
    tick: 0,
    ...over,
  }
}

const SECOND = 1_000_000

describe('durations', () => {
  it('measures time since arrival against the local clock only', () => {
    // **The one thing this clock can answer honestly.** Both values are ours.
    expect(sinceArrival(Date.now() - 5_000)).toBe(5)
    expect(sinceArrival(Date.now())).toBe(0)
  })

  it('never reports a negative elapsed time', () => {
    // A stamp from the future is a clock that moved, not time running
    // backwards; clamping is right here because both ends are local.
    expect(sinceArrival(Date.now() + 10_000)).toBe(0)
  })

  it("subtracts the capture's own two stamps, and ADDS local elapsed time", () => {
    // **Not a substitution.** The panel this replaced used `Date.now()` minus
    // arrival for the whole figure, which reports the viewer's network rather
    // than the venue's silence.
    const observed = 1_000 * SECOND
    const lastRecv = 940 * SECOND
    // Sixty seconds of measured silence, and the row arrived just now.
    expect(heardAgo(observed, lastRecv, Date.now())).toBe(60)
    // Ten seconds later, it is seventy — the measured part is unchanged.
    expect(heardAgo(observed, lastRecv, Date.now() - 10_000)).toBe(70)
  })

  it('has no answer where the capture gave it no stamps', () => {
    // Null, not zero: "heard just now" is a claim, and there is no evidence.
    expect(heardAgo(null, 940 * SECOND, Date.now())).toBeNull()
    expect(heardAgo(1_000 * SECOND, null, Date.now())).toBeNull()
    expect(heardAgo(undefined, undefined, Date.now())).toBeNull()
  })

  it('crosses clocks deliberately for lag, and does NOT clamp it', () => {
    // **It can be negative**, meaning the venue claims data from the future,
    // and that is shown rather than hidden: it is the finding the two fields
    // exist to produce.
    expect(venueLag(1_000 * SECOND, 995 * SECOND)).toBe(5)
    expect(venueLag(1_000 * SECOND, 1_005 * SECOND)).toBe(-5)
    expect(venueLag(null, 995 * SECOND)).toBeNull()
  })
})

describe('why the screen is silent', () => {
  // These four were ONE sentence — "No venue has published status yet" —
  // which is true of exactly one of them. During an outage it is a
  // reassuring falsehood, and it cost an afternoon of looking at the capture
  // instead of the broker. Each gets its own test for that reason.

  it('says nothing when venues are publishing', () => {
    const venues = new Map([['hyperliquid', { venue: 'hyperliquid', body: {}, received_ms: 0 }]])
    const why = whySilent(state({ venues }))
    expect(why.case).toBe('publishing')
    expect(silenceReason(why)).toBe('')
  })

  it('distinguishes our stream not being up yet', () => {
    const why = whySilent(state({ seenBoard: false }))
    expect(why.case).toBe('no-stream')
    expect(silenceReason(why)).toMatch(/stream/i)
  })

  it('distinguishes the tower having no broker, and says how hard it tried', () => {
    const why = whySilent(
      state({ broker: { connected: false, attempts: 7, refusal: 'connection refused' } }),
    )
    expect(why.case).toBe('no-broker')
    const said = silenceReason(why)
    expect(said).toMatch(/no broker/i)
    expect(said).toContain('7 attempts')
    expect(said).toContain('connection refused')
    // Counted, never judged: no threshold, no verdict.
    expect(said).not.toMatch(/too many|critical|unhealthy/i)
  })

  it('omits a count of zero rather than printing one that misreads', () => {
    // Zero attempts means the connection WAS established and dropped, and the
    // count belongs to the client reconnecting rather than to us. "0 attempts"
    // would read as "it has not tried".
    const why = whySilent(
      state({ broker: { connected: false, attempts: 0, refusal: 'the connection dropped' } }),
    )
    expect(silenceReason(why)).not.toContain('0 attempt')
    expect(silenceReason(why)).toMatch(/keeps trying/i)
  })

  it('distinguishes a broker with nothing published — the only true case', () => {
    const why = whySilent(state({ broker: { connected: true, attempts: 0, refusal: null } }))
    expect(why.case).toBe('nothing-published')
    expect(silenceReason(why)).toMatch(/no venue has published/i)
  })

  it('gives every state a different sentence', () => {
    const said = [
      whySilent(state({ seenBoard: false })),
      whySilent(state({ broker: { connected: false, attempts: 3, refusal: null } })),
      whySilent(state({ broker: { connected: true, attempts: 0, refusal: null } })),
    ].map(silenceReason)
    expect(new Set(said).size).toBe(said.length)
  })
})
