import { CandlestickSeries, type Time } from 'lightweight-charts'
import { useEffect, useMemo, useState } from 'react'

import { useChart } from './charts/useChart'
import { $api } from './contract/client'
import { panelState } from './panel'
import { dec, plot } from './contract/money'
import { useRecordAdvances } from './live/status'

/**
 * Rows are not candles.
 *
 * The tape republishes a candle as it forms — measured at **7,109 rows for 234
 * candles**, about thirty to one — so the row cap that sizes a table starves a
 * chart. The window is the control here, and the limit is set well above what a
 * window of this length can hold; `total` says if even that was capped.
 */
const CANDLE_LIMIT = 20_000

/**
 * The whole tape, with the cap doing the selecting.
 *
 * **Not "the last hour of wall clock."** That is what this asked for first, and
 * against a tape thirty-three hours old it drew nothing — correctly, which is
 * why it was easy to miss. A chart wants the NEWEST candles the tape holds, and
 * "newest" is a count, not a period: the route returns the newest rows when the
 * cap bites, so a wide window and a cap give exactly that, on a live tape and a
 * historical one alike.
 */
const WHOLE_TAPE = { from: 0, to: 9_000_000_000_000_000 }

type Row = Record<string, unknown>

/**
 * One ticker's candles, charted.
 *
 * **`is_final` is not consulted.** Not one of the 7,109 rows in this tape is
 * final, so filtering on it draws nothing — and it is the wrong question
 * anyway: a minute chart shows the minute as last seen, not only the minutes
 * that have closed.
 */
export default function Candles() {
  const { container, chart } = useChart(220)
  // The same rule as the tape table: told when the record moves, and refetched
  // then. A chart that stopped an hour ago looks exactly like a live one.
  const advanced = useRecordAdvances('candles')
  const [ticker, setTicker] = useState<string | null>(null)

  // **The record names its instruments; this does not read rows to find
  // them.** Until 2026-09-23 the first read named no ticker, because the list
  // came from the response — 11,932 rows and 4,092,528 bytes to learn six
  // names, on every load and every advance. `/v1/instruments` answers that
  // question directly, and the panel showing it has already fetched this, so
  // TanStack serves both from one query.
  const { data: held } = $api.useQuery('get', '/v1/instruments')
  const tickers = useMemo(
    () =>
      [
        ...new Set(
          (held?.instruments ?? []).filter((i) => i.kind === 'candles').map((i) => i.ticker),
        ),
      ].sort(),
    [held],
  )
  // Chosen from what the RECORD holds rather than hardcoded: a tape from
  // another venue has none of these six. The list does not change when the
  // choice does, because it never came from the filtered read.
  const chosen = ticker && tickers.includes(ticker) ? ticker : (tickers[0] ?? null)

  // **Waits for what narrows it** — TanStack's documented dependent query. No
  // unfiltered read is ever issued, where before one fired and was corrected.
  const read = $api.useQuery(
    'get',
    '/v1/tape/{kind}',
    {
      params: {
        path: { kind: 'candles' },
        query: { ...WHOLE_TAPE, limit: CANDLE_LIMIT, ...(chosen ? { ticker: chosen } : {}) },
      },
    },
    { enabled: !!chosen },
  )
  const state = panelState(read, (d) => d.rows.length === 0)
  const { data, error } = read
  const rows = (data?.rows ?? []) as Row[]

  const candles = useMemo(() => {
    if (!chosen) return []
    // **Last row wins.** A later row is a later state of the same forming
    // candle — its close moved, its high may have. Keeping the first would draw
    // the bar as it opened and never update it. A Map gives this for free,
    // because a later set replaces an earlier one and the rows arrive in
    // stream order.
    const byTime = new Map<number, { time: Time; open: number; high: number; low: number; close: number }>()
    for (const r of rows) {
      if (String(r.ticker) !== chosen) continue
      const at = r.at_micros as number
      const time = Math.floor(at / 1_000_000) as unknown as Time
      byTime.set(at, {
        time,
        // `plot` is the one float this screen may make, and `money.ts` is the
        // only file allowed to make it.
        open: plot(dec(r.open as string))!,
        high: plot(dec(r.high as string))!,
        low: plot(dec(r.low as string))!,
        close: plot(dec(r.close as string))!,
      })
    }
    // Ascending, and not for tidiness: the library binary-searches its data, so
    // out-of-order points measurably slow first paint.
    return [...byTime.values()].sort((a, b) => (a.time as number) - (b.time as number))
  }, [rows, chosen])

  useEffect(() => {
    if (!chart || candles.length === 0) return
    const series = chart.addSeries(CandlestickSeries, {
      upColor: '#16a34a',
      downColor: '#dc2626',
      borderVisible: false,
      wickUpColor: '#16a34a',
      wickDownColor: '#dc2626',
    })
    series.setData(candles)
    chart.timeScale().fitContent()
    return () => {
      chart.removeSeries(series)
    }
  }, [chart, candles])

  if (error) {
    return (
      <section>
        <h2>Candles</h2>
        <div className="refusal">
          <strong>The candles could not be read.</strong>
          <p>{String(error)}</p>
        </div>
      </section>
    )
  }

  return (
    <section>
      <h2>
        Candles{' '}
        <span className="count">
          {data ? `${candles.length} of ${rows.length} rows` : '…'}
        </span>
      </h2>
      <p className="muted">
        {tickers.length > 0 ? (
          <select value={chosen ?? ''} onChange={(e) => setTicker(e.target.value)}>
            {tickers.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        ) : null}{' '}
        · the newest the tape holds · a candle is republished as it forms, so rows outnumber candles
        {' · '}
        {advanced === null
          ? 'the record has not moved since this page loaded'
          : `advanced ${advanced}s ago`}
      </p>
      {/* **It had no reading case**, so an outstanding read showed a heading
          and an empty chart with nothing to say why. */}
      {state.kind === 'reading' ? <p className="muted">reading the candles…</p> : null}
      {state.kind === 'empty' ? <p className="muted">No candles in that window.</p> : null}
      <div ref={container} style={{ height: 220 }} />
    </section>
  )
}
