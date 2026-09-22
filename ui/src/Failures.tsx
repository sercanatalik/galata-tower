import { $api } from './contract/client'
import { useRecordAdvances } from './live/status'

/**
 * What the record could not parse.
 *
 * **A failure is not a gap, and the difference is the point.** A gap is a
 * known absence with a cause and bounds, and the panel above reports it. A
 * failure is a payload that *arrived* and produced no row — the record looks
 * complete, the rows are simply not there, and until this panel nothing said
 * so. The archive has written these since Tier 1 and nothing read them back.
 *
 * The bytes are still in the record, under the sequence shown here. This
 * offers the join rather than performing it: serving raw venue payloads over
 * HTTP is a different surface with a different risk.
 */
export default function Failures() {
  const advanced = useRecordAdvances('quotes')
  const { data, error, isPending } = $api.useQuery('get', '/v1/failures')

  if (error) {
    return (
      <section>
        <h2>Failures</h2>
        <div className="refusal">
          <strong>The failures could not be read.</strong>
          <p>{String(error)}</p>
        </div>
      </section>
    )
  }

  const failing = data?.failing ?? []

  return (
    <section>
      <h2>
        Failures <span className="count">{data ? failing.length : '…'}</span>
      </h2>
      <p className="muted">
        payloads that arrived and produced no row — the bytes are still in the record, under the
        sequences below
        {' · '}
        {advanced === null ? 'the record has not moved since this page loaded' : `advanced ${advanced}s ago`}
      </p>
      {isPending ? <p className="muted">reading the failures…</p> : null}
      {data && failing.length === 0 ? (
        /* **None is a sentence, not an empty table.** And it says how much was
           looked at, so a clean record reads differently from a wrong archive
           root — which would also show nothing. */
        <p className="muted">
          The record holds no failure. {data.partitions} partition
          {data.partitions === 1 ? '' : 's'} examined, {data.with_failures} with anything to read.
        </p>
      ) : null}
      {failing.length > 0 ? (
        <table className="tape">
          <thead>
            <tr>
              <th>venue</th>
              <th>dataset</th>
              <th>channel</th>
              <th>what went wrong</th>
              <th className="num">count</th>
              <th className="num">sequences</th>
            </tr>
          </thead>
          <tbody>
            {failing.map((f) => (
              <tr key={`${f.venue}/${f.kind}/${f.channel}/${f.error}`}>
                <td>
                  <code>{f.venue}</code>
                </td>
                {/* Both, because they are not the same thing: a `bbo` channel
                    lands under a `quotes` kind, and a failure row in a
                    partition its payload is not in is exactly what the
                    sequence exists to make findable. */}
                <td className="muted">{f.kind}</td>
                <td className="muted">{f.channel}</td>
                <td>{f.error}</td>
                <td className="num">{f.failures.toLocaleString()}</td>
                <td className="num">
                  <code>
                    {f.first_seq === f.last_seq ? f.first_seq : `${f.first_seq}–${f.last_seq}`}
                  </code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  )
}
