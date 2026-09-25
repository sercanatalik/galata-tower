import Boundary from '../Boundary'
import type { components } from '../contract/api'
import { useAbout, useBoard, useFailures, useGaps, usePartitions, useRates } from '../data/queries'
import { count, day, forHumans, stamp } from '../kit/format'
import { Head, Panel } from '../kit/Panel'

type Partition = components['schemas']['Partition']

export interface Cell {
  segments: number
  state: 'open' | 'holding' | 'compacted'
}

/** Partitions as venue → kind → date → cell. A partition is open on the tower's today. */
export function grid(partitions: readonly Partition[], today: string) {
  const venues = new Map<string, Map<string, Map<string, Cell>>>()
  const dates = new Set<string>()
  for (const p of partitions) {
    const levels = Object.fromEntries(p.path.split('/').map((part) => part.split('=') as [string, string]))
    const { venue, kind, date } = levels
    if (!venue || !kind || !date) continue
    dates.add(date)
    const kinds = venues.get(venue) ?? new Map()
    venues.set(venue, kinds)
    const days = kinds.get(kind) ?? new Map()
    kinds.set(kind, days)
    days.set(date, {
      segments: p.segments,
      state: date >= today ? 'open' : p.segments > 1 ? 'holding' : 'compacted',
    })
  }
  return { venues, dates: spanOf([...dates]) }
}

/** Every date from the first to the last, so a day with no partition is shown as one. */
export function spanOf(dates: string[]): string[] {
  if (dates.length === 0) return []
  const sorted = [...dates].sort()
  const out: string[] = []
  const end = new Date(`${sorted[sorted.length - 1]}T00:00:00Z`)
  for (let d = new Date(`${sorted[0]}T00:00:00Z`); d <= end; d.setUTCDate(d.getUTCDate() + 1)) {
    out.push(d.toISOString().slice(0, 10))
  }
  return out
}

const shade = (n: number) => `rgba(236,232,223,${Math.min(0.32, 0.05 + Math.log10(n + 1) * 0.068).toFixed(2)})`
const STATE = { open: 'open · today', holding: 'closed · holding', compacted: 'compacted' }

function Partitions() {
  const read = usePartitions()
  const board = useBoard()
  const today = board.data?.today ?? new Date().toISOString().slice(0, 10)
  return (
    <section className="section" aria-labelledby="pt">
      <Head title="Partitions" id="pt">{read.data ? `${read.data.length} partitions · segments per kind and day` : 'segments per kind and day'}</Head>
      <Panel what="The partitions" read={read} isEmpty={(d) => d.length === 0} empty={<strong>The archive holds no partition</strong>}>
        {(d) => {
          const { venues, dates } = grid(d, today)
          return (
            <div style={{ padding: 8, overflowX: 'auto' }}>
              {[...venues].map(([venue, kinds]) => (
                <table key={venue} className="parts" aria-label={`${venue} partitions`}>
                  <thead>
                    <tr>
                      <th className="eyebrow" style={{ textAlign: 'left', padding: '6px 8px' }}>{venue}</th>
                      {dates.map((date) => <th key={date} className="eyebrow" style={{ textAlign: 'left', padding: '6px 8px' }}>{day(date)}</th>)}
                    </tr>
                  </thead>
                  <tbody>
                    {[...kinds].map(([kind, days]) => (
                      <tr key={kind}>
                        <th scope="row" className="mono" style={{ textAlign: 'left', fontSize: 12.5, fontWeight: 400, padding: '0 8px', whiteSpace: 'nowrap' }}>{kind}</th>
                        {dates.map((date) => {
                          const cell = days.get(date)
                          if (!cell) {
                            return <td key={date} className="none"><div className="n">—</div><div className="s">no partition</div></td>
                          }
                          return (
                            <td key={date} className={cell.state === 'open' ? 'open' : undefined} style={{ background: shade(cell.segments) }}>
                              <div className="n">{count(cell.segments)}</div>
                              <div className="s">{STATE[cell.state]}</div>
                            </td>
                          )
                        })}
                      </tr>
                    ))}
                  </tbody>
                </table>
              ))}
            </div>
          )
        }}
      </Panel>
      <p className="muted" style={{ fontSize: 12.5 }}>
        Shade follows the segment count and nothing else. Compaction leaves one segment per closed day; how many is too many is yours to judge.
      </p>
    </section>
  )
}

function Rates() {
  const read = useRates()
  return (
    <section className="section" aria-labelledby="rt">
      <Head title="Rows per hour" id="rt">from the tape · where a feed that stays up but thins out shows</Head>
      <Panel what="The rates" read={read} isEmpty={(d) => d.buckets.length === 0} empty={<strong>The tape holds no rows yet</strong>}>
        {(d) => {
          const hours = new Map<string, typeof d.buckets>()
          for (const b of d.buckets) {
            const key = `${b.venue}|${b.hour_micros}`
            hours.set(key, [...(hours.get(key) ?? []), b])
          }
          const newest = [...hours.entries()].sort((a, b) => b[1][0].hour_micros - a[1][0].hour_micros).slice(0, 6)
          return (
            <div className="pad" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 28 }}>
              {newest.map(([key, buckets]) => {
                const max = Math.max(...buckets.map((b) => b.rows))
                return (
                  <div key={key} style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                    <span className="mono" style={{ fontSize: 12.5 }}>
                      {buckets[0].venue} · {stamp(buckets[0].hour_micros)}
                    </span>
                    {[...buckets].sort((a, b) => b.rows - a.rows).map((b) => (
                      <div key={b.kind} className="bars">
                        <span className="muted">{b.kind}</span>
                        <div className="track"><div style={{ width: `${Math.max(1, Math.round((100 * b.rows) / max))}%` }} /></div>
                        <span className="mono" style={{ textAlign: 'right' }}>{count(b.rows)}</span>
                      </div>
                    ))}
                  </div>
                )
              })}
            </div>
          )
        }}
      </Panel>
      {read.data?.capped ? <p className="muted" style={{ fontSize: 12 }}>Capped to the newest hours.</p> : null}
    </section>
  )
}

const SWATCH: Record<string, string> = { downtime: 'var(--hatch)', crash_unflushed: 'var(--crash)' }

function Gaps() {
  const read = useGaps()
  return (
    <section className="section" aria-labelledby="gp">
      <Head title="Gaps by cause" id="gp">as the capture wrote them</Head>
      <Panel what="The gaps" read={read} isEmpty={(d) => d.causes.length === 0} empty={<><strong>The record states no gap</strong><span>Absent rows are not a gap; only what the capture wrote down is.</span></>}>
        {(d) => (
          <>
            {d.causes.map((c) => (
              <div key={c.cause} className="pad" style={{ borderBottom: '1px solid var(--rule)', display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <span style={{ width: 12, height: 12, borderRadius: 2, background: SWATCH[c.cause] ?? 'var(--faint)' }} />
                  <span className="mono" style={{ fontSize: 14, fontWeight: 500 }}>{c.cause}</span>
                  <span className="mono" style={{ fontSize: 18, marginLeft: 'auto' }}>{forHumans(c.missing_micros)}</span>
                </div>
                <div className="muted" style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', fontSize: 12 }}>
                  <span><span className="mono" style={{ color: 'var(--text)' }}>{c.intervals}</span> intervals</span>
                  <span><span className="mono" style={{ color: 'var(--text)' }}>{count(c.rows)}</span> rows</span>
                  <span><span className="mono" style={{ color: 'var(--text)' }}>{c.tickers}</span> tickers</span>
                </div>
                <span className="mono muted" style={{ fontSize: 11.5 }}>
                  {stamp(c.first_micros)} → {stamp(c.last_micros)} · {c.series.join(', ')}
                </span>
              </div>
            ))}
            <p className="pad muted" style={{ fontSize: 12 }}>
              Durations are the union of a cause's intervals. Rows show breadth: one outage is written once per ticker and series.
            </p>
          </>
        )}
      </Panel>
    </section>
  )
}

function Failures() {
  const read = useFailures()
  return (
    <section className="section" aria-labelledby="fl">
      <Head title="Parse failures" id="fl">{read.data ? `${read.data.with_failures} of ${read.data.partitions} partitions` : ''}</Head>
      <Panel
        what="The failures"
        read={read}
        isEmpty={(d) => d.failing.length === 0}
        empty={
          read.data?.partitions ? (
            <><strong>None of {read.data.partitions} partitions holds a payload that failed to parse</strong><span>Read from the archive, so today's partitions are included.</span></>
          ) : (
            <><strong>There are no partitions to read</strong><span>Nothing is claimed about failures; the truth bar says whether the archive is there.</span></>
          )
        }
      >
        {(d) => (
          <table className="grid" style={{ margin: 16, width: 'calc(100% - 32px)' }}>
            <thead><tr><th>where</th><th>error</th><th className="num">count</th><th>last</th></tr></thead>
            <tbody>
              {d.failing.map((f) => (
                <tr key={`${f.venue}${f.kind}${f.channel}${f.error}`}>
                  <td className="mono">{f.venue}/{f.kind}/{f.channel}</td>
                  <td>{f.error}</td>
                  <td className="num">{count(f.failures)}</td>
                  <td className="mono">{stamp(f.last_micros)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>
    </section>
  )
}

function Roots() {
  const read = useAbout()
  return (
    <section className="section" aria-labelledby="ro">
      <h2 id="ro" className="eyebrow">Roots</h2>
      <Panel what="The roots" read={read} isEmpty={() => false} empty={null}>
        {(a) => (
          <div className="pad mono" style={{ fontSize: 12, display: 'flex', flexDirection: 'column', gap: 4 }}>
            <div className="kv"><span>{a.archive.var}</span><span>{a.archive.path}</span></div>
            <div className="kv"><span>{a.tape.var}</span><span>{a.tape.path}</span></div>
            <div className="kv"><span>prunes on</span><span>{a.prune_on.join(' · ')}</span></div>
          </div>
        )}
      </Panel>
    </section>
  )
}

export default function Record() {
  return (
    <div className="record">
      <div style={{ display: 'flex', flexDirection: 'column', gap: 32, minWidth: 0 }}>
        <Boundary name="Partitions"><Partitions /></Boundary>
        <Boundary name="Rows per hour"><Rates /></Boundary>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 32, minWidth: 0 }}>
        <Boundary name="Gaps"><Gaps /></Boundary>
        <Boundary name="Failures"><Failures /></Boundary>
        <Boundary name="Roots"><Roots /></Boundary>
      </div>
    </div>
  )
}
