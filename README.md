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
  GET /v1/status       every venue's live status, as server-sent events
```

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

`GALATA_TOWER_LISTEN` moves it off `127.0.0.1:8777`, and `GALATA_BROKER` off
`127.0.0.1:4222`. Loopback is the default for both because serving other
machines, and reaching another machine's bus, are deployment decisions — made
by setting these rather than by the binary assuming them.

**The tower starts whether or not a broker answers.** The record is a fact on
disk and does not need a bus to be true, so a refused connection is reported
and the record is still served.

## What is not here yet

- **Market data.** `markets.<venue>.<ticker>.<kind>` is a firehose and needs a
  different answer from a status timer; the subject root was separated for that
  reason.

Licensed under the MIT licence.
