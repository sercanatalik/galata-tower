/**
 * What a panel knows about its read.
 *
 * **Refused, reading, empty, ready — in that order, and the order is the
 * decision.** A query that has errored is not still reading, and a query that
 * is reading has no emptiness to report.
 *
 * Both defects this replaced came from getting that order wrong by omission.
 * `Instruments` never asked about `error`, so an errored read fell through to
 * *empty* — and *empty* carried a confident explanation. With `/v1/instruments`
 * returning 500 the panel said *"The record holds no instrument… an empty table
 * here means an empty or unreadable tape root"*, every clause of which was
 * false, and which is worse than a blank panel because it names a cause a
 * reader will go and check.
 *
 * Seven panels each made this choice by hand; two were missing branches. Copies
 * of a decision are correct until the next copy is written.
 */
export type PanelState<T> =
  /** The read failed. Nothing is known about the data. */
  | { kind: 'refused'; error: unknown }
  /** The read has neither succeeded nor failed. */
  | { kind: 'reading' }
  /** The read succeeded and holds nothing to show. */
  | { kind: 'empty'; data: T }
  /** The read succeeded and there is something to draw. */
  | { kind: 'ready'; data: T }

/** What a query hands back, narrowed to what this decision needs. */
export interface Read<T> {
  data: T | undefined
  error: unknown
  isPending: boolean
}

/**
 * Which of the four a panel is in.
 *
 * `isEmpty` is the caller's: *nothing to show* means no rows here, no causes
 * there, no instruments elsewhere. This decides which state applies, never
 * what counts as empty.
 */
export function panelState<T>(read: Read<T>, isEmpty: (data: T) => boolean): PanelState<T> {
  // **First, because an errored read is not a reading one** — and because
  // falling past this is what let a 500 be reported as an empty record.
  if (read.error !== null && read.error !== undefined) {
    return { kind: 'refused', error: read.error }
  }
  // Before emptiness: a read still in flight has no emptiness to report. A
  // panel that says "nothing here" while the answer is on its way is guessing.
  //
  // **A DISABLED query lands here too, and stays.** TanStack reports a query
  // waiting on `enabled` as pending forever, so a panel with a dependent read
  // will say "reading…" about a read it never issued — which `Candles` did,
  // against a tape holding no candles. That is not a fifth state: the caller
  // knows WHY it asked for nothing and can say so in its own words, which is
  // more use than anything this function could return.
  //
  // TanStack documents both halves of this: a disabled query with no cached
  // data starts at `status: 'pending'` with `fetchStatus: 'idle'`, and
  // "you likely cannot use the isPending flag to show a loading spinner" —
  // `isLoading`, which is `isFetching && isPending`, is the flag meant for
  // that. It is not used here because `isLoading` is false for a disabled
  // query AND for one that has never been asked, and this function would
  // still have to call the second of those something.
  if (read.isPending || read.data === undefined) {
    return { kind: 'reading' }
  }
  return isEmpty(read.data) ? { kind: 'empty', data: read.data } : { kind: 'ready', data: read.data }
}
