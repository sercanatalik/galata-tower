//! The tower: a read API over the record, and the screen that reads it.
//!
//! **It watches the record, not the worker.** A heartbeat is a claim; a closed
//! partition still holding 1,412 segments is a fact on disk. Every route here
//! answers from the store's own listing rather than from anything a capture
//! process says about itself — which is why this binary links no capture loop
//! and cannot be made to run one.
//!
//! One process serves both halves: the API below, and `ui/dist` embedded at
//! compile time, so there is no node process in a deployment.

mod failures;
mod tape;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use galata_broker::{BrokerIdentity, NatsSubscriber};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};
use tokio_stream::Stream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// The screen, embedded at compile time.
///
/// **One binary serves the API and the screen**, so a deployment runs no node
/// process. The folder must exist when this compiles, which is why `ui/dist`
/// carries a placeholder in the tree rather than being generated on demand.
///
/// **A debug build does not embed it.** `rust-embed` reads the folder from
/// disk at run time unless `debug-embed` is on, so `cargo run` serves whatever
/// `ui/dist` holds right now and a rebuild is not needed to see a screen
/// change. A release build embeds, through `include_bytes!` — which cargo
/// records as a real dependency, so a release DOES rebuild when the folder
/// changes. Both halves were measured on 2026-09-22, after a day in which a
/// stale screen was blamed for what turned out to be a silenced `tsc -b`
/// failure leaving `ui/dist` unwritten. Neither half needs a build script, and
/// one was written and thrown away on the strength of that wrong guess.
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../ui/dist"]
struct Screen;

/// Where the record lives, and the live status every browser shares.
#[derive(Clone)]
struct Tower {
    /// The archive root: `var/archive` in a datawatch deployment.
    archive: PathBuf,
    /// The tape root: `var/tape` beside it.
    tape: PathBuf,
    /// One subscription's traffic, fanned to every connected screen.
    ///
    /// Two kinds of thing on one channel, which is the shape the predecessor's
    /// tower used (`Live::Entry` / `Live::Alert`) and for the same reason: a
    /// second channel would need a second subscription per browser and a
    /// second lag policy, and would buy nothing.
    status: broadcast::Sender<Live>,
    /// The newest snapshot per venue.
    ///
    /// **Insert or replace, never remove** -- carried from the predecessor's
    /// `Board::observe`, and held on the SERVER so that *once seen, never
    /// dropped* does not depend on every client implementing it. A venue that
    /// stops publishing keeps its last snapshot here, because absence after
    /// presence is the statement an operator most needs rendered.
    ///
    /// Read-mostly: one writer at the publisher's cadence, a reader per
    /// connecting browser and per lag. `Arc<Snapshot>` so a board frame clones
    /// pointers rather than payloads.
    board: Arc<RwLock<BTreeMap<String, Arc<Snapshot>>>>,
    /// Whether there is a broker at all, for a browser that connects mid-outage.
    broker: Arc<RwLock<BrokerState>>,
    /// Each kind's durable bound, as the watch last read it.
    bounds: Arc<RwLock<BTreeMap<String, i64>>>,
}

/// One thing the tower has to say.
///
/// A status snapshot, or a change in whether there is a broker to get them
/// from. The second is not decoration: without it an outage and a quiet venue
/// look identical to a browser, which is the defect this type exists to fix.
#[derive(Clone, Debug)]
enum Live {
    Status(Arc<Snapshot>),
    Broker(BrokerState),
    /// One kind's tape grew. **Not the rows** — the route that serves them
    /// already caps them, and a browser that does not draw this kind should
    /// not pay to receive it.
    Tape(TapeMoved),
}

/// A kind's tape has a new durable bound.
#[derive(Clone, Debug, Serialize, ToSchema)]
struct TapeMoved {
    /// Which dataset, as `/v1/tape/{kind}` spells it.
    kind: String,
    /// The new durable position.
    bound: i64,
}

/// Whether the tower has a status subscription — **reported, not judged**.
///
/// There is no threshold here and no verdict. The attempt count is a number
/// and the refusal is the broker's own words; whether either is acceptable is
/// the operator's call, and a tower that decided it would be deciding with
/// less information than they have.
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
struct BrokerState {
    /// True between a subscription being established and its ending.
    connected: bool,
    /// Connect attempts since the tower last held a subscription.
    ///
    /// Zero while connected. It climbs during an outage, which is what makes
    /// a misconfiguration visible: a wrong password is retried for ever, and
    /// this is what says so.
    attempts: u32,
    /// What the broker said when it last refused, verbatim.
    ///
    /// The identity type cannot print the password, so this is safe to show.
    refusal: Option<String>,
}

/// What a browser is handed on connect, and after a gap.
///
/// The venues are cloned rather than borrowed: a board frame is sent once per
/// connection and once per lag, not once per snapshot, and a lifetime in the
/// contract's schema would buy nothing at that rate.
#[derive(Serialize, ToSchema)]
struct Board {
    /// Whether these venues are current or a record of an interrupted stream.
    broker: BrokerState,
    /// Each kind's durable bound, for the kinds that have written anything.
    ///
    /// Here as well as in the event, for the same reason the broker's state
    /// is: the event says it MOVED, and this says where it STANDS. A browser
    /// connecting into a quiet hour would otherwise learn nothing until the
    /// next move, which may never come.
    #[schema(value_type = Object)]
    bounds: BTreeMap<String, i64>,
    /// Every venue seen since the tower started, newest snapshot each.
    venues: Vec<Snapshot>,
}

/// One venue's status, exactly as it was published.
///
/// The payload is not parsed here. The tower forwards what the venue said; the
/// screen's types come from the contract, and a tower that re-typed the
/// snapshot would be a second statement of it free to disagree.
#[derive(Clone, Debug, Serialize, ToSchema)]
struct Snapshot {
    /// The subject it arrived on, e.g. `status.hyperliquid`.
    subject: String,
    /// The venue, taken from the subject.
    venue: String,
    /// The snapshot body, as published.
    #[schema(value_type = Object)]
    body: serde_json::Value,
}

/// **Stated, not derived.** The publisher sends one snapshot per venue per
/// interval, so 64 is roughly a minute of backlog at datawatch's cadence: long
/// enough that a browser tabbed away briefly loses nothing, short enough that a
/// dead one cannot hold a minute of memory per venue. Nothing here has measured
/// it; a venue count or a cadence that makes a minute the wrong size is what
/// would change it.
const STATUS_BACKLOG: usize = 64;

/// One partition of the record, as the store lists it.
#[derive(Serialize, ToSchema)]
struct Partition {
    /// Its path relative to the archive root.
    path: String,
}

/// A closed day still holding more segments than compaction should have left.
///
/// **Reported, never judged.** The number is here; whether it is bad belongs to
/// whoever set the threshold.
#[derive(Serialize, ToSchema)]
struct Overdue {
    /// The partition.
    path: String,
    /// How many segments it still holds.
    segments: usize,
}

/// What this tower reads, and how the tape may be narrowed.
///
/// The screen builds its own queries, and `prune_on` is what makes a windowed
/// read cheap — so it is served rather than duplicated in TypeScript, where it
/// would be a second copy free to disagree.
#[derive(Serialize, ToSchema)]
struct About {
    /// The archive root being watched.
    archive: String,
    /// The tape columns a reader can prune on, from the schema itself.
    prune_on: Vec<String>,
}

/// The partitions the record holds.
#[utoipa::path(
    get,
    path = "/v1/partitions",
    responses((status = 200, description = "Every partition in the archive", body = Vec<Partition>)),
)]
async fn partitions(State(tower): State<Tower>) -> Json<Vec<Partition>> {
    let root = tower.archive.clone();
    let found = tokio::task::spawn_blocking(move || galata_segments::partitions(&root))
        .await
        .unwrap_or_default();
    Json(
        found
            .into_iter()
            .map(|path| Partition {
                path: path
                    .strip_prefix(&tower.archive)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            })
            .collect(),
    )
}

/// How many segments a closed partition may hold before it is worth naming.
///
/// **One, and that is not arbitrary.** Compaction leaves one segment per
/// partition, so *more than one* is exactly *not compacted since it closed*.
/// Anything above this would be a judgement about how much neglect is
/// acceptable, which is the operator's — which is why it is a parameter.
const COMPACTED_TO: usize = 1;

/// What a caller may narrow the overdue listing by.
#[derive(Deserialize, utoipa::IntoParams)]
struct OverdueQuery {
    /// The most segments a closed partition may hold. Defaults to
    /// [`COMPACTED_TO`]. Zero lists every closed partition holding anything,
    /// which shows the shape of the store rather than only its problems.
    max_segments: Option<usize>,
}

/// Closed days still holding more segments than compaction should have left.
///
/// **Closed means closed.** Until 2026-09-22 this passed `"9999-99-99"` as the
/// current day, so every dated partition counted as closed — including the one
/// being written. Six of twelve rows on the screen were the day still being
/// captured, which holds many small segments by design, under a heading
/// reading *Closed*.
///
/// That is the drift the calendar module warns about in its own header: *"a
/// second implementation does not fail when it drifts — it disagrees."* The
/// day is now `date_of` on this tower's clock — the same function that NAMES
/// the partitions — so there is one calendar and one definition of closed,
/// shared with the `galata-compact` that acts on it.
///
/// It also cost the surface its purpose: this exists to catch *"the wrong var
/// directory, the stale binary and the `--dry-run` left in"*, and all three
/// were buried under guaranteed daily noise.
#[utoipa::path(
    get,
    path = "/v1/overdue",
    params(OverdueQuery),
    responses((status = 200, description = "Closed days still holding segments", body = Vec<Overdue>)),
)]
async fn overdue(
    State(tower): State<Tower>,
    Query(query): Query<OverdueQuery>,
) -> Json<Vec<Overdue>> {
    let root = tower.archive.clone();
    // **The clock is read here, and the judgement takes the day.** The library
    // splits these for a stated reason — so the rule stays replayable and a
    // test can drive it without waiting for midnight — and the same split is
    // what makes the boundary testable at all.
    let today = today_utc();
    let max_segments = query.max_segments.unwrap_or(COMPACTED_TO);
    // A listing walks the store, so it does not belong on the async executor.
    let found = tokio::task::spawn_blocking(move || {
        galata_segments::overdue_closed(&root, &today, max_segments)
    })
    .await
    .unwrap_or_default();
    Json(
        found
            .into_iter()
            .map(|(path, segments)| Overdue {
                path: path
                    .strip_prefix(&tower.archive)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
                segments,
            })
            .collect(),
    )
}

/// The current UTC day, as the partitions spell it.
///
/// `galata_datawatch::date_of` and nothing else: a partition is NAMED by that
/// function, and whether it is closed is decided by comparing against it. A
/// second way of formatting the day would not fail when it drifted — it would
/// disagree, which is the failure this whole route just had.
fn today_utc() -> String {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_micros()).unwrap_or(i64::MAX))
        // Before the epoch is a clock that is very wrong; naming day zero is
        // the conservative answer, because it makes every partition closed
        // rather than none, and an operator seeing the whole store listed will
        // look at the clock.
        .unwrap_or(0);
    galata_datawatch::date_of(micros)
}

/// What this tower reads.
#[utoipa::path(
    get,
    path = "/v1/about",
    responses((status = 200, description = "The archive root and the tape's prune columns", body = About)),
)]
async fn about(State(tower): State<Tower>) -> Json<About> {
    Json(About {
        archive: tower.archive.display().to_string(),
        prune_on: galata_datawatch::tape::schema::PRUNE_ON
            .iter()
            .map(|column| (*column).to_owned())
            .collect(),
    })
}

/// The screen, or its index for any path the client routes itself.
async fn screen(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = Screen::get(path).or_else(|| Screen::get("index.html"));
    match file {
        Some(content) => {
            let mime = mime_guess_for(path);
            ([(header::CONTENT_TYPE, mime)], content.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "no screen is embedded in this build").into_response(),
    }
}

/// Enough of a type table for what a built screen actually contains.
fn mime_guess_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "text/html; charset=utf-8",
    }
}

/// One subscription, for every browser.
///
/// Opened once at boot and outliving every request: a subscription per browser
/// would multiply the broker's fan-out by the number of open tabs and fire the
/// grant check on every page load.
///
/// **The tower starts whether or not a broker answers.** The record is a fact
/// on disk and does not need a bus to be true, so a refused connection is
/// reported and the process continues serving the half that works.
///
/// **And it keeps trying.** Until 2026-09-22 this returned on a refusal and
/// again when an established subscription ended, so a tower that came up one
/// second before its broker was accepting stayed statusless until somebody
/// restarted it — which is exactly what happened while verifying the change
/// before this one. There is no attempt count at which stopping is better than
/// continuing: a broker absent for an hour may return in the next minute, and
/// a tower that gave up is one somebody has to notice first.
///
/// The retry lives here rather than in `galata-broker`. `async-nats` has
/// `retry_on_initial_connect()` and `connect_as` deliberately does not set it,
/// because the capture needs *a broker that does not answer* (an outage: warn
/// and carry on) to stay distinct from *a broker that refuses the identity* (a
/// misconfiguration: exit non-zero). Setting it there would erase that.
fn subscribe_to_status(
    addr: String,
    status: broadcast::Sender<Live>,
    board: Arc<RwLock<BTreeMap<String, Arc<Snapshot>>>>,
    broker: Arc<RwLock<BrokerState>>,
) {
    tokio::spawn(async move {
        let identity = BrokerIdentity::new(
            "reader",
            std::env::var(galata_broker::password_var("reader")).unwrap_or_default(),
            galata_broker::password_var("reader"),
        );
        let mut wait = FIRST_RETRY;
        let mut attempts: u32 = 0;
        loop {
            attempts = attempts.saturating_add(1);
            match NatsSubscriber::connect(&addr, &identity, "status.>").await {
                Ok(mut subscriber) => {
                    tracing::info!(attempts, "subscribed to status.>");
                    // The ladder is reset by a subscription, not by a
                    // connection: a broker that accepts and immediately drops
                    // is still an outage, and backing off from the floor each
                    // time is the point of the floor being a whole second.
                    wait = FIRST_RETRY;
                    attempts = 0;
                    say(
                        &broker,
                        &status,
                        BrokerState {
                            connected: true,
                            attempts: 0,
                            refusal: None,
                        },
                    )
                    .await;
                    consume(&mut subscriber, &status, &board, &broker).await;
                    tracing::warn!("the status subscription ended");
                    say(
                        &broker,
                        &status,
                        BrokerState {
                            connected: false,
                            attempts: 0,
                            refusal: None,
                        },
                    )
                    .await;
                }
                Err(refusal) => {
                    // Named, not silent, and the password is not in it: the
                    // identity type cannot print it.
                    tracing::warn!(%refusal, attempts, "no status stream; the record is still served");
                    say(
                        &broker,
                        &status,
                        BrokerState {
                            connected: false,
                            attempts,
                            refusal: Some(refusal.to_string()),
                        },
                    )
                    .await;
                }
            }
            tokio::time::sleep(jittered(wait)).await;
            wait = (wait * 2).min(LONGEST_RETRY);
        }
    });
}

/// One second, then two, then four, to thirty.
///
/// The ceiling matters more than the floor. A race at boot should cost about a
/// second, and an hour of absence should not be an hour of connect attempts;
/// thirty seconds is slow enough to be free and quick enough that a returning
/// broker is picked up before anyone reloads the page.
const FIRST_RETRY: Duration = Duration::from_secs(1);
const LONGEST_RETRY: Duration = Duration::from_secs(30);

/// The wait, give or take a quarter of itself.
///
/// **Jitter, for the reason this tree already gives in the capture's reconnect
/// path**: retries that fall into lockstep with a broker restarting on a timer
/// miss it every time, and several towers coming back together should not
/// arrive as one. It comes from the clock rather than from `rand`, because a
/// dependency for ±25% on a retry delay is a dependency for nothing — this
/// needs successive waits to differ, not to be unguessable.
fn jittered(wait: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.subsec_nanos())
        .unwrap_or(0);
    let band = (wait.as_millis() as u64) / 2;
    let offset = if band == 0 {
        0
    } else {
        u64::from(nanos) % band
    };
    wait - wait / 4 + Duration::from_millis(offset)
}

/// Record a broker state and tell every connected browser.
///
/// Both, always: the event is the notification that it changed, and the stored
/// value is what a browser connecting a moment later is handed. Writing one
/// without the other is how a state stream becomes an event stream.
async fn say(
    broker: &Arc<RwLock<BrokerState>>,
    status: &broadcast::Sender<Live>,
    next: BrokerState,
) {
    *broker.write().await = next.clone();
    let _ = status.send(Live::Broker(next));
}

/// Drain one subscription until it ends, reporting the transport meanwhile.
///
/// Separated from the supervisor so that the loop above reads as what it is —
/// connect, consume, back off, repeat — rather than as three levels of nesting.
///
/// **There are two retries here, and only one of them is ours.** `async-nats`
/// reconnects an ESTABLISHED connection by itself, which is why the first
/// version of this change passed every test above and still claimed a broker
/// with `nats-server` killed: the subscription had not ended, so nothing
/// noticed. The loop above handles what the client will not — a connection
/// never established, since `connect_as` deliberately leaves
/// `retry_on_initial_connect` unset, and a subscription that genuinely ends.
/// This watches the client's own state and says what it sees.
async fn consume(
    subscriber: &mut NatsSubscriber,
    status: &broadcast::Sender<Live>,
    board: &Arc<RwLock<BTreeMap<String, Arc<Snapshot>>>>,
    broker: &Arc<RwLock<BrokerState>>,
) {
    // A second is well under the publisher's cadence, so an outage is on the
    // screen before the venue ages visibly. Polling rather than subscribing to
    // the client's event stream keeps the broker crate's surface to one
    // read-only accessor.
    let mut watch = tokio::time::interval(Duration::from_secs(1));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut was_up = true;
    loop {
        let (subject, payload) = tokio::select! {
            received = subscriber.next_addressed() => match received {
                Some(pair) => pair,
                None => return,
            },
            _ = watch.tick() => {
                let up = subscriber.connected();
                if up != was_up {
                    was_up = up;
                    tracing::warn!(connected = up, "the broker's transport changed");
                    say(
                        broker,
                        status,
                        BrokerState {
                            connected: up,
                            attempts: 0,
                            refusal: (!up).then(|| {
                                "the connection dropped; the client is reconnecting".to_owned()
                            }),
                        },
                    )
                    .await;
                }
                continue;
            }
        };
        // The venue is the subject's second token. A subject that does not
        // carry one is not a status subject, and is skipped rather than
        // guessed at.
        let Some(venue) = subject.split('.').nth(1) else {
            continue;
        };
        let body = serde_json::from_slice(&payload).unwrap_or(serde_json::Value::Null);
        let snapshot = Arc::new(Snapshot {
            subject: subject.clone(),
            venue: venue.to_owned(),
            body,
        });
        // Insert or replace, never remove.
        board
            .write()
            .await
            .insert(venue.to_owned(), Arc::clone(&snapshot));
        // `send` fails only when nobody is listening, which is the ordinary
        // case for a tower with no browser open.
        let _ = status.send(Live::Status(snapshot));
        // A message is proof the transport is up, whatever the last tick saw.
        if !was_up {
            was_up = true;
            say(
                broker,
                status,
                BrokerState {
                    connected: true,
                    attempts: 0,
                    refusal: None,
                },
            )
            .await;
        }
    }
}

/// How often the record is asked whether it moved.
///
/// **Below the compactor's own cadence, deliberately.** The tape is durable
/// parquet; measuring it more often samples a file that has not been rewritten.
/// One second at 53µs a kind is 265µs a second for the five served kinds —
/// which is the measurement that made a server-side watch the obvious shape
/// rather than a thing each browser does.
const TAPE_WATCH: Duration = Duration::from_secs(1);

/// Watch the record, and say when it moves.
///
/// **Made once, for every browser.** Asking whether the tape grew is 53µs;
/// reading it is 2.85ms. The cheap question is asked continuously here so that
/// the expensive answer is fetched only when it changed — where a
/// `refetchInterval` in the browser would pay the expensive half on a timer to
/// usually learn nothing, and a conditional request would still cost a round
/// trip per browser per interval to be told nothing happened. The tower
/// already holds an open channel to every browser; not asking is cheaper than
/// a cheap way of asking.
fn watch_the_record(
    tape: PathBuf,
    status: broadcast::Sender<Live>,
    bounds: Arc<RwLock<BTreeMap<String, i64>>>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TAPE_WATCH);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last: BTreeMap<String, i64> = BTreeMap::new();
        // Reported on the EDGE. A tape root that is not there is one log line,
        // not one a second for as long as the tower runs.
        let mut said_empty = false;
        loop {
            ticker.tick().await;
            // Off the runtime: this opens files. 53µs is short, but short is
            // not the same as non-blocking, and five of them share one thread
            // with every open SSE stream.
            let root = tape.clone();
            let Ok(found) = tokio::task::spawn_blocking(move || tape::bounds(&root)).await else {
                continue;
            };

            if found.is_empty() {
                if !said_empty {
                    said_empty = true;
                    tracing::info!(
                        root = %tape.display(),
                        "no tape has written anything yet; the screen will say so"
                    );
                }
            } else {
                said_empty = false;
            }

            for moved in moves(&last, &found) {
                let _ = status.send(Live::Tape(moved));
            }
            last = found.clone();
            *bounds.write().await = found;
        }
    });
}

/// Which kinds moved, given what was last seen and what is there now.
///
/// Separated from the watch so it can be held by tests: a loop that only ever
/// runs against a real tape is a loop whose backwards case is reasoned about
/// rather than exercised, and the backwards case is the one that would redraw
/// every chart if it were wrong.
fn moves(last: &BTreeMap<String, i64>, found: &BTreeMap<String, i64>) -> Vec<TapeMoved> {
    let mut moved = Vec::new();
    for (kind, position) in found {
        match last.get(kind) {
            // Unchanged. The ordinary case, and it sends nothing.
            Some(previous) if previous == position => {}
            // **Backwards is not a move.** It means the store was replaced
            // underneath the tower, which is worth a line in the log and is
            // not worth a chart redraw.
            Some(previous) if previous > position => tracing::warn!(
                %kind, previous, position,
                "the tape's bound went backwards; the store was replaced"
            ),
            _ => moved.push(TapeMoved {
                kind: kind.clone(),
                bound: *position,
            }),
        }
    }
    moved
}

/// Every venue's status, as it arrives.
///
/// SSE rather than a WebSocket: the traffic is one-directional, and the
/// browser's `EventSource` reconnects, backs off and honours `retry:` without a
/// line of client code.
#[utoipa::path(
    get,
    path = "/v1/status",
    responses((status = 200, description = "A server-sent event stream: a board frame on connect, then status, broker, tape and lagged events", body = Snapshot)),
)]
async fn status(
    State(tower): State<Tower>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    // **Subscribe FIRST, then read the board.** A snapshot arriving between the
    // two is then delivered by the stream rather than lost between them. The
    // other order has a hole exactly one message wide -- the kind of race that
    // appears once a week in production and never in a test.
    let mut rx = tower.status.subscribe();
    let broker = tower.broker.read().await.clone();
    let bounds = tower.bounds.read().await.clone();
    let opening = board_event(&*tower.board.read().await, broker, bounds);

    let stream = async_stream::stream! {
        // What is true now, before anything live. Sent even when empty, so a
        // browser can tell "nothing has published" from "not connected".
        yield Ok(opening);
        loop {
            match rx.recv().await {
                Ok(live) => yield Ok(event_for(Ok(live))),
                // The count AND the state. Two events because they say
                // different things: *you missed n* is worth reporting to an
                // operator, and *here is what is true* is what fixes it.
                // Sending only the count -- which this tower did until
                // 2026-09-22 -- leaves the browser to recover by waiting an
                // interval per venue.
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    yield Ok(event_for(Err(BroadcastStreamRecvError::Lagged(missed))));
                    let broker = tower.broker.read().await.clone();
                    let bounds = tower.bounds.read().await.clone();
                    yield Ok(board_event(&*tower.board.read().await, broker, bounds));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Every venue's newest snapshot, as one frame.
///
/// **The answer to every gap.** Connect, lag, reconnect -- all three are the
/// same question, *what is true now*, and this is it. That is what makes this a
/// STATE stream rather than an event stream: a message is a notification that
/// the state moved, and the state itself is always available.
fn board_event(
    board: &BTreeMap<String, Arc<Snapshot>>,
    broker: BrokerState,
    bounds: BTreeMap<String, i64>,
) -> Event {
    // **The broker's state travels with the venues.** A browser connecting
    // during an outage used to be handed an empty list and nothing else, and
    // rendered "no venue has published status yet" — false, reassuring, and
    // worth an afternoon of looking in the wrong place.
    let frame = Board {
        broker,
        bounds,
        venues: board.values().map(|s| (**s).clone()).collect(),
    };
    Event::default()
        .event("board")
        .json_data(&frame)
        .unwrap_or_else(|_| Event::default().event("board").data("{}"))
}

/// One received item as the event the browser sees.
///
/// Separated from the handler so the lag path can be exercised: forcing a real
/// receiver to fall behind through an HTTP connection is awkward, and a branch
/// that is only ever reasoned about is a branch that is not held.
fn event_for(received: Result<Live, BroadcastStreamRecvError>) -> Event {
    match received {
        Ok(Live::Status(snapshot)) => Event::default()
            .event("status")
            .json_data(&*snapshot)
            .unwrap_or_else(|_| Event::default().event("status").data("{}")),
        // Its own event name, so a browser dispatches rather than inspects.
        Ok(Live::Broker(state)) => Event::default()
            .event("broker")
            .json_data(&state)
            .unwrap_or_else(|_| Event::default().event("broker").data("{}")),
        Ok(Live::Tape(moved)) => Event::default()
            .event("tape")
            .json_data(&moved)
            .unwrap_or_else(|_| Event::default().event("tape").data("{}")),
        // **A gap is an event, never an absence.** Dropping is safe only
        // because the stream is level-triggered -- the next snapshot is the
        // whole state -- and even then the browser is told how far it fell
        // rather than quietly missing them.
        Err(BroadcastStreamRecvError::Lagged(missed)) => Event::default()
            .event("lagged")
            .json_data(serde_json::json!({ "missed": missed }))
            .unwrap_or_else(|_| Event::default().event("lagged").data("{}")),
    }
}

/// The window a caller asks for, in venue micros — the clock the tape is
/// sorted and dated by.
#[derive(Deserialize, utoipa::IntoParams)]
struct WindowQuery {
    /// Start, inclusive.
    from: i64,
    /// End, exclusive.
    to: i64,
    /// The most rows to return. Defaults to `tape::DEFAULT_LIMIT`.
    limit: Option<usize>,
    /// One instrument, or every instrument in the window.
    ///
    /// Matched whole — `BTC` is not `BTCUSD`. Omitted means every, which is
    /// what a table of the newest rows wants by default.
    ticker: Option<String>,
}

/// One dataset, over a window, as the durable bound permits.
///
/// **Decimals arrive as strings.** See `tape.rs`: serialising with
/// `arrow-json` would send them unquoted, and `JSON.parse` would round every
/// price before anything could decline to.
#[utoipa::path(
    get,
    path = "/v1/tape/{kind}",
    params(("kind" = String, Path, description = "quotes, trades, candles, funding, marks or gaps"), WindowQuery),
    responses(
        (status = 200, description = "The window's rows, and the durable bound", body = tape::View),
        (status = 400, description = "An unknown dataset, or a window that runs backwards", body = String),
    ),
)]
async fn tape_view(
    State(tower): State<Tower>,
    UrlPath(kind): UrlPath<String>,
    Query(window): Query<WindowQuery>,
) -> Response {
    let kind = match tape::kind_of(&kind) {
        Ok(kind) => kind,
        Err(refusal) => return (StatusCode::BAD_REQUEST, refusal.to_string()).into_response(),
    };
    // A listing walks the store and parquet is decoded, so this does not belong
    // on the async executor.
    let root = tower.tape.clone();
    let read = tokio::task::spawn_blocking(move || {
        tape::view(
            &root,
            kind,
            window.from,
            window.to,
            window.limit.unwrap_or(tape::DEFAULT_LIMIT),
            window.ticker,
        )
    })
    .await;
    match read {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(refusal)) => (StatusCode::BAD_REQUEST, refusal.to_string()).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the read did not finish: {join}"),
        )
            .into_response(),
    }
}

/// What the record says is missing, and why.
///
/// **A gap is never inferred from silence.** This reports intervals the record
/// states are gaps; absent rows are not one, and nothing here turns them into
/// one.
#[utoipa::path(
    get,
    path = "/v1/gaps",
    params(WindowQuery),
    responses(
        (status = 200, description = "Gaps in the window, by cause", body = tape::Coverage),
        (status = 400, description = "A window that runs backwards", body = String),
    ),
)]
async fn gaps(State(tower): State<Tower>, Query(window): Query<WindowQuery>) -> Response {
    // Parquet is decoded here, so it does not belong on the async executor.
    let root = tower.tape.clone();
    let read =
        tokio::task::spawn_blocking(move || tape::coverage(&root, window.from, window.to)).await;
    match read {
        Ok(Ok(coverage)) => Json(coverage).into_response(),
        Ok(Err(refusal)) => (StatusCode::BAD_REQUEST, refusal.to_string()).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the read did not finish: {join}"),
        )
            .into_response(),
    }
}

/// Every instrument the record holds, and when each was last seen.
///
/// **The record, not the bus.** The instruments panel read live status and
/// nothing else until 2026-09-22, so the one surface naming the instruments
/// was the one that could not answer without a broker — in a binary whose
/// whole sentence is *it watches the record, not the worker*. The roadmap's
/// exit condition for this tier asks for *"six instruments, their ages"*, and
/// it showed zero against a tape holding all six.
#[utoipa::path(
    get,
    path = "/v1/instruments",
    responses((status = 200, description = "Every instrument the tape holds, newest first", body = tape::Instruments)),
)]
async fn instruments(State(tower): State<Tower>) -> Response {
    // Parquet is decoded here, so it does not belong on the async executor.
    let root = tower.tape.clone();
    match tokio::task::spawn_blocking(move || tape::instruments(&root)).await {
        Ok(found) => Json(found).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the read did not finish: {join}"),
        )
            .into_response(),
    }
}

/// What the record says it could not parse.
///
/// **A failure is not a gap.** A gap is a known absence with a cause and
/// bounds, and `/v1/gaps` reports it. A failure is a payload that ARRIVED and
/// produced no row — the record looks complete and the rows are simply not
/// there. The archive has written these since Tier 1 and nothing has ever
/// read them back.
#[utoipa::path(
    get,
    path = "/v1/failures",
    responses((status = 200, description = "What the record could not parse, by error", body = failures::Failures)),
)]
async fn failures(State(tower): State<Tower>) -> Response {
    // A walk and, where there is anything to read, parquet. Not the executor's.
    let root = tower.archive.clone();
    match tokio::task::spawn_blocking(move || failures::failures(&root)).await {
        Ok(found) => Json(found).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the read did not finish: {join}"),
        )
            .into_response(),
    }
}

/// How much of each day the record holds.
///
/// **The question nothing else answered**: *is this day usable?* That a day
/// exists, that it wants compacting, and that some time is missing across the
/// whole record are three different facts, and none of them is this one.
///
/// Every figure is in our clock. `recv_micros` is what the partitions are
/// dated by, what a gap's bounds are written in, and what *did we have this
/// data* means — the predecessor puts it in one line: *"recv_micros is what
/// coverage, gaps and latency are measured in."*
#[utoipa::path(
    get,
    path = "/v1/coverage",
    responses((status = 200, description = "How much of each day the record covers, newest first", body = tape::Covered)),
)]
async fn coverage(State(tower): State<Tower>) -> Response {
    let root = tower.tape.clone();
    match tokio::task::spawn_blocking(move || tape::covered_days(&root)).await {
        Ok(found) => Json(found).into_response(),
        Err(join) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the read did not finish: {join}"),
        )
            .into_response(),
    }
}

/// The document, described once by the routes that answer it.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "galata-tower",
        description = "A read API over the galata-datawatch record. It watches the record, not the worker.",
    ),
    components(schemas(About, Partition, Overdue, Board, BrokerState, TapeMoved))
)]
struct Contract;

/// The routes, and the document that comes from them.
///
/// **One declaration.** `OpenApiRouter` takes each path from the route itself,
/// so there is no second list to fall out of step with this one.
fn router(tower: Tower) -> (Router, utoipa::openapi::OpenApi) {
    OpenApiRouter::with_openapi(Contract::openapi())
        .routes(routes!(about))
        .routes(routes!(partitions))
        .routes(routes!(overdue))
        .routes(routes!(status))
        .routes(routes!(tape_view))
        .routes(routes!(gaps))
        .routes(routes!(instruments))
        .routes(routes!(failures))
        .routes(routes!(coverage))
        .with_state(tower)
        .split_for_parts()
}

/// The document as it is committed, byte for byte.
///
/// Printed by the binary that serves the routes, never by a separate example:
/// the predecessor's fixtures came from a `cargo run` in a different
/// repository, and that distance is what let them go stale.
fn dump_openapi() -> Result<String, Box<dyn std::error::Error>> {
    let (status, _) = broadcast::channel(STATUS_BACKLOG);
    let (_, api) = router(Tower {
        archive: PathBuf::from("."),
        tape: PathBuf::from("."),
        status,
        board: Arc::new(RwLock::new(BTreeMap::new())),
        broker: Arc::default(),
        bounds: Arc::default(),
    });
    Ok(serde_json::to_string_pretty(&api)? + "\n")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Before the subscriber: this prints a document and exits, and a log line
    // on stdout would corrupt it.
    if std::env::args().nth(1).as_deref() == Some("--dump-openapi") {
        print!("{}", dump_openapi()?);
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // One value, from the environment, with a default that matches what a
    // datawatch deployment writes. Nothing is invented: if the directory is not
    // there, the listing is empty and the screen says so.
    let archive = std::env::var("GALATA_ARCHIVE").unwrap_or_else(|_| "var/archive".to_owned());
    let (status, _) = broadcast::channel(STATUS_BACKLOG);
    let board = Arc::new(RwLock::new(BTreeMap::new()));
    let broker_state: Arc<RwLock<BrokerState>> = Arc::default();
    let bounds: Arc<RwLock<BTreeMap<String, i64>>> = Arc::default();
    // Beside the archive by default, which is how datawatch lays them out.
    let tape = std::env::var("GALATA_TAPE").unwrap_or_else(|_| "var/tape".to_owned());
    let tower = Tower {
        archive: PathBuf::from(archive),
        tape: PathBuf::from(tape),
        status: status.clone(),
        board: Arc::clone(&board),
        broker: Arc::clone(&broker_state),
        bounds: Arc::clone(&bounds),
    };

    // Loopback by default, like everything else here: reaching another machine
    // is a deployment decision, made by setting this.
    let broker = std::env::var("GALATA_BROKER").unwrap_or_else(|_| "127.0.0.1:4222".to_owned());
    subscribe_to_status(broker, status.clone(), board, broker_state);
    // The record's own liveness, which does not depend on a bus at all.
    watch_the_record(tower.tape.clone(), status, bounds);

    // Everything else is the screen, which routes itself.
    let (app, _) = router(tower.clone());
    let app = app.fallback(screen);

    // Loopback by default. Serving other machines is a deployment decision,
    // and it is made by setting this rather than by the binary assuming it.
    let addr: SocketAddr = std::env::var("GALATA_TOWER_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8777".to_owned())
        .parse()?;
    tracing::info!(%addr, archive = %tower.archive.display(), "galata-tower listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests construct one now: the handler is a generator, and the
    // error type is what event_for matches on.
    use tokio_stream::StreamExt as _;
    use tokio_stream::wrappers::BroadcastStream;

    fn snapshot(venue: &str) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            subject: format!("status.{venue}"),
            venue: venue.to_owned(),
            body: serde_json::json!({ "venue": venue }),
        })
    }

    /// The event carries the snapshot, and the browser can tell it apart from a gap.
    #[test]
    fn a_snapshot_arrives_as_a_status_event() {
        let rendered = format!("{:?}", event_for(Ok(Live::Status(snapshot("hyperliquid")))));
        assert!(rendered.contains("status"), "{rendered}");
        assert!(rendered.contains("hyperliquid"), "{rendered}");
    }

    /// What a browser is handed the moment it connects.
    #[test]
    fn a_board_frame_carries_every_venue_seen() {
        let mut board = BTreeMap::new();
        board.insert("hyperliquid".to_owned(), snapshot("hyperliquid"));
        board.insert("rh-chain".to_owned(), snapshot("rh-chain"));

        let rendered = format!(
            "{:?}",
            board_event(&board, BrokerState::default(), BTreeMap::new())
        );
        assert!(rendered.contains("board"), "{rendered}");
        assert!(rendered.contains("hyperliquid"), "{rendered}");
        assert!(rendered.contains("rh-chain"), "{rendered}");
    }

    /// An empty board is still a frame, so a browser can tell "nothing has
    /// published" from "not connected".
    #[test]
    fn an_empty_board_is_sent_rather_than_nothing() {
        let rendered = format!(
            "{:?}",
            board_event(&BTreeMap::new(), BrokerState::default(), BTreeMap::new())
        );
        assert!(rendered.contains("board"), "{rendered}");
    }

    /// **Insert or replace, never remove.** A venue that goes quiet keeps its
    /// last snapshot, because absence after presence is the statement an
    /// operator most needs rendered.
    #[tokio::test]
    async fn a_quiet_venue_stays_in_the_board() {
        let board: Arc<RwLock<BTreeMap<String, Arc<Snapshot>>>> = Arc::default();
        board
            .write()
            .await
            .insert("hyperliquid".to_owned(), snapshot("hyperliquid"));
        // Another venue publishes; the first says nothing further.
        board
            .write()
            .await
            .insert("rh-chain".to_owned(), snapshot("rh-chain"));

        let held = board.read().await;
        assert_eq!(held.len(), 2, "a quiet venue must not be dropped");
        assert!(held.contains_key("hyperliquid"));

        let rendered = format!(
            "{:?}",
            board_event(&held, BrokerState::default(), BTreeMap::new())
        );
        assert!(
            rendered.contains("hyperliquid"),
            "and it must still reach a browser connecting later: {rendered}"
        );
    }

    /// **A gap is an event, never an absence**, and it says how far.
    #[tokio::test]
    async fn a_reader_that_falls_behind_is_told_how_many_it_missed() {
        // Capacity 1 so the lag is forced rather than waited for.
        let (tx, rx) = broadcast::channel::<Live>(1);
        for venue in ["a", "b", "c", "d"] {
            let _ = tx.send(Live::Status(snapshot(venue)));
        }
        let mut stream = BroadcastStream::new(rx);
        let first = stream.next().await.expect("the receiver yields");
        let rendered = format!("{:?}", event_for(first));

        assert!(
            rendered.contains("lagged"),
            "a receiver behind by three must be told, not quietly skipped: {rendered}"
        );
        assert!(
            rendered.contains("missed"),
            "and the event must carry the count: {rendered}"
        );
    }

    /// **The defect this change exists for.** A browser connecting during an
    /// outage was handed an empty list and rendered "no venue has published
    /// status yet" — which is a different statement from "there is no broker",
    /// and was the false one.
    #[test]
    fn a_board_frame_during_an_outage_says_there_is_no_broker() {
        let rendered = format!(
            "{:?}",
            board_event(
                &BTreeMap::new(),
                BrokerState {
                    connected: false,
                    attempts: 7,
                    refusal: Some("connection refused".to_owned()),
                },
                BTreeMap::new(),
            )
        );
        assert!(
            rendered.contains(r#"\"connected\":false"#),
            "the frame must carry the broker's state, not just the venues: {rendered}"
        );
        assert!(
            rendered.contains(r#"\"attempts\":7"#),
            "and the attempt count, because that is what makes a wrong password visible: {rendered}"
        );
        assert!(
            rendered.contains("connection refused"),
            "and the broker's own words: {rendered}"
        );
    }

    /// The change is its own event, so the browser dispatches on the name
    /// rather than inspecting the payload.
    #[test]
    fn a_broker_change_arrives_as_its_own_event() {
        let rendered = format!(
            "{:?}",
            event_for(Ok(Live::Broker(BrokerState {
                connected: true,
                attempts: 0,
                refusal: None,
            })))
        );
        assert!(rendered.contains("broker"), "{rendered}");
        assert!(rendered.contains(r#"\"connected\":true"#), "{rendered}");
    }

    /// The whole point of the ladder: an absent broker is retried often at
    /// first and rarely later, and thirty seconds is the ceiling.
    #[test]
    fn the_wait_climbs_and_then_stops_climbing() {
        let mut wait = FIRST_RETRY;
        let mut seen = vec![wait];
        for _ in 0..10 {
            wait = (wait * 2).min(LONGEST_RETRY);
            seen.push(wait);
        }
        assert_eq!(
            seen[0],
            Duration::from_secs(1),
            "a boot race costs a second"
        );
        assert_eq!(seen[1], Duration::from_secs(2), "{seen:?}");
        assert_eq!(
            *seen.last().expect("ten doublings"),
            LONGEST_RETRY,
            "and it settles at the ceiling rather than growing without bound: {seen:?}"
        );
        assert!(
            seen.iter().all(|w| *w <= LONGEST_RETRY),
            "nothing above the ceiling: {seen:?}"
        );
    }

    /// **Jitter, and it has to actually vary.** A `jittered` that returned its
    /// argument would pass every other test here; this is the one that fails.
    #[test]
    fn the_wait_is_jittered_within_a_quarter_either_side() {
        let wait = Duration::from_secs(8);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let got = jittered(wait);
            assert!(
                got >= Duration::from_secs(6) && got <= Duration::from_secs(10),
                "a quarter either side of eight seconds, got {got:?}"
            );
            seen.insert(got);
            // Enough for the clock's nanoseconds to move.
            std::thread::yield_now();
        }
        assert!(
            seen.len() > 1,
            "two hundred waits that are all identical are not jitter: {seen:?}"
        );
    }

    /// A sub-millisecond wait has no room for a band; it must not divide by zero.
    #[test]
    fn a_wait_too_small_to_jitter_is_still_a_wait() {
        let got = jittered(Duration::from_micros(500));
        assert!(got <= Duration::from_millis(1), "{got:?}");
    }

    fn bounds(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    /// The ordinary case, and the one that must cost nothing: a quiet record
    /// sends no event, so a browser reads no rows.
    #[test]
    fn a_record_that_has_not_moved_says_nothing() {
        let seen = bounds(&[("quotes", 68286), ("candles", 12)]);
        assert!(
            moves(&seen, &seen).is_empty(),
            "an unchanged bound must not wake every open screen"
        );
    }

    /// A kind appearing for the first time is a move: the screen has never
    /// drawn it.
    #[test]
    fn a_kind_that_appears_is_a_move() {
        let moved = moves(&BTreeMap::new(), &bounds(&[("quotes", 1)]));
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].kind, "quotes");
        assert_eq!(moved[0].bound, 1);
    }

    /// Only the kind that moved, so a quotes write does not redraw candles.
    #[test]
    fn only_the_kind_that_moved_is_reported() {
        let moved = moves(
            &bounds(&[("quotes", 10), ("candles", 5)]),
            &bounds(&[("quotes", 11), ("candles", 5)]),
        );
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert_eq!(moved[0].kind, "quotes");
    }

    /// **Backwards is not a move.** The store was replaced underneath the
    /// tower; that is a log line, not a redraw of every chart.
    #[test]
    fn a_bound_that_went_backwards_is_not_a_move() {
        let moved = moves(&bounds(&[("quotes", 100)]), &bounds(&[("quotes", 40)]));
        assert!(
            moved.is_empty(),
            "a store replaced underneath the tower must not read as growth: {moved:?}"
        );
    }

    /// A kind that stops being present is not reported as anything. The screen
    /// keeps what it drew, which is the same *once seen, never dropped* rule
    /// the venue board holds.
    #[test]
    fn a_kind_that_disappears_is_not_reported() {
        let moved = moves(&bounds(&[("quotes", 10)]), &BTreeMap::new());
        assert!(moved.is_empty(), "{moved:?}");
    }

    /// The frame carries where the record STANDS, so a browser connecting into
    /// a quiet hour does not wait for a move that may never come.
    #[test]
    fn a_board_frame_carries_each_kind_s_bound() {
        let rendered = format!(
            "{:?}",
            board_event(
                &BTreeMap::new(),
                BrokerState::default(),
                bounds(&[("quotes", 68286)]),
            )
        );
        assert!(rendered.contains("bounds"), "{rendered}");
        assert!(rendered.contains("68286"), "{rendered}");
    }

    /// Its own event name, so the browser dispatches rather than inspects.
    #[test]
    fn a_move_arrives_as_a_tape_event() {
        let rendered = format!(
            "{:?}",
            event_for(Ok(Live::Tape(TapeMoved {
                kind: "quotes".to_owned(),
                bound: 7,
            })))
        );
        assert!(rendered.contains("tape"), "{rendered}");
        assert!(rendered.contains("quotes"), "{rendered}");
    }

    /// **Asking is an answer, never a failure.** `view` refuses an unwritten
    /// kind, correctly, because its caller asked for rows; this caller asked
    /// whether anything had arrived, and *no* is the answer. A watch that
    /// returned an error here would log one a second for ever.
    #[test]
    fn a_root_with_no_tape_answers_rather_than_failing() {
        let nowhere = PathBuf::from("/galata-tower-no-such-tape-root");
        assert!(
            tape::bounds(&nowhere).is_empty(),
            "a root that is not there must be silence, not a refusal"
        );
    }

    /// **The day is named by the same function that names the partitions.**
    /// A second way of formatting it would not fail when it drifted, it would
    /// disagree — which is precisely what `"9999-99-99"` did, every day.
    #[test]
    fn today_is_spelled_the_way_a_partition_is() {
        let today = today_utc();
        assert_eq!(
            today.len(),
            10,
            "YYYY-MM-DD, as a date= level holds it: {today}"
        );
        assert_eq!(
            today,
            galata_datawatch::date_of(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
                    .unwrap_or(0)
            ),
            "one calendar, not two"
        );
    }

    /// **Lexicographic is chronological**, which is the one thing the
    /// `YYYY-MM-DD` partition format buys — and it is what `overdue_closed`
    /// relies on to decide *strictly before*.
    #[test]
    fn the_day_order_the_listing_depends_on_holds() {
        // Around a month end and a year end, where a shorter format breaks.
        assert!("2026-09-21" < "2026-09-22");
        assert!("2026-09-30" < "2026-10-01");
        assert!("2026-12-31" < "2027-01-01");
        // And the boundary itself: today is NOT strictly before today.
        assert!(!("2026-09-22" < "2026-09-22"));
    }

    /// The sentinel this change removed made every day closed. Held so it
    /// cannot come back looking harmless.
    #[test]
    fn no_day_can_precede_the_sentinel_that_was_here() {
        // Why `"9999-99-99"` listed the day being written: every real date
        // sorts before it, so *strictly before* selected everything.
        assert!(today_utc().as_str() < "9999-99-99");
        assert!("2026-09-22" < "9999-99-99");
    }

    /// One segment is a compacted partition, so the default must not name it.
    #[test]
    fn the_default_threshold_is_what_compaction_leaves() {
        assert_eq!(
            COMPACTED_TO, 1,
            "compaction leaves one segment, and `overdue_closed` keeps count > max"
        );
    }

    /// The archive this tower is pointed at, where there is one.
    fn archive_root() -> Option<PathBuf> {
        let root = PathBuf::from("../../../galata-datawatch/var/archive");
        root.is_dir().then_some(root)
    }

    /// **The claim this change exists for, against the real store.**
    ///
    /// Before it, this archive listed six partitions dated the day still being
    /// captured, under a heading reading *Closed*. Checked against a real
    /// directory rather than against the code that filters it, because the
    /// defect was in what was HANDED to that filter.
    #[test]
    fn no_partition_dated_today_is_ever_listed() {
        let Some(root) = archive_root() else {
            eprintln!("SKIPPED: no archive to read");
            return;
        };
        let today = today_utc();
        let listed = galata_segments::overdue_closed(&root, &today, COMPACTED_TO);
        for (path, segments) in &listed {
            assert!(
                !path
                    .display()
                    .to_string()
                    .contains(&format!("date={today}")),
                "{} is today's partition and is still being written, yet it is \
                 listed as closed with {segments} segments",
                path.display()
            );
        }
        // And the filter is doing something: every row named is over the
        // threshold. A listing that silently returned nothing would pass the
        // assertion above while proving nothing.
        for (path, segments) in &listed {
            assert!(
                *segments > COMPACTED_TO,
                "{} has {segments}",
                path.display()
            );
        }
    }

    /// **The roadmap's exit condition for this tier**, on the point it was
    /// failing: *"shows six instruments, their ages"*. The panel read live
    /// status and nothing else, so against a tape holding all six it showed
    /// zero whenever no capture was running — which is the ordinary state of
    /// a tower reading an archive.
    #[test]
    fn the_record_names_its_instruments_without_a_broker() {
        let Some(root) = std::path::PathBuf::from("../../../galata-datawatch/var/tape")
            .is_dir()
            .then(|| std::path::PathBuf::from("../../../galata-datawatch/var/tape"))
        else {
            eprintln!("SKIPPED: no tape to read");
            return;
        };
        let found = tape::instruments(&root);
        let tickers: std::collections::BTreeSet<&str> = found
            .instruments
            .iter()
            .map(|i| i.ticker.as_str())
            .collect();
        assert!(
            tickers.len() >= 6,
            "the tape holds six instruments and nothing here needs a bus to say so: {tickers:?}"
        );
        assert!(found.bound > 0, "and the durable bound comes with them");
        // Every row counted stands behind something.
        assert!(found.instruments.iter().all(|i| i.rows > 0));
    }

    /// A dataset whose rows carry no venue time is still an instrument the
    /// record holds. `marks` is exactly that — thousands of rows and no
    /// `at_micros` — and it renders an em dash rather than a fabricated age.
    #[test]
    fn an_instrument_with_no_venue_time_still_counts_its_rows() {
        let nowhere = PathBuf::from("/galata-tower-no-such-tape-root");
        let found = tape::instruments(&nowhere);
        assert!(
            found.instruments.is_empty(),
            "a root that is not there is silence, not a refusal"
        );
        assert_eq!(found.bound, 0);
    }

    /// `say` writes both: the stored state for whoever connects next, and the
    /// event for whoever is connected now. One without the other is how a
    /// state stream quietly becomes an event stream.
    #[tokio::test]
    async fn a_broker_change_is_both_recorded_and_announced() {
        let broker: Arc<RwLock<BrokerState>> = Arc::default();
        let (tx, mut rx) = broadcast::channel::<Live>(4);
        say(
            &broker,
            &tx,
            BrokerState {
                connected: false,
                attempts: 3,
                refusal: Some("no route to host".to_owned()),
            },
        )
        .await;

        let held = broker.read().await.clone();
        assert_eq!(held.attempts, 3, "the state a later browser is handed");
        assert!(!held.connected);

        match rx.try_recv().expect("the connected browser is told too") {
            Live::Broker(state) => assert_eq!(state.attempts, 3),
            other => panic!("expected a broker event, got {other:?}"),
        }
    }
}
