import { describe, expect, it } from 'vitest'

import {
  afterBoard,
  afterDisconnect,
  afterLag,
  afterTapeMoved,
  heardAgo,
  isAReasonToResync,
  silenceReason,
  sinceArrival,
  venueLag,
  whySilent,
} from './status'
import type { LiveState } from './status'

/** A LiveState with only what a test cares about set. */
function state(over: Partial<LiveState>): LiveState {
  return {
    connected: true,
    seenBoard: true,
    broker: null,
    bounds: {},
    advancedAt: {},
    resyncs: 0,
    drops: 0,
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

  it('will not describe the tower while it cannot hear the tower', () => {
    // Observed on the real screen, 2026-09-23, by restarting the tower under a
    // live browser: it said "Live not connected" directly above "The tower is
    // subscribed; no venue has published yet." Both came from the same state
    // and only one could be known — `whySilent` asked whether a board had EVER
    // arrived, never whether the stream was up NOW, so it reported the last
    // board's broker as current.
    //
    // The broker here says connected, because that is what the last frame said
    // before the stream went. The answer must still be that we cannot see.
    const why = whySilent(
      state({ connected: false, seenBoard: true, broker: { connected: true, attempts: 0, refusal: null } }),
    )
    expect(why.case).toBe('stream-lost')
    const said = silenceReason(why)
    expect(said).toMatch(/dropped/i)
    // The two sentences that were wrong: it must claim neither.
    expect(said).not.toMatch(/subscribed/i)
    expect(said).not.toMatch(/no venue has published/i)
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

/**
 * What counts as a reason to distrust what the screen holds.
 *
 * **Three defects in two days lived in this decision**, each found by hand in
 * a browser: a predicate that matched only the tape routes, a signal taken
 * from the disconnection instead of the connection, and a guard reading one
 * value while the effect watched another. The answers below are what those
 * cost, kept as cases.
 */
describe('a reason to resync', () => {
  it('counts the record advancing', () => {
    expect(isAReasonToResync('advanced', 1)).toBe(true)
    // However many connections have happened — an advance stands alone.
    expect(isAReasonToResync('advanced', 0)).toBe(true)
  })

  it('counts a re-connection, because of what may have happened while away', () => {
    expect(isAReasonToResync('connected', 1)).toBe(true)
    expect(isAReasonToResync('connected', 7)).toBe(true)
  })

  it('does NOT count the first connection', () => {
    // The page has just read everything; refetching would undo that read.
    expect(isAReasonToResync('connected', 0)).toBe(false)
  })

  it('does NOT count a disconnection — the case that cost nine refusals', () => {
    // **The one that looks most like a reason.** The stream broke, so surely
    // something changed? But the tower is usually down at that moment, so the
    // refetch fails, and with retries off the failure is held. Every panel on
    // the screen showed a refusal until the page was reloaded.
    expect(isAReasonToResync('disconnected', 0)).toBe(false)
    expect(isAReasonToResync('disconnected', 5)).toBe(false)
  })

  it('is decided by the event, not by a count alone', () => {
    // Same count, opposite answers: the event is what carries the meaning.
    expect(isAReasonToResync('connected', 3)).toBe(true)
    expect(isAReasonToResync('disconnected', 3)).toBe(false)
  })
})

describe('the counters that only a real stream had ever exercised', () => {
  it('adds a lag to what was already missed, rather than replacing it', () => {
    // **The failure this is for** is not "the arithmetic is wrong" — it is a
    // refactor that captures `state` as a snapshot instead of reading the
    // module binding, after which every lag overwrites the last and the screen
    // reports the most recent gap as the session total.
    //
    // So the assertion is about the SECOND event, not the sum: a replacing
    // implementation also gets a single lag right.
    const once = afterLag(state({ missed: 0 }), 3)
    expect(once.missed).toBe(3)

    const twice = afterLag(state({ missed: 3 }), 4)
    expect(twice.missed).toBe(7)
    expect(twice.missed).not.toBe(4)
  })

  it('counts a drop once however many times the browser retries it', () => {
    // `EventSource` retries by itself, repeatedly, and every attempt raises
    // `error`. Watched live 2026-09-23: a tower killed for half a minute read
    // "1 drop" throughout.
    const dropped = afterDisconnect(state({ connected: true, drops: 0 }))
    expect(dropped).toEqual({ connected: false, drops: 1 })

    // Already down: the retry is not a new drop.
    const retried = afterDisconnect(state({ connected: false, drops: 1 }))
    expect(retried.connected).toBe(false)
    expect(retried.drops).toBeUndefined()

    // And ten more retries still leave it at one.
    let live = state({ connected: false, drops: 1 })
    for (let i = 0; i < 10; i += 1) live = { ...live, ...afterDisconnect(live) }
    expect(live.drops).toBe(1)
  })
})

describe('the record, per venue', () => {
  it('advances a kind when any one of its venues moves', () => {
    const live = state({ bounds: { quotes: { hyperliquid: 10, 'rh-crypto': 9_000_000 } } })
    const next = afterBoard(live, { quotes: { hyperliquid: 10, 'rh-crypto': 9_000_004 } }, 42)
    expect(next.advancedAt.quotes).toBe(42)
  })

  it('does not advance a kind whose every venue restates its position', () => {
    // A reconnect's frame restates where the record stands; it is not news.
    const live = state({ bounds: { quotes: { hyperliquid: 10, 'rh-crypto': 9_000_000 } } })
    const next = afterBoard(live, { quotes: { hyperliquid: 10, 'rh-crypto': 9_000_000 } }, 42)
    expect(next.advancedAt.quotes).toBeUndefined()
  })

  it("keeps the other venues' positions when one venue's stream moves", () => {
    const live = state({ bounds: { quotes: { hyperliquid: 10, 'rh-crypto': 9_000_000 } } })
    const next = afterTapeMoved(live, { kind: 'quotes', venue: 'rh-crypto', bound: 9_000_004 }, 7)
    expect(next.bounds.quotes).toEqual({ hyperliquid: 10, 'rh-crypto': 9_000_004 })
    expect(next.advancedAt.quotes).toBe(7)
  })
})
