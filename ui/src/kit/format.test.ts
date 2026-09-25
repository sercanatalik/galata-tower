import { describe, expect, it } from 'vitest'

import { forHumans } from './format'

const SECOND = 1_000_000
const MINUTE = 60 * SECOND
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/**
 * How long the record says is missing.
 *
 * **The number this renders is the one the gaps route exists to get right.**
 * The union of intervals per cause was 117,308s of downtime where summing the
 * rows said 2,815,397s; a renderer that then misplaced a unit would undo that
 * work at the last step.
 */
describe('forHumans', () => {
  it('uses seconds below a minute', () => {
    expect(forHumans(30 * SECOND)).toBe('30s')
    expect(forHumans(59 * SECOND)).toBe('59s')
  })

  it('uses minutes below an hour', () => {
    expect(forHumans(2 * MINUTE)).toBe('2m')
    expect(forHumans(59 * MINUTE)).toBe('59m')
  })

  it('uses hours below a day', () => {
    expect(forHumans(2 * HOUR)).toBe('2.0h')
    expect(forHumans(90 * MINUTE)).toBe('1.5h')
  })

  it('uses days above one', () => {
    expect(forHumans(2 * DAY)).toBe('2.0d')
  })

  it('renders the real downtime this archive holds', () => {
    // 117,308s — the union figure the route reports, which the screen shows
    // as 1.4d. Summing the rows instead would have made this 32.6d, and the
    // point of the test is that this number is small.
    expect(forHumans(117_308 * SECOND)).toBe('1.4d')
  })

  it('does not round a gap away to nothing', () => {
    // **A sub-second gap is still a gap.** Rounding it to `0s` reads as "no
    // gap", which is the one thing this panel must never say when the record
    // says otherwise. It DID return `0s` until this test was written — with
    // a name that contradicted its own assertion, which is how it was found.
    expect(forHumans(400_000)).toBe('<1s')
    expect(forHumans(1)).toBe('<1s')
    // And a genuine zero is still zero: nothing missing says nothing missing.
    expect(forHumans(0)).toBe('0s')
  })
})
