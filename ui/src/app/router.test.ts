import { describe, expect, it } from 'vitest'

import { href, parse } from './router'

describe('parse', () => {
  it('reads each view', () => {
    expect(parse('#/')).toEqual({ view: 'overview' })
    expect(parse('#/record')).toEqual({ view: 'record' })
    expect(parse('#/m/hyperliquid/BTC')).toEqual({ view: 'markets', venue: 'hyperliquid', ticker: 'BTC' })
  })

  it('falls back to the Overview for anything else', () => {
    expect(parse('')).toEqual({ view: 'overview' })
    expect(parse('#/nowhere')).toEqual({ view: 'overview' })
  })

  it('leaves the instrument open when the fragment does not name one', () => {
    expect(parse('#/m')).toEqual({ view: 'markets', venue: null, ticker: null })
  })

  it('round-trips through href', () => {
    const route = { view: 'markets', venue: 'hyperliquid', ticker: 'xyz:XYZ100' } as const
    expect(parse(href(route))).toEqual(route)
  })
})
