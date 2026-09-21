# ui

The screen. Not yet ported.

`ui/dist` carries a placeholder `index.html` **on purpose**: `rust-embed` reads
this folder at compile time, so an absent one is a build failure rather than an
empty page. The placeholder is tracked; `node_modules` is not.

## What lands here

The predecessor's React app — React 19, Vite, TanStack Query/Table, base-ui and
shadcn, Tailwind 4, `lightweight-charts` — repointed. It was built as a risk
board over a `tower` HTTP contract of risk, fold, valuation and scorecard, none
of which exists in a capture-only project. The screen half is reusable; what it
reads is not.

Its new scope: a symbol list per venue, the record's own facts beside the live
status stream, and charts over the tape's candles.

## The money rule, carried forward

Money crosses the wire as strings and is parsed with `decimal.js`.
`src/contract/money.ts` is to be the only file allowed to turn a `Decimal` into
a number for plotting, held by a guard, as it is in the predecessor — and as
`check-no-float-money.sh` holds the Rust half one repository over.
