# galata-tower

The market-data screen for [galata-datawatch](https://github.com/sercanatalik/galata-datawatch),
in one binary: an axum read API over the record, and the React app it serves.

> **Skeleton.** The API answers three routes and the screen is a placeholder.
> What is settled is the shape — the dependency wall, the layout, and what the
> tower is allowed to know.

## It watches the record, not the worker

A heartbeat is a claim. A closed partition still holding 1,412 segments is a
fact on disk. Every route answers from `galata-segments`' own listing rather
than from anything a capture process says about itself:

```
  GET /v1/about        the archive root, and the tape columns a read can prune on
  GET /v1/partitions   the partitions the record holds
  GET /v1/overdue      closed days still holding more segments than compaction left
```

`/v1/overdue` reports and does not judge. What counts as too many segments is
the operator's threshold, not a number this binary has an opinion about.

## The wall

This tree links **no capture loop**, and that is checked rather than claimed:

```sh
./scripts/check-no-capture-loop.sh
```

`galata-datawatch` is taken with `default-features = false`, which is one word
in a manifest and **85 crates** in the tree — 164 against 249, measured here.
Turn the feature on and `reqwest`, `rustls`, `tokio-tungstenite`, `tungstenite`,
`rustls-pki-types` and `webpki-roots` arrive behind a screen whose job is to
read parquet. The guard is watched failing, by planting exactly that.

`tokio`, `hyper` and `hyper-util` *are* here and are fine. They are axum's, and
a server needs a server. The rule is not "no async" — it is "nothing that exists
to talk to a venue".

## Layout

```
  crates/galata-tower/   the server
  ui/                    the React app; `ui/dist` is embedded at compile time
  scripts/               the guards
```

One binary serves both halves, following the cereyan pattern already proven in
this tree — so a deployment runs no node process.

## Running it

```sh
GALATA_ARCHIVE=../galata-datawatch/var/archive cargo run
```

`GALATA_TOWER_LISTEN` moves it off `127.0.0.1:8777`. Loopback is the default
because serving other machines is a deployment decision, made by setting that
rather than by the binary assuming it.

## What is not here yet

- **The screen.** `ui/` holds a placeholder; the React app is ported from the
  predecessor, which was built as a risk board and has to be repointed.
- **The live status stream.** `galata-broker` and `status.<venue>` arrive with
  the status surface, and bring `async-nats` with them.
- **The generated contract.** `utoipa` → `openapi.snapshot.json` →
  `openapi-typescript`, with a `--check` mode failing CI on drift. A snapshot of
  three routes is a snapshot nobody reads; it arrives with the surface.

Licensed under the MIT licence.
