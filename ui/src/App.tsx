import { href, useRoute } from './app/router'
import TruthBar from './app/TruthBar'
import Boundary from './Boundary'
import { useFollowTheArchive, useFollowTheRecord, useLiveStatus } from './live/status'
import Markets from './views/Markets'
import Overview from './views/Overview'
import Record from './views/Record'

function Wordmark() {
  return (
    <a href="#/" className="wordmark">
      <svg width="18" height="22" viewBox="0 0 18 22" fill="none" stroke="var(--amber)" strokeWidth="1.6" aria-hidden="true">
        <path d="M9 1v3M5 21l1.5-13h5L13 21M4 21h10M6.2 12h5.6M9 4l-3 4h6z" />
      </svg>
      <div>
        galata<span>·tower</span>
      </div>
    </a>
  )
}

export default function App() {
  useFollowTheRecord()
  useFollowTheArchive()
  const route = useRoute()
  const live = useLiveStatus()
  const views = [
    { view: 'overview', name: 'Overview', to: href({ view: 'overview' }) },
    { view: 'markets', name: 'Markets', to: '#/m' },
    { view: 'record', name: 'Record', to: href({ view: 'record' }) },
  ] as const

  return (
    <>
      <header className="shell-head">
        <Wordmark />
        <nav className="nav" aria-label="Views">
          {views.map((v) => (
            <a key={v.view} href={v.to} aria-current={route.view === v.view ? 'page' : undefined}>
              {v.name}
            </a>
          ))}
        </nav>
        <div className="stream mono">
          <span className={`dot ${live.connected ? '' : 'off'}`} />
          {live.connected ? 'stream live' : 'stream down'}
          {live.drops > 0 ? ` · ${live.drops} drops` : ''}
          {live.missed > 0 ? ` · ${live.missed} missed` : ''}
        </div>
      </header>
      <main className="page">
        <Boundary name="The truth bar">
          <TruthBar />
        </Boundary>
        {route.view === 'overview' ? <Overview /> : null}
        {route.view === 'markets' ? <Markets venue={route.venue} ticker={route.ticker} /> : null}
        {route.view === 'record' ? <Record /> : null}
      </main>
    </>
  )
}
