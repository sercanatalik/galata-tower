import { Component, type ErrorInfo, type ReactNode } from 'react'

/**
 * What a caught throw should say.
 *
 * Separated from the component so it can be held by a test without a DOM:
 * the decisions here are about what a reader is told, and those are the part
 * worth holding.
 */
export function describe(thrown: unknown): string {
  if (thrown instanceof Error) return thrown.message || thrown.name
  // **A throw need not be an Error.** `throw 'oops'` and `throw undefined` are
  // both legal, and the second is the one that matters — see `caught` below.
  if (typeof thrown === 'string' && thrown !== '') return thrown
  return `a ${typeof thrown} was thrown: ${String(thrown)}`
}

interface Props {
  /** The panel's name, for the refusal. */
  name: string
  children: ReactNode
}

interface State {
  /**
   * **A boolean, not the value's truthiness.**
   *
   * A boundary that renders its fallback only when the caught value is truthy
   * re-renders the throwing child on `throw undefined` — and React escalates
   * past the boundary and unmounts the whole tree, which is precisely the
   * failure this component exists to prevent, reached through its own fix.
   */
  caught: boolean
  thrown: unknown
}

/**
 * One panel's failure stays in that panel.
 *
 * **Measured, not assumed.** A throw planted in the gaps panel, built and
 * served, left the page with no text content at all: the archive root, the
 * partitions, the overdue days, the live status, the tape and the chart, all
 * gone because a sibling threw. There was no boundary anywhere in `ui/`.
 *
 * That is a worse failure than the one it came from. The tower already refuses
 * to let an absent broker stop it serving the record — *the record is a fact
 * on disk and does not need a bus to be true* — and the screen was throwing
 * that away at the first bad value.
 *
 * Per panel rather than one at the top: the partition listing has nothing to
 * do with the tape's decimals, and there is no reason for one to hide the
 * other. The panels answer different questions from different routes, and the
 * boundary follows that seam.
 *
 * **No retry.** A render that threw throws again on the same data, so a button
 * that re-renders it is a button that does nothing. When the record advances
 * the query refetches and the panel remounts with different data, which is the
 * only thing that could actually help.
 */
export default class Boundary extends Component<Props, State> {
  state: State = { caught: false, thrown: null }

  /**
   * Pure, and called during render — which is why the console line is in
   * `componentDidCatch` and not here. A side effect in this method is how a
   * boundary starts behaving differently under concurrent rendering.
   */
  static getDerivedStateFromError(thrown: unknown): State {
    return { caught: true, thrown }
  }

  componentDidCatch(thrown: unknown, info: ErrorInfo) {
    // Said out loud. A panel that quietly renders a refusal every time is a
    // panel somebody should notice.
    console.error(`[galata-tower] ${this.props.name} failed while rendering`, thrown, info)
  }

  render() {
    if (!this.state.caught) return this.props.children
    return (
      <section>
        <h2>{this.props.name}</h2>
        <div className="refusal">
          <strong>This panel could not be drawn.</strong>
          <p>{describe(this.state.thrown)}</p>
          <p className="muted">
            The rest of the screen is unaffected: these panels read different
            things, and one being wrong says nothing about the others.
          </p>
        </div>
      </section>
    )
  }
}
