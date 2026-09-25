import { Component, type ErrorInfo, type ReactNode } from 'react'

/** What a caught throw says, whatever was thrown. */
export function describe(thrown: unknown): string {
  if (thrown instanceof Error) return thrown.message || thrown.name
  if (typeof thrown === 'string' && thrown !== '') return thrown
  return `a ${typeof thrown} was thrown: ${String(thrown)}`
}

interface Props {
  /** The section's name, for the refusal. */
  name: string
  children: ReactNode
}

interface State {
  /** Separate from `thrown`, because a falsy throw is still a throw. */
  caught: boolean
  thrown: unknown
}

/** Keeps one section's failure in that section. */
export default class Boundary extends Component<Props, State> {
  state: State = { caught: false, thrown: null }

  static getDerivedStateFromError(thrown: unknown): State {
    return { caught: true, thrown }
  }

  componentDidCatch(thrown: unknown, info: ErrorInfo) {
    console.error(`[galata-tower] ${this.props.name} failed while rendering`, thrown, info)
  }

  render() {
    if (!this.state.caught) return this.props.children
    return (
      <div className="card">
        <div className="state refused" role="alert">
          <div>
            <strong>{this.props.name} could not be drawn</strong>
            <code>{describe(this.state.thrown)}</code>
            <span>The rest of the screen reads other things and is unaffected.</span>
          </div>
        </div>
      </div>
    )
  }
}
