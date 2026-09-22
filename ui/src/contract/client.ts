// The one client, and it is inferred rather than written.
//
// The predecessor hand-rolled 233 lines of fetch wrappers beside 669 lines of
// hand-kept types. Here `paths` comes from `api.d.ts`, which is generated from
// the committed `openapi.snapshot.json`, which is generated from the routes --
// so a path this file cannot spell is a path the server does not serve, and
// the compiler says so.
//
// No runtime validation, deliberately: the contract is generated from the
// server's own types and diffed in the gate, so a validator here would be
// checking the generator rather than the data. The one thing that genuinely
// cannot be trusted from the wire is a decimal, and that is `money.ts`.

import createFetchClient from 'openapi-fetch'
import createClient from 'openapi-react-query'

import type { paths } from './api'

// Same origin: one binary serves the API and this screen. In dev, Vite proxies
// `/v1` to the running tower.
const fetchClient = createFetchClient<paths>({ baseUrl: '/' })

export const $api = createClient(fetchClient)
