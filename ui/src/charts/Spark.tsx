import type { components } from '../contract/api'
import { dec, plot } from '../contract/money'

type Bar = components['schemas']['Bar']

/** Closes as polyline points in a `w`×`h` box. */
export function sparkPoints(bars: readonly Bar[], w: number, h: number): string {
  const closes = bars.map((b) => plot(dec(b.close)) ?? 0)
  if (closes.length < 2) return ''
  const lo = Math.min(...closes)
  const hi = Math.max(...closes)
  const span = hi - lo || 1
  return closes
    .map((c, i) => `${((i * w) / (closes.length - 1)).toFixed(1)},${(h - 2 - ((c - lo) / span) * (h - 4)).toFixed(1)}`)
    .join(' ')
}

export default function Spark({ bars, color }: { bars: readonly Bar[]; color: string }) {
  return (
    <svg width="100%" height="48" viewBox="0 0 200 48" preserveAspectRatio="none" aria-hidden="true">
      <polyline
        points={sparkPoints(bars, 200, 48)}
        fill="none"
        stroke={color}
        strokeWidth="1.5"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  )
}
