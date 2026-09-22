import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import App from './App'
import './index.css'

// The record changes when a capture process writes, which is often; but a
// listing walks the store, so this asks on an interval rather than on every
// focus, and says so.
const queryClient = new QueryClient({
  defaultOptions: { queries: { refetchOnWindowFocus: false, staleTime: 5_000 } },
})

const root = document.getElementById('root')
if (!root) throw new Error('index.html has no #root; the screen has nowhere to mount')

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
