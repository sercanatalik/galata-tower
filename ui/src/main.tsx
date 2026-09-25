import '@fontsource-variable/instrument-sans'
import '@fontsource-variable/jetbrains-mono'

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import App from './App'
import './index.css'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The stream says when data moved; nothing refetches on focus or on a timer.
      refetchOnWindowFocus: false,
      staleTime: 5_000,
      // The API is this page's own origin; the default mode can pause a query as pending for ever.
      networkMode: 'always',
      // The tower's refusals are deterministic, so a retry only delays the honest answer.
      retry: false,
    },
  },
})

const root = document.getElementById('root')
if (!root) throw new Error('index.html has no #root; the screen has nowhere to mount')

createRoot(root, {
  onUncaughtError: (thrown, info) => console.error('[galata-tower] a throw escaped every boundary', thrown, info),
  onCaughtError: (thrown, info) => console.error('[galata-tower] a section failed and was contained', thrown, info),
}).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
