// One lightweight-charts instance per panel, sized to its container, in the
// viewer's theme. The chart is created once per theme; series are attached by
// the panel and fed on every render.
//
// Carried from the predecessor's `components/charts/useChart.ts`, with one
// change: it took the theme from `next-themes`, a library this screen does not
// have, for a capability its CSS already implements. Carrying the dependency to
// carry the hook would have been carrying the wrong half.

import { useEffect, useRef, useState } from 'react'

import { ColorType, createChart, type IChartApi } from 'lightweight-charts'

/** Whether the viewer is in a dark scheme, and changes to it. */
function useDark(): boolean {
  const [dark, setDark] = useState(
    () => window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false,
  )
  useEffect(() => {
    const query = window.matchMedia?.('(prefers-color-scheme: dark)')
    if (!query) return
    const onChange = (e: MediaQueryListEvent) => setDark(e.matches)
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }, [])
  return dark
}

export function useChart(height?: number) {
  const container = useRef<HTMLDivElement | null>(null)
  const [chart, setChart] = useState<IChartApi | null>(null)
  const dark = useDark()

  // The chart is rebuilt when the theme flips; its series follow on the next render.
  useEffect(() => {
    const el = container.current
    if (!el) return
    const c = createChart(el, {
      autoSize: true,
      height,
      layout: {
        background: { type: ColorType.Solid, color: 'transparent' },
        textColor: dark ? '#9a9a9a' : '#666',
        fontSize: 10,
        attributionLogo: false,
      },
      grid: {
        vertLines: { color: dark ? '#2a2a2a' : '#e3e3e3' },
        horzLines: { color: dark ? '#2a2a2a' : '#e3e3e3' },
      },
      rightPriceScale: { borderVisible: false },
      timeScale: { borderVisible: false, timeVisible: true, secondsVisible: false },
      crosshair: { mode: 0 },
    })
    setChart(c)
    return () => {
      c.remove()
      setChart(null)
    }
  }, [dark, height])

  return { container, chart }
}
