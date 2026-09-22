# galata-tower

The market-data screen for [galata-datawatch](https://github.com/sercanatalik/galata-datawatch),
in one binary: an axum read API over the record, and the React app it serves.

> **Early.** The record and the live status are served and shown; market data
> and charts are not. What is settled is the shape — the dependency wall, the
> generated contract, and what the tower is allowed to know.

## It watches the record, not the worker

A heartbeat is a claim. A closed partition still holding 1,412 segments is a
fact on disk. Every route answers from `galata-segments`' own listing rather
than from anything a capture process says about itself:

```
  GET /v1/about        the archive root, and the tape columns a read can prune on
  GET /v1/partitions   the partitions the record holds
  GET /v1/overdue      closed days still holding more segments than compaction left
  GET /v1/gaps         what the record says is missing, by cause
  GET /v1/status       live state, as server-sent events: venue status, the
                       broker's reachability, and when the record advances
  GET /v1/tape/{kind}  a window of one dataset, bounded by what is durable
```

**A decimal crosses as a string.** Every price and size in the tape is
`Decimal128(38, 18)`, and the tape route sends them quoted — `"80770.000000..."`,
not `80770.0`. Sent as JSON numbers they would be doubles before the browser
could decline to round them, and `check-no-float-money.sh` could not see it,
because nothing would have converted anything. `arrow-json`'s writer emits them
unquoted, which is why this tower serialises the rows itself.

`scripts/check-documented-routes.sh` holds that list to the contract, in both
directions: a route added without a line here fails, and so does a line for a
route that is not served.

`/v1/overdue` reports and does not judge. What counts as too many segments is
the operator's threshold, not a number this binary has an opinion about.

## The wall

This tree links **no capture loop**, and that is checked rather than claimed:

```sh
./scripts/check-no-capture-loop.sh
```

`galata-datawatch` is taken with `default-features = false`, which is one word
in a manifest and **85 crates** in the tree — 168 against 253, measured here.
The guard asserts exactly that: `cargo tree -e features` names
`galata-datawatch feature "capture"` when it is on, and the guard greps for it,
so the rule is checked rather than approximated.

**The rule is not "no async" — it is "nothing that exists to talk to a venue".**
`tokio`, `hyper` and `hyper-util` are here and are fine: they are axum's, and a
server needs a server. So is `rustls`, once `galata-broker` arrives, because
NATS runs over TLS and TLS exists to talk to anything. This guard forbade it
until 2026-09-22 for a reason its own header said was not the rule; the list now
names the venue transports — `tokio-tungstenite`, `tungstenite`, `reqwest` — as
a second net for one added directly, since that leaves the feature off.

Both halves are watched failing: turning the feature on, and adding a transport
to the manifest. Verified with `galata-broker` present — 243 crates, `rustls`
among them, guard green — and still red if the capture feature is on beside it,
so the broker is not a hole.

## The contract is generated, not written

```sh
./scripts/check-contract-drift.sh            # is the committed document what the code serves?
./scripts/check-contract-drift.sh --write    # refresh it, and the screen's types with it
```

`utoipa` derives the schemas from the response types and `utoipa-axum` derives
the paths from the routes, so a route is declared once. The binary that serves
them prints the document — `galata-tower --dump-openapi` — and
`openapi.snapshot.json` is committed as the reviewed authority.
`ui/src/contract/api.d.ts` is generated from that snapshot and committed too,
so the screen's types and the server's have one source rather than two that
agree by inspection.

The predecessor did this by hand: 669 lines of TypeScript, a 233-line client,
and a 343-line fixture test whose own header names the weakness — *"a field the
server REMOVES fails naming it, and a field it ADDS passes. One direction."*
Its fixtures were produced by a `cargo run` in a different repository. Both
halves are here, the generated types are 165 lines, and a byte diff catches a
field added as readily as one removed.

The guard regenerates twice and compares before trusting the diff, because a
byte comparison is only fair over a deterministic generator — and a flaky guard
teaches people to ignore it.

## Testing the screen

```sh
cd ui && pnpm test
```

The server's tests and the screen's are separate suites, and until 2026-09-22
there was only one of them. `check-screen-tests.sh` runs the second in the
gate.

They cover the pure functions and nothing else: money parsing, the two clocks'
arithmetic, why the screen is silent, and how a duration renders. No jsdom and
no testing-library — every one of those takes values and returns values, and a
DOM would cost setup on every run to hold nothing extra. About 130ms.

**What they cannot see is a panel that computes correctly and draws nothing.**
That is not hypothetical: two panels once asked for wall-clock windows against
a thirty-three-hour-old tape and rendered empty tables, and a ticker selector
derived from capped rows offered one instrument of six. Both were found by
opening the browser, and that stays the answer for anything that renders.

## Layout

```
  crates/galata-tower/   the server
  ui/                    the React app; `ui/dist` is embedded at compile time
  openapi.snapshot.json  the contract, generated and committed
  scripts/               the guards
```

One binary serves both halves, following the cereyan pattern already proven in
this tree — so a deployment runs no node process.

## Running it

```sh
GALATA_ARCHIVE=../galata-datawatch/var/archive cargo run
```

`GALATA_TAPE` points at the tape, `var/tape` by default, which
`galata-tape-rebuild` writes beside the archive.
`GALATA_TOWER_LISTEN` moves the tower off `127.0.0.1:8777`, and `GALATA_BROKER`
off `127.0.0.1:4222`. Loopback is the default for both because serving other
machines, and reaching another machine's bus, are deployment decisions — made
by setting these rather than by the binary assuming them.

**The tower starts whether or not a broker answers.** The record is a fact on
disk and does not need a bus to be true, so a refused connection is reported
and the record is still served.

## Following the record

The screen does not poll. The tower reads each tape kind's durable bound once a
second and sends a `tape` event only when one moves; the browser refetches that
kind's rows when it is told, and not otherwise.

That shape was measured rather than assumed. Against the real tape:

```
  open + bound   53µs      "has it grown?"
  full view      2.85ms    39,231 rows decoded
```

Asking is fifty-four times cheaper than reading, so the tower asks — once, for
every browser — and the expensive half runs only when the answer is yes. A
`refetchInterval` in the browser would pay the expensive half on a timer to
usually learn nothing; a conditional request would still cost a round trip per
browser per interval to be told nothing happened. The tower already holds an
open channel to every browser, and not asking is cheaper than a cheap way of
asking.

`cargo run --release --example cost-of-a-bound` is how those two numbers were
got, and is kept runnable so the next person can check them rather than trust
them.

Each panel shows how long since the record it draws last advanced, because a
current table and an hour-old one are otherwise identical.

## What it deliberately does not do

- **Read `markets.>` from the broker.** The record has market data, bounded,
  and an hour of candles lives there anyway. The subject roots were separated
  so that a dashboard could take `status.>` *without* the firehose, and the
  `reader` grant covers `status.>` alone.

  The reason is the predecessor's: *"an identity that could read every algo's
  book is exactly what a password on an operator's laptop should not be."* A
  process that serves a web page holds that password in its environment.

  The grant is the boundary and the server enforces it;
  `check-no-market-reach.sh` holds the other side, so that a subscription
  written here fails in this repository rather than being repaired by widening
  the grant.

Licensed under the MIT licence.
