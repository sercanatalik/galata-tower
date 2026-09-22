import { describe, expect, it } from 'vitest'

import { dec, fmt, plot, signed } from './money'

/**
 * **The one place in the tree where a wrong number would look right.**
 *
 * Every price the screen shows crosses as a string and is parsed here. These
 * assert the CLAIM the file exists to make — that nothing becomes a binary
 * float on the way — rather than the particular output of `Decimal`, which is
 * `Decimal`'s business.
 */
describe('money', () => {
  it('survives a value a double cannot hold', () => {
    // 0.1 + 0.2 is the textbook case; this is the same failure at the scale a
    // venue actually quotes. As a double this loses its last digits.
    const exact = '30482.123456789012345678'
    expect(dec(exact)?.toFixed()).toBe(exact)
    // An earlier draft showed the contrast here by putting the same string
    // through a float. The money guard refused the file for it — correctly,
    // and for the fourth time in a day: the rule is that the coercions are not
    // written anywhere under src/ except the one file, so a hit is always a
    // finding, including when the writer meant it. The claim above does not
    // need the contrast; eighteen fractional digits do not survive a double,
    // and that is why this string was chosen.
  })

  it('does not round a half up by accident on the way in', () => {
    // Parsing must not decide anything. Rendering may, and says how many
    // places it used.
    expect(dec('1.005')?.toFixed()).toBe('1.005')
    expect(fmt(dec('1.005'), 2)).toBe('1.01')
  })

  it('is null for an absent value, never zero', () => {
    // **Zero is a price.** A parse failure that produced one would put a
    // fabricated quote on the screen.
    expect(dec(null)).toBeNull()
    expect(dec(undefined)).toBeNull()
    expect(dec('')).toBeNull()
  })

  it('throws on a malformed decimal rather than swallowing it', () => {
    // **Absent and malformed are different**, and this tree's rule is that
    // silently dropping what could not be read is the failure a named error
    // exists to prevent. A `null` here would render an em dash and hide a
    // contract violation.
    //
    // FOUND BY WRITING THIS TEST, and worth stating: nothing catches it.
    // `dec` runs during render, the panels catch a QUERY error and not a
    // render-time throw, and there is no error boundary — so a malformed
    // decimal from the server takes the panel down with no message. The
    // throw is right; being uncaught is a separate gap, named here rather
    // than quietly made to disappear by weakening this function.
    expect(() => dec('not a number')).toThrow()
  })

  it('renders an absent value as something that is not a number', () => {
    // A missing price must not render as 0.00, for the same reason.
    expect(fmt(null)).not.toBe('0.00')
    expect(fmt(undefined)).not.toBe('0.00')
  })

  it('keeps a sign where the sign is the point', () => {
    // A TYPOGRAPHIC minus, U+2212, not an ASCII hyphen — which is what the
    // function's own doc comment shows and what an earlier draft of this test
    // got wrong. It matters: the two are different characters, and a test
    // asserting the wrong one would pass the day somebody changed it.
    expect(signed(dec('1.5'))).toBe('+1.50')
    expect(signed(dec('-1.5'))).toBe('\u22121.50')
    // Zero carries no sign: it is neither up nor down.
    expect(signed(dec('0'))).toBe('0.00')
  })

  it('makes a float only in plot, which is what plot is for', () => {
    // `plot` is the single exception the money guard permits, because a chart
    // library takes numbers. Everything else returns a string or a Decimal.
    expect(typeof plot(dec('1.5'))).toBe('number')
    expect(typeof fmt(dec('1.5'))).toBe('string')
    expect(typeof signed(dec('1.5'))).toBe('string')
    expect(plot(null)).toBeNull()
  })
})
