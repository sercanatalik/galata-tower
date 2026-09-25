// Durations and times. Integers the record measured, so no money path is involved.

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** A duration in whole units, the largest that fits. A sub-second gap is still a gap. */
export function forHumans(micros: number): string {
  const seconds = Math.round(micros / 1_000_000)
  if (micros > 0 && seconds === 0) return '<1s'
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`
  if (seconds < 86_400) return `${(seconds / 3600).toFixed(1)}h`
  return `${(seconds / 86_400).toFixed(1)}d`
}

/** A lag in two units, e.g. `3 d 3 h`, `4 m 12 s`. */
export function lag(micros: number): string {
  const s = Math.max(0, Math.floor(micros / 1_000_000))
  const d = Math.floor(s / 86_400)
  const h = Math.floor((s % 86_400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 0) return `${d} d ${h} h`
  if (h > 0) return `${h} h ${m} m`
  if (m > 0) return `${m} m ${s % 60} s`
  return `${s} s`
}

/** `22 Sep 06:30 UTC`, or with seconds. */
export function stamp(micros: number | null | undefined, seconds = false): string {
  if (micros === null || micros === undefined || micros <= 0) return '—'
  const d = new Date(micros / 1000)
  const hh = String(d.getUTCHours()).padStart(2, '0')
  const mm = String(d.getUTCMinutes()).padStart(2, '0')
  const ss = seconds ? `:${String(d.getUTCSeconds()).padStart(2, '0')}` : ''
  return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${hh}:${mm}${ss} UTC`
}

/** `06:30:54.544`, venue or receipt time of day. */
export function clock(micros: number | null | undefined): string {
  if (micros === null || micros === undefined || micros <= 0) return '—'
  return new Date(micros / 1000).toISOString().slice(11, 23)
}

/** `22 Sep`, from a `YYYY-MM-DD` partition date. */
export function day(date: string): string {
  const [, m, d] = date.split('-')
  return `${Number.parseInt(d, 10)} ${MONTHS[Number.parseInt(m, 10) - 1]}`
}

/** A count with grouped thousands. */
export function count(n: number | null | undefined): string {
  return n === null || n === undefined ? '—' : n.toLocaleString('en-US')
}
