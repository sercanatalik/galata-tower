import {
  CandlestickSeries,
  ColorType,
  createChart,
  CrosshairMode,
  HistogramSeries,
  type IChartApi,
  type ISeriesApi,
  type Time,
  type UTCTimestamp,
} from 'lightweight-charts'
import { useEffect, useMemo, useRef } from 'react'

import type { components } from '../contract/api'
import { dec, plot } from '../contract/money'
import { BackfillBand, type Band } from './BackfillBand'

type Bar = components['schemas']['Bar']

const UP = '#6aa9e0'
const DOWN = '#e8843f'

const seconds = (micros: number) => Math.floor(micros / 1_000_000) as UTCTimestamp

/** Runs of backfilled bars, each from its first bar to the bar after its last. */
export function backfilledRuns(bars: readonly Bar[]): Band[] {
  const out: Band[] = []
  let start: number | null = null
  bars.forEach((bar, i) => {
    if (bar.backfilled && start === null) start = i
    const ends = start !== null && (!bar.backfilled || i === bars.length - 1)
    if (ends && start !== null) {
      const last = bar.backfilled ? i : i - 1
      const to = bars[Math.min(last + 1, bars.length - 1)]
      out.push({ from: seconds(bars[start].at_micros) as Time, to: seconds(to.at_micros) as Time })
      start = null
    }
  })
  return out
}

interface Parts {
  chart: IChartApi
  candles: ISeriesApi<'Candlestick'>
  volume: ISeriesApi<'Histogram'>
  band: BackfillBand
}

/** Candles with volume beneath, and backfilled spans hatched behind them. */
export default function CandleChart({ bars, height }: { bars: readonly Bar[]; height: number }) {
  const container = useRef<HTMLDivElement | null>(null)
  const parts = useRef<Parts | null>(null)

  // One owner: the chart, its series and its primitive live and die together. `remove()` disposes them all,
  // so nothing touches a series after its chart is gone.
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
      crosshair: {
        mode: CrosshairMode.Normal,
        vertLine: { color: '#6e6b64', labelBackgroundColor: '#262b30' },
        horzLine: { color: '#6e6b64', labelBackgroundColor: '#262b30' },
      },
    })
    const candles = chart.addSeries(CandlestickSeries, {
      upColor: UP,
      downColor: DOWN,
      borderVisible: false,
      wickUpColor: UP,
      wickDownColor: DOWN,
      priceLineColor: UP,
      priceLineStyle: 3,
    })
    candles.priceScale().applyOptions({ scaleMargins: { top: 0.08, bottom: 0.22 } })
    const volume = chart.addSeries(HistogramSeries, {
      priceScaleId: 'volume',
      priceFormat: { type: 'volume' },
      lastValueVisible: false,
      priceLineVisible: false,
    })
    chart.priceScale('volume').applyOptions({ scaleMargins: { top: 0.84, bottom: 0 } })
    const band = new BackfillBand()
    candles.attachPrimitive(band)
    parts.current = { chart, candles, volume, band }
    return () => {
      parts.current = null
      chart.remove()
    }
  }, [height])

  const data = useMemo(
    () =>
      bars.map((b) => {
        const open = plot(dec(b.open)) ?? 0
        const close = plot(dec(b.close)) ?? 0
        return {
          candle: { time: seconds(b.at_micros), open, high: plot(dec(b.high)) ?? 0, low: plot(dec(b.low)) ?? 0, close },
          volume: {
            time: seconds(b.at_micros),
            value: plot(dec(b.volume)) ?? 0,
            color: close >= open ? 'rgba(106,169,224,0.45)' : 'rgba(232,132,63,0.45)',
          },
        }
      }),
    [bars],
  )

  useEffect(() => {
    const p = parts.current
    if (!p) return
    p.candles.setData(data.map((d) => d.candle))
    p.volume.setData(data.map((d) => d.volume))
    p.band.set(backfilledRuns(bars))
    p.chart.timeScale().fitContent()
  }, [data, bars, height])

  return <div ref={container} style={{ height }} />
}
