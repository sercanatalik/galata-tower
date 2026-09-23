import { describe, expect, it } from 'vitest'

import { panelState } from './panel'

const never = () => false
const always = () => true

/**
 * **Four states from three inputs, and two of these rows are shipped
 * defects.** Both came from a panel making this choice by hand and omitting a
 * case — which is what seven hand-written copies of one decision produce.
 */
describe('what a panel knows', () => {
  it('reports a failed read as refused', () => {
    const state = panelState({ data: undefined, error: 'boom', isPending: false }, never)
    expect(state.kind).toBe('refused')
    expect(state).toMatchObject({ error: 'boom' })
  })

  it('does NOT report a failed read as empty — the Instruments defect', () => {
    // With /v1/instruments returning 500 the panel said "The record holds no
    // instrument… an empty table here means an empty or unreadable tape root".
    // Every clause false, and worse than a blank because it names a cause.
    // An error wins even when the emptiness test would say empty.
    const state = panelState({ data: undefined, error: new Error('500'), isPending: false }, always)
    expect(state.kind).toBe('refused')
  })

  it('does NOT report a failed read as reading', () => {
    // The other half of the ordering: a query that errored is not still in
    // flight, however the flags happen to be set.
    const state = panelState({ data: undefined, error: 'boom', isPending: true }, never)
    expect(state.kind).toBe('refused')
  })

  it('reports a read in flight as reading — the Candles defect', () => {
    // Candles had no pending case at all, so it showed a heading and nothing
    // else while its read was outstanding.
    expect(panelState({ data: undefined, error: null, isPending: true }, never).kind).toBe('reading')
  })

  it('does NOT report a read in flight as empty', () => {
    // A panel saying "nothing here" while the answer is on its way is
    // guessing, and it guesses wrong exactly when the record is large.
    expect(panelState({ data: undefined, error: null, isPending: true }, always).kind).toBe(
      'reading',
    )
  })

  it('treats absent data as still reading even when nothing says pending', () => {
    // Defensive: the settled-but-dataless state is how a paused query looked
    // for a whole day before networkMode was fixed.
    expect(panelState({ data: undefined, error: null, isPending: false }, never).kind).toBe(
      'reading',
    )
  })

  it('reports an empty success as empty, carrying the data', () => {
    const state = panelState({ data: { rows: [] }, error: null, isPending: false }, (d) =>
      d.rows.length === 0,
    )
    expect(state.kind).toBe('empty')
    expect(state).toMatchObject({ data: { rows: [] } })
  })

  it('reports a non-empty success as ready', () => {
    const state = panelState({ data: { rows: [1] }, error: null, isPending: false }, (d) =>
      d.rows.length === 0,
    )
    expect(state.kind).toBe('ready')
    expect(state).toMatchObject({ data: { rows: [1] } })
  })

  it('leaves what counts as empty to the caller', () => {
    const data = { rows: [1, 2] }
    const read = { data, error: null, isPending: false }
    expect(panelState(read, never).kind).toBe('ready')
    expect(panelState(read, always).kind).toBe('empty')
  })
})
