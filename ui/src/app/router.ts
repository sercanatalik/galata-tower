import { useSyncExternalStore } from 'react'

/** Where the screen is. The fragment is the whole of it, so every view survives a reload. */
export type Route =
  | { view: 'overview' }
  | { view: 'markets'; venue: string | null; ticker: string | null }
  | { view: 'record' }

/** A fragment as a route. Anything unrecognised is the Overview. */
export function parse(hash: string): Route {
  const parts = hash.replace(/^#\/?/, '').split('/').filter(Boolean).map(decodeURIComponent)
  if (parts[0] === 'record') return { view: 'record' }
  if (parts[0] === 'm') return { view: 'markets', venue: parts[1] ?? null, ticker: parts[2] ?? null }
  return { view: 'overview' }
}

/** A route as the fragment that reaches it. */
export function href(route: Route): string {
  switch (route.view) {
    case 'overview':
      return '#/'
    case 'record':
      return '#/record'
    case 'markets':
      return route.venue && route.ticker
        ? `#/m/${encodeURIComponent(route.venue)}/${encodeURIComponent(route.ticker)}`
        : '#/m'
  }
}

function subscribe(listener: () => void) {
  window.addEventListener('hashchange', listener)
  return () => window.removeEventListener('hashchange', listener)
}

export function useRoute(): Route {
  const hash = useSyncExternalStore(subscribe, () => window.location.hash, () => '')
  return parse(hash)
}
