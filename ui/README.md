# ui

**The galata-tower screen.** A React app, built by Vite into `ui/dist` and
embedded into the tower binary at compile time, so a deployment runs no node
process.

## Stack

| | |
|---|---|
| Framework | React 19, TypeScript 5.9 |
| Build | Vite 7; `base: './'`, so the screen works from whatever path it is mounted at |
| Data | TanStack Query through `openapi-react-query` and `openapi-fetch` |
| Contract | `openapi-typescript` types generated from the tower's committed OpenAPI document |
| Money | `decimal.js`; decimals arrive as strings and never become floats on the way in |
| Charts | `lightweight-charts` 5 |
| Fonts | `@fontsource-variable` Instrument Sans and JetBrains Mono, bundled into `dist` |
| Tests | Vitest, in a Node environment, with no DOM |

## Commands

```sh
pnpm install
pnpm dev          # Vite dev server; /v1 is proxied to a tower on 127.0.0.1:8777
pnpm test         # the unit tests, ~130 ms
pnpm build        # type-check, then write ui/dist
pnpm contract     # regenerate src/contract/api.d.ts from ../openapi.snapshot.json
```

For development, run the tower (`cargo run` from the repository root) and
`pnpm dev` side by side. A debug build of the tower reads `ui/dist` from disk
at run time, so a `pnpm build` shows up without recompiling the server. A
release build embeds the files.

## Layout

```text
  src/
    main.tsx            fonts, the query client, the root
    App.tsx             the shell: header, truth bar, and the view the fragment names
    app/                router.ts (hash routes) · TruthBar.tsx
    views/              Overview.tsx · Markets.tsx · Record.tsx
    kit/                Panel.tsx (the four states, rendered once) · format.ts (times, durations, counts)
    charts/             CandleChart.tsx · BackfillBand.ts (a v5 series primitive) · Spark.tsx · useChart.ts
    data/               queries.ts (every read in one place) · truth.ts (the truth bar's decisions)
    Boundary.tsx        the error boundary around each section
    panel.ts            which of the four states a read is in
    contract/
      api.d.ts          GENERATED from ../openapi.snapshot.json; never edited
      client.ts         the one API client, typed by api.d.ts
      money.ts          the only place a decimal string is parsed, and the only float
    live/status.ts      the SSE store: venues, broker, tape bounds, archive frontiers
  dist/                 the committed build, embedded by rust-embed
```

The look is set by tokens in `index.css`: a dark ground, Instrument Sans for text and JetBrains Mono for figures, both bundled rather than fetched. Blue means up, orange means down, and amber is reserved for the tower's own state.

## Rules the screen keeps

- **The contract is generated.** `api.d.ts` comes from the server's own
  types, through the committed OpenAPI snapshot. A path the client cannot
  spell is a path the server does not serve, and the compiler says so.
  `../scripts/check-contract-drift.sh` fails the gate if the two disagree.
- **Money never touches a float.** Every price and size arrives as a string
  and is parsed once, in `src/contract/money.ts`, into a `Decimal`. That file
  is also the only one allowed to turn a `Decimal` into a number for
  plotting. `../scripts/check-no-float-money.sh` rejects `parseFloat(` and
  `Number(` anywhere else under `src/`.
- **Live status is level-triggered.** Every message is a whole snapshot for
  one venue, so the store replaces rather than patches, and a dropped message
  costs one interval, never information. A venue that stops publishing stays
  on screen with its age climbing, because absence after presence is what an
  operator most needs to see.
- **The screen does not poll.** Panels refetch when the server's SSE stream
  says their data moved.
- **`dist/` is the build of `src/`.** It is committed because `rust-embed`
  needs it at compile time. `../scripts/check-dist-drift.sh` rebuilds and
  compares, so a forgotten `pnpm build` fails the gate instead of shipping
  last week's screen.

The tests cover pure functions only: money parsing, the arithmetic of the two
clocks, why a panel is silent, and how a duration renders. Anything that
renders is checked by opening a browser.
