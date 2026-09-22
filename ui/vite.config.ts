import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// `dist/` is read by rust-embed at compile time, so the build output is part
// of the Rust build's input. Relative asset paths, because the binary serves
// the screen from whatever path the operator mounted it at.
export default defineConfig({
  plugins: [react()],
  base: './',
  build: { outDir: 'dist', emptyOutDir: true },
  // The API is the same origin in production -- one binary serves both. In dev
  // it is the running tower.
  server: { proxy: { '/v1': 'http://127.0.0.1:8777' } },
})
