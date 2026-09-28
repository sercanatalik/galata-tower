import { ColorType, createChart, type IChartApi, type ISeriesApi, LineSeries } from 'lightweight-charts'
import { useEffect, useRef } from 'react'

import { type ChartPoint } from '../data/signals'

/**
 * One stored signal's history as a line. An absent figure is a whitespace point,
 * a gap in the line and never a value, so a hole in the record shows as one.
 * Styled as the Markets chart, and in the ink colour: the board is not
 * colour-judged.
 */
export default function SignalChart({ points, height }: { points: readonly ChartPoint[]; height: number }) {
  const container = useRef<HTMLDivElement | null>(null)
  const parts = useRef<{ chart: IChartApi; line: ISeriesApi<'Line'> } | null>(null)

  useEffect(() => {
    const el = container.current
    if (!el) return
    const chart = createChart(el, {
      autoSize: true,
      height,
      layout: {
        background: { type: ColorType.Solid, color: 'transparent' },
        textColor: '#a29e94',
        fontFamily: "'JetBrains Mono Variable', ui-monospace, monospace",
        fontSize: 11,
        attributionLogo: false,
      },
      grid: { vertLines: { color: '#1c2024' }, horzLines: { color: '#20252a' } },
      rightPriceScale: { borderVisible: false },
      timeScale: { borderVisible: false, timeVisible: true, secondsVisible: false },
    })
    const line = chart.addSeries(LineSeries, { color: '#d8d4ca', lineWidth: 1, priceLineVisible: false })
    parts.current = { chart, line }
    return () => {
      parts.current = null
      chart.remove()
    }
  }, [height])

  useEffect(() => {
    const p = parts.current
    if (!p) return
    p.line.setData([...points])
    p.chart.timeScale().fitContent()
  }, [points])

  return <div ref={container} style={{ height }} />
}
