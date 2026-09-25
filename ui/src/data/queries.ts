import { $api } from '../contract/client'

/** The whole tape: every read of a summary is bounded by its causes or instruments, not its rows. */
export const WHOLE_TAPE = { from: 0, to: 9_000_000_000_000_000 }

/** Quotes shown under the chart. */
export const TAPE_ROWS = 12

export const useAbout = () => $api.useQuery('get', '/v1/about')
export const useLatest = () => $api.useQuery('get', '/v1/latest')
export const useBoard = () => $api.useQuery('get', '/v1/board')
export const useTimeline = () => $api.useQuery('get', '/v1/timeline')
export const useInstruments = () => $api.useQuery('get', '/v1/instruments')
export const usePartitions = () => $api.useQuery('get', '/v1/partitions')
export const useFailures = () => $api.useQuery('get', '/v1/failures')
export const useRates = () => $api.useQuery('get', '/v1/rates')
export const useGaps = () => $api.useQuery('get', '/v1/gaps', { params: { query: WHOLE_TAPE } })

export function useCandles(venue: string | null, ticker: string | null, interval: string) {
  return $api.useQuery(
    'get',
    '/v1/candles',
    { params: { query: { venue: venue ?? '', ticker: ticker ?? '', interval } } },
    { enabled: !!venue && !!ticker },
  )
}

export function useQuotes(ticker: string | null) {
  return $api.useQuery(
    'get',
    '/v1/tape/{kind}',
    { params: { path: { kind: 'quotes' }, query: { ...WHOLE_TAPE, limit: TAPE_ROWS, ticker: ticker ?? '' } } },
    { enabled: !!ticker },
  )
}

/** The ledger's fold report for a venue, passed through by the tower. */
export const usePortfolio = (venue: string) =>
  $api.useQuery('get', '/v1/portfolio', { params: { query: { venue } } })

/** What the Portfolio board derives its statistics over, stated beside the matrix. */
export const PORTFOLIO_STATISTICS = { horizon: '30m', hours: 48, min_observations: 20, z: 2, reference: 'BTC' }

/** Statistics derived from the tape on request, over the last `hours` to the minute. */
export function useStatistics(venue: string) {
  const to = Math.floor(Date.now() / 60_000) * 60_000_000
  const { horizon, hours, min_observations, z, reference } = PORTFOLIO_STATISTICS
  return $api.useQuery('get', '/v1/statistics', {
    params: { query: { venue, horizon, from: to - hours * 3_600_000_000, to, min_observations, z, reference } },
  })
}
