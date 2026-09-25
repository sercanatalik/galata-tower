/** What a panel knows about its read. The order of the four is the decision. */
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

/** Which of the four a panel is in. What counts as empty is the caller's. */
export function panelState<T>(read: Read<T>, isEmpty: (data: T) => boolean): PanelState<T> {
  // First: a failed read must never be reported as an empty record.
  if (read.error !== null && read.error !== undefined) {
    return { kind: 'refused', error: read.error }
  }
  // A disabled query stays here; the caller says why it asked for nothing.
  if (read.isPending || read.data === undefined) {
    return { kind: 'reading' }
  }
  return isEmpty(read.data) ? { kind: 'empty', data: read.data } : { kind: 'ready', data: read.data }
}
