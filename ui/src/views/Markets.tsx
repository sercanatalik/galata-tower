import { useState } from 'react'

import { href } from '../app/router'
import Boundary from '../Boundary'
import CandleChart from '../charts/CandleChart'
import { dec, fmt } from '../contract/money'
import { TAPE_ROWS, useCandles, useInstruments, useLatest, useQuotes } from '../data/queries'
import { body } from '../data/truth'
import { clock, count, stamp } from '../kit/format'
import { Panel } from '../kit/Panel'
import { heardAgo, useLiveStatus, venueLag } from '../live/status'
import { change, places, shown } from './Overview'

const INTERVALS = ['1m', '5m', '15m', '1h'] as const
type Interval = (typeof INTERVALS)[number]

function Rail({ venue, ticker }: { venue: string; ticker: string }) {
  const latest = useLatest()
  const prices = latest.data?.venues.find((v) => v.venue === venue)?.prices ?? []
  return (
    <aside className="rail" aria-label="Instruments">
      <div className="eyebrow" style={{ padding: '0 12px 8px' }}>{venue} · {prices.length}</div>
      {prices.map((p) => {
        const now = shown(p)
        return (
          <a key={p.ticker} href={href({ view: 'markets', venue, ticker: p.ticker })} aria-current={p.ticker === ticker ? 'page' : undefined}>
            <span className="t">{p.ticker}</span>
            <span className="p">{now ? fmt(now, places(now)) : '—'}</span>
            <span className="s">{p.quote_recv_micros ? `archive ${clock(p.quote_recv_micros).slice(0, 8)}` : 'no price'}</span>
            <span />
          </a>
        )
      })}
    </aside>
  )
}

function Header({ venue, ticker, interval, onInterval }: { venue: string; ticker: string; interval: Interval; onInterval: (i: Interval) => void }) {
  const latest = useLatest()
  const price = latest.data?.venues.find((v) => v.venue === venue)?.prices.find((p) => p.ticker === ticker)
  const now = price ? shown(price) : null
  const bid = dec(price?.bid)
  const ask = dec(price?.ask)
  return (
    <div style={{ display: 'flex', alignItems: 'flex-end', gap: 24, flexWrap: 'wrap' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 12 }}>
          <h1 style={{ fontSize: 28, fontWeight: 700, letterSpacing: '-0.02em' }}>{ticker}</h1>
          <span className="muted" style={{ fontSize: 14 }}>{venue}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 14 }}>
          <span className="head-price">{now ? fmt(now, places(now)) : '—'}</span>
          {bid && ask ? (
            <span className="mono muted" style={{ fontSize: 13 }}>
              {fmt(bid, places(bid))} / {fmt(ask, places(ask))}
            </span>
          ) : null}
        </div>
        <span className="mono muted" style={{ fontSize: 12 }}>
          {price?.quote_recv_micros ? `newest in the archive · arrived ${stamp(price.quote_recv_micros, true)}` : 'no quote in the archive window'}
        </span>
      </div>
      <div className="segmented" role="group" aria-label="Interval">
        {INTERVALS.map((i) => (
          <button key={i} type="button" aria-pressed={i === interval} onClick={() => onInterval(i)}>
            {i}
          </button>
        ))}
      </div>
    </div>
  )
}

function Chart({ venue, ticker, interval }: { venue: string; ticker: string; interval: Interval }) {
  const read = useCandles(venue, ticker, interval)
  return (
    <Panel what="The candles" read={read} isEmpty={(d) => d.bars.length === 0} empty={<strong>The tape holds no candles for {ticker}</strong>}>
      {(d) => {
        const backfilled = d.bars.filter((b) => b.backfilled).length
        const move = change(d.bars)
        return (
          <div style={{ position: 'relative', padding: '14px 8px 6px 14px' }}>
            {backfilled > 0 ? (
              <div className="chart-note">
                <strong>{backfilled} of {d.bars.length} bars arrived by a backfill</strong>
                <span className="muted">The venue sent them on reconnect. The record holds no quotes or trades for the hatched span.</span>
              </div>
            ) : null}
            <CandleChart bars={d.bars} height={440} />
            <div className="mono muted" style={{ fontSize: 11.5, padding: '6px 0 4px' }}>
              from the tape · {stamp(d.bars[0].at_micros)} → {stamp(d.bars[d.bars.length - 1].at_micros)}
              {move ? <span className={move.up ? 'up' : 'down'}> · {move.text}</span> : null}
              {d.capped ? ' · capped to the newest 5,000 bars' : ''}
            </div>
          </div>
        )
      }}
    </Panel>
  )
}

function RecordFacts({ venue, ticker }: { venue: string; ticker: string }) {
  const read = useInstruments()
  return (
    <section className="card pad" aria-labelledby="rec" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between' }}>
        <h2 id="rec" className="eyebrow">From the record</h2>
        <span className="muted" style={{ fontSize: 11.5 }}>no broker needed</span>
      </div>
      <Panel what="The instruments" read={read} bare isEmpty={(d) => !d.instruments.some((i) => i.venue === venue && i.ticker === ticker)} empty={<span>The tape holds nothing for {ticker}.</span>}>
        {(d) => {
          const mine = d.instruments.filter((i) => i.venue === venue && i.ticker === ticker)
          const newest = Math.max(...mine.map((i) => i.last_micros))
          return (
            <>
              {mine.map((i) => (
                <div key={i.kind} className="kv"><span>{i.kind}</span><span className="mono">{count(i.rows)} rows</span></div>
              ))}
              <span className="mono muted" style={{ fontSize: 11.5 }}>newest venue time · {stamp(newest, true)}</span>
            </>
          )
        }}
      </Panel>
    </section>
  )
}

function LiveFacts({ venue, ticker }: { venue: string; ticker: string }) {
  const live = useLiveStatus()
  const v = live.venues.get(venue)
  const b = body(v)
  const pairs = (b.pairs ?? []).filter((p) => p.ticker === ticker)
  return (
    <section className="card pad" aria-labelledby="live" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between' }}>
        <h2 id="live" className="eyebrow">From the venue, live</h2>
        <span className="mono muted" style={{ fontSize: 11.5 }}>status.{venue}</span>
      </div>
      {pairs.length === 0 ? (
        <span className="muted" style={{ fontSize: 13 }}>
          {live.broker?.connected === false ? 'The bus is down, so the venue cannot be heard.' : `status.${venue} does not report ${ticker}.`}
        </span>
      ) : (
        <table className="grid">
          <thead>
            <tr><th>series</th><th>state</th><th className="num">msgs</th><th className="num">heard</th><th className="num">behind</th></tr>
          </thead>
          <tbody>
            {pairs.map((p) => {
              const heard = v ? heardAgo(b.observed_at_micros, p.last_recv_micros, v.received_ms) : null
              const behind = venueLag(p.last_recv_micros, p.last_event_micros)
              return (
                <tr key={p.series}>
                  <td>{p.series}</td>
                  <td>{p.state}</td>
                  <td className="num">{count(p.count)}</td>
                  <td className="num">{heard === null ? '—' : `${heard} s`}</td>
                  <td className="num">{behind === null ? '—' : `${behind} s`}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      )}
      <span className="muted" style={{ fontSize: 11.5 }}>
        msgs in the capture's {b.count_window_secs ?? '?'} s window · behind: venue time to receipt, by the capture's clock
      </span>
    </section>
  )
}

function TapeQuotes({ ticker }: { ticker: string }) {
  const read = useQuotes(ticker)
  return (
    <section className="card pad" aria-labelledby="tp" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between' }}>
        <h2 id="tp" className="eyebrow">Tape · quotes</h2>
        <span className="mono muted" style={{ fontSize: 11.5 }}>
          newest {TAPE_ROWS}
          {read.data ? ` of ${count(read.data.total)}` : ''}
        </span>
      </div>
      <Panel what="The tape" read={read} bare isEmpty={(d) => d.rows.length === 0} empty={<span>{read.data?.written === false ? 'The tape has never written quotes.' : 'No quotes for this instrument.'}</span>}>
        {(d) => (
          <table className="grid">
            <thead>
              <tr><th>venue time</th><th className="num">bid</th><th className="num">ask</th><th className="num">ask size</th><th className="num">latency</th></tr>
            </thead>
            <tbody>
              {[...d.rows].reverse().map((row, i) => {
                const r = row as Record<string, string | number | null>
                const bid = dec(r.bid_px as string | null)
                const at = r.at_micros as number | null
                const recv = r.recv_micros as number | null
                return (
                  <tr key={`${r.stream_seq}-${i}`}>
                    <td className="mono">{clock(at)}</td>
                    <td className="num">{fmt(bid, places(bid))}</td>
                    <td className="num">{fmt(dec(r.ask_px as string | null), places(bid))}</td>
                    <td className="num">{fmt(dec(r.ask_sz as string | null), 4)}</td>
                    <td className="num muted">{at && recv ? `${Math.round((recv - at) / 1000)} ms` : '—'}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </Panel>
    </section>
  )
}

export default function Markets({ venue, ticker }: { venue: string | null; ticker: string | null }) {
  const [interval, setBarWidth] = useState<Interval>('15m')
  const latest = useLatest()
  const first = latest.data?.venues[0]
  const v = venue ?? first?.venue ?? null
  const t = ticker ?? first?.prices[0]?.ticker ?? null

  if (!v || !t) {
    return (
      <Panel what="The latest prices" read={latest} isEmpty={() => true} empty={<><strong>No instrument to show</strong>{(latest.data?.refusals ?? []).map((r) => <span key={r}>{r}</span>)}</>}>
        {() => null}
      </Panel>
    )
  }

  return (
    <div className="markets">
      <Boundary name="Instruments"><Rail venue={v} ticker={t} /></Boundary>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 20, minWidth: 0 }}>
        <Boundary name="The price"><Header venue={v} ticker={t} interval={interval} onInterval={setBarWidth} /></Boundary>
        <Boundary name="The chart"><Chart venue={v} ticker={t} interval={interval} /></Boundary>
        <div className="three">
          <Boundary name="From the record"><RecordFacts venue={v} ticker={t} /></Boundary>
          <Boundary name="From the venue"><LiveFacts venue={v} ticker={t} /></Boundary>
          <Boundary name="Tape"><TapeQuotes ticker={t} /></Boundary>
        </div>
      </div>
    </div>
  )
}
