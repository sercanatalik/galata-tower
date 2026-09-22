import { describe as suite, expect, it } from 'vitest'

import Boundary, { describe } from './Boundary'

/**
 * What a boundary DECIDES, held without a DOM.
 *
 * Whether the fallback appears on screen is a browser question, and it is how
 * this change's premise was established: a throw planted in one panel left the
 * page with no text content at all. What is held here is the two decisions a
 * test can hold honestly — that everything thrown counts as caught, and that
 * the reader is told what it was.
 */
suite('a boundary catches whatever was thrown', () => {
  it('counts an ordinary Error as caught', () => {
    const next = Boundary.getDerivedStateFromError(new Error('a malformed decimal'))
    expect(next.caught).toBe(true)
  })

  it('counts a FALSY throw as caught — the case that would undo this', () => {
    // **A boundary that decided by the caught value's truthiness** would
    // re-render the throwing child here, and React escalates past a boundary
    // that does that, unmounting the whole tree. That is the failure this
    // component exists to prevent, reached through its own fix.
    for (const falsy of [undefined, null, 0, '', false, NaN]) {
      expect(Boundary.getDerivedStateFromError(falsy).caught).toBe(true)
    }
  })

  it('starts clean, so an unfailed panel renders its children', () => {
    // The default state must not read as caught, or every panel renders a
    // refusal for a failure that never happened.
    expect(new Boundary({ name: 'Tape', children: null }).state.caught).toBe(false)
  })
})

suite('what the refusal says', () => {
  it("uses an Error's message", () => {
    expect(describe(new Error('the tape could not be read'))).toBe('the tape could not be read')
  })

  it('falls back to the name where the message is empty', () => {
    // `new Error('')` is legal and its message is blank; a refusal reading
    // "This panel could not be drawn." and then nothing says less than the
    // class name does.
    expect(describe(new Error(''))).toBe('Error')
  })

  it('passes a thrown string through', () => {
    expect(describe('oops')).toBe('oops')
  })

  it('describes a throw that is not an Error at all', () => {
    // **Says what it can rather than nothing.** `undefined` here is the same
    // value that would have taken the tree down.
    expect(describe(undefined)).toContain('undefined')
    expect(describe(null)).toContain('null')
    expect(describe(42)).toContain('42')
    expect(describe({ code: 7 })).toContain('object')
  })

  it('never returns an empty description', () => {
    for (const thrown of [undefined, null, 0, '', false, NaN, new Error('')]) {
      expect(describe(thrown).length).toBeGreaterThan(0)
    }
  })
})
