import { useAbout } from '../data/queries'
import { facts, lags, type Fact } from '../data/truth'
import { stamp } from '../kit/format'
import { Refusal } from '../kit/Panel'
import { useLiveStatus } from '../live/status'

function Check({ tone }: { tone: Fact['tone'] }) {
  const stroke = tone === 'bad' ? 'var(--amber)' : tone === 'unknown' ? 'var(--muted)' : 'var(--text)'
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke={stroke} strokeWidth="1.6" aria-hidden="true">
      {tone === 'ok' ? <path d="M2.5 7.5l3 3 6-7" /> : tone === 'bad' ? <path d="M7 3v5M7 10.5v.5" /> : <path d="M3 7h8" />}
    </svg>
  )
}

/** What the tower can vouch for, and how far the tape trails the archive. */
export default function TruthBar() {
  const about = useAbout()
  const live = useLiveStatus()
  const failed: unknown = about.error
  if (failed) return <div className="card"><Refusal what="The tower's own state" error={failed} /></div>
  const all = facts(about.data, live)
  const behind = about.data ? lags(about.data.frontiers, live.archive) : []
  return (
    <section aria-label="What the tower can vouch for" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <div className="facts" style={all.length > 4 ? { gridTemplateColumns: `repeat(${Math.min(all.length, 6)}, minmax(0, 1fr))` } : undefined}>
        {all.map((f) => (
          <div key={f.label} className={`fact ${f.tone === 'ok' ? '' : f.tone}`}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Check tone={f.tone} />
              <span className="eyebrow">{f.label}</span>
            </div>
            <div className="value">{f.value}</div>
            <div className="detail" title={f.detail}>{f.detail || ' '}</div>
          </div>
        ))}
      </div>
      {behind.map((l) => (
        <div key={l.venue} className="lag" role="status">
          <svg width="22" height="22" viewBox="0 0 22 22" fill="none" stroke="var(--amber)" strokeWidth="1.8" aria-hidden="true">
            <circle cx="11" cy="11" r="9" />
            <path d="M11 6v6l3.5 2" />
          </svg>
          <div>
            <div className="title">
              {l.tape === null ? `${l.venue}: the tape holds nothing the archive has` : `${l.venue}: the tape is ${l.text} behind the archive`}
            </div>
            <div className="body">
              The tape ends at {stamp(l.tape)}. The archive holds arrivals to {stamp(l.archive, true)}. Prices come from
              the archive; charts, counts and coverage come from the tape.
            </div>
          </div>
          <code>galata-tape-rebuild {l.venue}</code>
        </div>
      ))}
    </section>
  )
}
