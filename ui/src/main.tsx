import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import App from './App'
import './index.css'

// The record changes when a capture process writes, which is often; but a
// listing walks the store, so this asks on an interval rather than on every
// focus, and says so.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      staleTime: 5_000,

      // **The API is the same origin as this page.** TanStack's default
      // network mode pauses a query's retry whenever its online manager
      // believes the browser is offline — and a paused query stays `pending`
      // for ever with no error, which renders exactly like a first load.
      //
      // Every panel here branches on `isPending` to say "reading…", so that
      // left all six refusal branches dead: planted a `400` in a route a panel
      // calls and the panel said "reading the gaps…" at nine seconds, at
      // seventy-three seconds, and after one request. The cache said
      // `status: "pending", fetchStatus: "paused", error: null`.
      //
      // The gate is also asking a question that cannot be relevant. This is
      // one binary serving its own screen, usually over loopback: there is no
      // network condition under which the page loads and its own API does not.
      // In the measured run `navigator.onLine` was even `true` while the query
      // sat paused.
      networkMode: 'always',

      // **A retry here is a guess that the answer will change.** The refusals
      // this tower produces are deterministic — a window that runs backwards,
      // a dataset it does not hold, a limit of zero — so three retries give
      // the same answer three times and postpone the honest one by seven
      // seconds. If the tower is down, saying so at once beats silence, and
      // the record advancing already refetches everything, so a transient
      // failure repairs itself on the next advance without a retry policy.
      retry: false,
    },
  },
})

const root = document.getElementById('root')
if (!root) throw new Error('index.html has no #root; the screen has nowhere to mount')

createRoot(root, {
  // **React 19's own hooks for this.** Without them an uncaught throw reaches
  // only `window.reportError`, and a caught one only `console.error` with
  // React's wording. Both are reported here in the screen's own words,
  // because the one thing this tree keeps relearning is that a failure nobody
  // is told about costs an afternoon.
  onUncaughtError: (thrown, info) => {
    console.error('[galata-tower] a throw escaped every boundary', thrown, info)
  },
  onCaughtError: (thrown, info) => {
    console.error('[galata-tower] a panel failed and was contained', thrown, info)
  },
}).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
