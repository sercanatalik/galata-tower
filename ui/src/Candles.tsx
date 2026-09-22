import { CandlestickSeries, type Time } from 'lightweight-charts'
import { useEffect, useMemo, useState } from 'react'

import { useChart } from './charts/useChart'
import { $api } from './contract/client'
import { dec, plot } from './contract/money'
import { useRecordAdvances, useRemembered } from './live/status'

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

  // **Ask for the ticker being drawn.** This read 11,932 rows and kept the
  // 1,960 belonging to the chosen instrument — after the rest had crossed the
  // wire. The route filters now, so the discarded rows are never sent.
  //
  // The first read names no ticker, because the list of instruments comes
  // from what the window holds and there is nothing to choose from yet.
  const { data, error } = $api.useQuery('get', '/v1/tape/{kind}', {
    params: {
      path: { kind: 'candles' },
      query: { ...WHOLE_TAPE, limit: CANDLE_LIMIT, ...(ticker ? { ticker } : {}) },
    },
  })

  const rows = (data?.rows ?? []) as Row[]
  // **From the route, and remembered.** Once a ticker is chosen the read
  // matches only that one, so a list taken from the latest response alone
  // would collapse to a single entry with no way back.
  const tickers = useRemembered(data?.tickers)
  // Chosen from what the window holds rather than hardcoded: a tape from
  // another venue has none of these six.
  const chosen = ticker && tickers.includes(ticker) ? ticker : (tickers[0] ?? null)
  // **Adopt what is being drawn**, so the next read asks for it. The FIRST
  // read cannot: the instrument list comes from the response, so there is
  // nothing to name yet. That one read is the price of not hardcoding a list
  // of tickers; every read after it carries only the rows drawn.
  useEffect(() => {
    if (!ticker && chosen) setTicker(chosen)
  }, [ticker, chosen])

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
      {data && rows.length === 0 ? (
        <p className="muted">No candles in that window.</p>
      ) : null}
      <div ref={container} style={{ height: 220 }} />
    </section>
  )
}
