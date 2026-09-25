import type { ReactNode } from 'react'

import { panelState, type Read } from '../panel'

/** The server's refusal, as it said it. */
export function said(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error)
  } catch {
    return String(error)
  }
}

export function Refusal({ what, error }: { what: string; error: unknown }) {
  return (
    <div className="state refused" role="alert">
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="var(--down)" strokeWidth="1.7" aria-hidden="true">
        <circle cx="10" cy="10" r="8" />
        <path d="M10 5.5v5.5M10 14v.5" />
      </svg>
      <div>
        <strong>{what} could not be read</strong>
        <code>{said(error)}</code>
        <span>The read failed, so nothing is known. This is not an empty record.</span>
      </div>
    </div>
  )
}

function Reading() {
  return (
    <div className="state" aria-busy="true">
      <span className="mono" style={{ fontSize: 11.5 }}>reading…</span>
      <div className="skeleton" style={{ width: '92%' }} />
      <div className="skeleton" style={{ width: '78%' }} />
      <div className="skeleton" style={{ width: '85%' }} />
    </div>
  )
}

interface Props<T> {
  /** What is being read, for the refusal. */
  what: string
  read: Read<T>
  isEmpty: (data: T) => boolean
  /** What emptiness means here. Shown only after a successful read. */
  empty: ReactNode
  /** Drawn only when the read succeeded and holds something. */
  children: (data: T) => ReactNode
  /** Draw without the card around it. */
  bare?: boolean
}

/** Every panel's four states. A view writes only what it draws when ready. */
export function Panel<T>({ what, read, isEmpty, empty, children, bare }: Props<T>) {
  const state = panelState(read, isEmpty)
  const body =
    state.kind === 'refused' ? (
      <Refusal what={what} error={state.error} />
    ) : state.kind === 'reading' ? (
      <Reading />
    ) : state.kind === 'empty' ? (
      <div className="state">{empty}</div>
    ) : (
      children(state.data)
    )
  if (bare && state.kind === 'ready') return <>{body}</>
  return <div className="card">{body}</div>
}

/** A section heading with its meta line. */
export function Head({ title, id, children }: { title: string; id: string; children?: ReactNode }) {
  return (
    <div className="section-head">
      <h2 id={id}>{title}</h2>
      {children ? <span className="meta">{children}</span> : null}
    </div>
  )
}
