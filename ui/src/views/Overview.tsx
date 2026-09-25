import Decimal from 'decimal.js'

import { href } from '../app/router'
import Boundary from '../Boundary'
import Spark from '../charts/Spark'
import type { components } from '../contract/api'
import { dec, fmt, type Money } from '../contract/money'
import { useBoard, useCandles, useLatest, useTimeline } from '../data/queries'
import { body } from '../data/truth'
import { count, day, forHumans, stamp } from '../kit/format'
import { Head, Panel } from '../kit/Panel'
import { useLiveStatus } from '../live/status'

type Price = components['schemas']['Price']
type Bar = components['schemas']['Bar']
type Timeline = components['schemas']['Timeline']

/** The price to show: the last trade, else the mid. */
export function shown(p: Price): Money | null {
  const last = dec(p.last)
  if (last) return last
  const bid = dec(p.bid)
  const ask = dec(p.ask)
  return bid && ask ? bid.plus(ask).div(2) : (bid ?? ask)
}

/** Decimal places that suit a price's size. */
export function places(m: Money | null): number {
  if (!m) return 2
  const a = m.abs()
  return a.gte(1000) ? 0 : a.gte(100) ? 2 : a.gte(1) ? 3 : 5
}

/** First open to last close, as a signed percentage. */
export function change(bars: readonly Bar[]): { text: string; up: boolean } | null {
  if (bars.length === 0) return null
  const open = dec(bars[0].open)
  const close = dec(bars[bars.length - 1].close)
  if (!open || !close || open.isZero()) return null
  const pct = close.minus(open).div(open).mul(100)
  return { text: `${pct.isNegative() ? '−' : '+'}${pct.abs().toFixed(2)}%`, up: !pct.isNegative() }
}

function Tile({ venue, price }: { venue: string; price: Price }) {
  const candles = useCandles(venue, price.ticker, '1h')
  const bars = candles.data?.bars ?? []
  const move = change(bars)
  const now = shown(price)
  return (
    <a className="tile" href={href({ view: 'markets', venue, ticker: price.ticker })}>
      <div className="row">
        <span className="ticker">{price.ticker}</span>
        {move ? <span className={`mono ${move.up ? 'up' : 'down'}`} style={{ fontSize: 12 }}>{move.text}</span> : null}
      </div>
      <div className="price">{now ? fmt(now, places(now)) : '—'}</div>
      {bars.length > 1 ? <Spark bars={bars} color={move?.up === false ? 'var(--down)' : 'var(--up)'} /> : <div style={{ height: 48 }} />}
      <div className="foot mono">
        {price.quote_recv_micros ? `archive · ${stamp(price.quote_recv_micros, true).replace(/^\d+ \w+ /, '')}` : 'no price in window'}
      </div>
    </a>
  )
}

function MarketTiles() {
  const read = useLatest()
  return (
    <section className="section" aria-labelledby="mk">
      <Head title="Markets" id="mk">
        price: newest quote or trade in the archive · change and sparkline: the tape's candles, 1h
      </Head>
      <Panel
        what="The latest prices"
        read={read}
        bare
        isEmpty={(d) => d.venues.length === 0}
        empty={
          <>
            <strong>No venue could be priced</strong>
            {(read.data?.refusals ?? []).map((r) => <span key={r}>{r}</span>)}
          </>
        }
      >
        {(d) => (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            {d.venues.map((v) => (
              <div key={v.venue} className="tiles">
                {v.prices.map((p) => <Tile key={p.ticker} venue={v.venue} price={p} />)}
              </div>
            ))}
            {d.refusals.map((r) => <span key={r} className="muted" style={{ fontSize: 12 }}>{r}</span>)}
          </div>
        )}
      </Panel>
    </section>
  )
}

function Datasets() {
  const read = useBoard()
  const live = useLiveStatus()
  return (
    <section className="section" aria-labelledby="ds">
      <Head title="Datasets" id="ds">tape rows · archive segments today · live messages · newest day covered</Head>
      <Panel what="The board" read={read} isEmpty={(d) => d.cells.length === 0} empty={<strong>The tape holds no dataset yet</strong>}>
        {(d) => {
          const venues = [...new Set(d.cells.map((c) => c.venue))]
          const kinds = [...new Set(d.cells.map((c) => c.kind))]
          return (
            <div className="board" style={{ gridTemplateColumns: `180px repeat(${kinds.length}, minmax(150px, 1fr))` }}>
              <div className="venue eyebrow">venue</div>
              {kinds.map((k) => <div key={k} className="eyebrow">{k}</div>)}
              {venues.map((venue) => {
                const b = body(live.venues.get(venue))
                const msgs = new Map<string, number>()
                for (const p of b.pairs ?? []) msgs.set(p.series ?? '', (msgs.get(p.series ?? '') ?? 0) + (p.count ?? 0))
                const tickers = new Set((b.pairs ?? []).map((p) => p.ticker)).size
                return [
                  <div key={venue} className="venue" style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                    <span style={{ fontSize: 15, fontWeight: 600 }}>{venue}</span>
                    <span className="muted" style={{ fontSize: 12 }}>
                      {b.subs_declared ? `${tickers} tickers · ${b.subs_held} of ${b.subs_declared} subscriptions held` : 'no live status'}
                    </span>
                  </div>,
                  ...kinds.map((kind) => {
                    const c = d.cells.find((x) => x.venue === venue && x.kind === kind)
                    if (!c) return <div key={kind} className="muted">—</div>
                    const cov = c.window_micros > 0 ? new Decimal(c.covered_micros).div(c.window_micros) : null
                    return (
                      <div key={kind} style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                        <div className="line"><span>in tape</span><span className="mono" style={{ fontSize: 15 }}>{count(c.tape_rows)}</span></div>
                        <div className="line"><span>archive today</span><span className="mono">{c.archive_today === null || c.archive_today === undefined ? 'derived' : `${count(c.archive_today)} seg`}</span></div>
                        <div className="line"><span>live, {b.count_window_secs ?? '?'} s</span><span className="mono">{msgs.has(kind) ? count(msgs.get(kind)) : '—'}</span></div>
                        {kind === 'gaps' ? (
                          <span className="mono muted" style={{ fontSize: 11 }}>{count(c.gap_rows)} gap rows</span>
                        ) : (
                          <>
                            <div className="meter"><div style={{ width: `${cov ? cov.mul(100).toFixed(1) : 0}%` }} /></div>
                            <span className="mono muted" style={{ fontSize: 11 }}>
                              {c.coverage_date ? `${day(c.coverage_date)} · ${cov ? cov.mul(100).toFixed(1) : '0'}% covered` : 'no day covered'}
                              {c.gap_rows ? ` · ${c.gap_rows} gap rows` : ''}
                            </span>
                          </>
                        )}
                      </div>
                    )
                  }),
                ]
              })}
            </div>
          )
        }}
      </Panel>
    </section>
  )
}

const W = 1360
const LABEL = 132
const LANE = 22
const STEP = 34
const DAY = 86_400_000_000

const CAUSE_FILL: Record<string, string> = { downtime: 'url(#hatch)', crash_unflushed: 'var(--crash)' }

/** The span the timeline draws: whole UTC days around everything it holds. */
export function extent(t: Timeline): [number, number] | null {
  const all = [
    ...t.lanes.flatMap((l) => [...l.held, ...l.gaps, ...(l.archive_only ? [l.archive_only] : [])]),
    ...t.archive.flatMap((a) => a.spans),
  ]
  if (all.length === 0) return null
  const lo = Math.min(...all.map((s) => s.from))
  const hi = Math.max(...all.map((s) => s.to))
  return [lo - (lo % DAY), hi - (hi % DAY) + DAY]
}

function TimelineChart({ t }: { t: Timeline }) {
  const span = extent(t)
  if (!span) return null
  const [lo, hi] = span
  const x = (m: number) => LABEL + ((m - lo) / (hi - lo)) * (W - LABEL - 8)
  const w = (a: number, b: number) => Math.max(3, x(b) - x(a))
  const days = Array.from({ length: Math.round((hi - lo) / DAY) }, (_, i) => lo + i * DAY)
  const newest = Math.max(...t.archive.flatMap((a) => a.spans.map((s) => s.to)), 0)
  const rows: Array<{ archive: Timeline['archive'][number] } | { lane: Timeline['lanes'][number] }> = [
    ...t.archive.map((a) => ({ archive: a })),
    ...t.lanes.map((l) => ({ lane: l })),
  ]
  const height = 30 + rows.length * STEP + 40
  const legendY = height - 18

  return (
    <svg width="100%" viewBox={`0 0 ${W} ${height}`} role="img" aria-label="The record over time, by venue and dataset">
      <defs>
        <pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <rect width="6" height="6" fill="#1b1f23" />
          <line x1="0" y1="0" x2="0" y2="6" stroke="#5e625f" strokeWidth="2" />
        </pattern>
      </defs>
      {days.map((d) => (
        <g key={d}>
          <line x1={x(d)} y1={16} x2={x(d)} y2={legendY - 16} stroke="#262b30" />
          <text x={x(d) + 6} y={12} fill="#a29e94" fontSize="11" fontFamily="var(--mono)">{stamp(d).split(' ').slice(0, 2).join(' ')}</text>
        </g>
      ))}
      {rows.map((row, i) => {
        const y = 30 + i * STEP
        if ('archive' in row) {
          const a = row.archive
          return (
            <g key={`a-${a.venue}`}>
              <text x={0} y={y + 11} fill="#ece8df" fontSize="12" fontWeight="600" fontFamily="var(--sans)">archive</text>
              <text x={0} y={y + 24} fill="#a29e94" fontSize="10.5" fontFamily="var(--sans)">{a.venue}</text>
              <rect x={LABEL} y={y} width={W - LABEL - 8} height={LANE} fill="#111316" />
              {a.spans.map((s) => <rect key={s.from} x={x(s.from)} y={y} width={w(s.from, s.to)} height={LANE} fill="#ece8df" />)}
            </g>
          )
        }
        const l = row.lane
        return (
          <g key={`${l.venue}-${l.kind}`}>
            <text x={0} y={y + 15} fill="#ece8df" fontSize="12" fontWeight="500" fontFamily="var(--sans)">{l.kind}</text>
            <rect x={LABEL} y={y} width={W - LABEL - 8} height={LANE} fill="#111316" />
            {l.held.map((s) => <rect key={`h${s.from}`} x={x(s.from)} y={y} width={w(s.from, s.to)} height={LANE} fill="#ece8df" />)}
            {l.gaps.map((g) => (
              <rect key={`g${g.cause}${g.from}`} x={x(g.from)} y={y} width={w(g.from, g.to)} height={LANE} fill={CAUSE_FILL[g.cause] ?? '#6e6b64'}>
                <title>{`${g.cause} · ${forHumans(g.to - g.from)} · ${stamp(g.from)} → ${stamp(g.to)}`}</title>
              </rect>
            ))}
            {l.backfilled.map((s) => (
              <rect key={`b${s.from}`} x={x(s.from)} y={y} width={w(s.from, s.to)} height={LANE} fill="#6aa9e0" fillOpacity="0.28">
                <title>{`backfilled by the venue · ${stamp(s.from)} → ${stamp(s.to)}`}</title>
              </rect>
            ))}
            {l.archive_only ? (
              <rect x={x(l.archive_only.from)} y={y + 0.5} width={w(l.archive_only.from, l.archive_only.to)} height={LANE - 1} fill="none" stroke="#6e6b64" strokeDasharray="3 3">
                <title>{`in the archive, not yet in the tape · ${forHumans(l.archive_only.to - l.archive_only.from)}`}</title>
              </rect>
            ) : null}
          </g>
        )
      })}
      {newest > 0 ? (
        <g>
          <line x1={x(newest)} y1={20} x2={x(newest)} y2={legendY - 16} stroke="var(--amber)" strokeWidth="1.5" />
          <text x={x(newest) > W - 200 ? x(newest) - 6 : x(newest) + 6} y={28} textAnchor={x(newest) > W - 200 ? 'end' : 'start'} fill="var(--amber)" fontSize="11" fontFamily="var(--mono)">newest arrival {stamp(newest).slice(-9, -4)}</text>
        </g>
      ) : null}
      <g fontSize="11.5" fontFamily="var(--sans)" fill="#a29e94">
        <rect x={LABEL} y={legendY - 10} width="12" height="12" fill="#ece8df" /><text x={LABEL + 18} y={legendY}>held</text>
        <rect x={LABEL + 76} y={legendY - 10} width="12" height="12" fill="url(#hatch)" /><text x={LABEL + 94} y={legendY}>downtime</text>
        <rect x={LABEL + 178} y={legendY - 10} width="12" height="12" fill="var(--crash)" /><text x={LABEL + 196} y={legendY}>crash_unflushed</text>
        <rect x={LABEL + 320} y={legendY - 10} width="12" height="12" fill="#6aa9e0" fillOpacity="0.5" /><text x={LABEL + 338} y={legendY}>backfilled by venue</text>
        <rect x={LABEL + 484} y={legendY - 10} width="12" height="12" fill="none" stroke="#6e6b64" strokeDasharray="3 3" /><text x={LABEL + 502} y={legendY}>in the archive, not yet in the tape</text>
      </g>
    </svg>
  )
}

function RecordTimeline() {
  const read = useTimeline()
  return (
    <section className="section" aria-labelledby="tl">
      <Head title="The record over time" id="tl">held rows and recorded gaps by cause · UTC · a gap is never inferred from silence</Head>
      <Panel what="The timeline" read={read} isEmpty={(t) => t.lanes.length === 0 && t.archive.length === 0} empty={<strong>Neither root holds anything yet</strong>}>
        {(t) => (
          <div style={{ padding: '18px 20px 12px' }}>
            <TimelineChart t={t} />
          </div>
        )}
      </Panel>
    </section>
  )
}

export default function Overview() {
  return (
    <>
      <Boundary name="Markets"><MarketTiles /></Boundary>
      <Boundary name="Datasets"><Datasets /></Boundary>
      <Boundary name="The record over time"><RecordTimeline /></Boundary>
    </>
  )
}
