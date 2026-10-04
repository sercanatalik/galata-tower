//! Background tasks: the NATS status subscription and the record watcher.
//!
//! These run for the tower's lifetime, independent of any request. They share
//! state with the request handlers through the [`Tower`]'s `Arc<RwLock<...>>`
//! fields.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use galata_broker::{BrokerIdentity, NatsSubscriber};
use tokio::sync::{RwLock, broadcast};

use crate::{BrokerState, Live, Snapshot, TapeMoved, Frontiers, tape, shape};

/// One second, then two, then four, to thirty.
///
/// The ceiling matters more than the floor. A race at boot should cost about a
/// second, and an hour of absence should not be an hour of connect attempts;
/// thirty seconds is slow enough to be free and quick enough that a returning
/// broker is picked up before anyone reloads the page.
pub(crate) const FIRST_RETRY: Duration = Duration::from_secs(1);
pub(crate) const LONGEST_RETRY: Duration = Duration::from_secs(30);

/// How often the record is asked whether it moved.
///
/// **Below the compactor's own cadence, deliberately.** The tape is durable
/// parquet; measuring it more often samples a file that has not been rewritten.
/// One second at 53µs a kind is 265µs a second for the five served kinds —
/// which is the measurement that made a server-side watch the obvious shape
/// rather than a thing each browser does.
const TAPE_WATCH: Duration = Duration::from_secs(1);

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
pub(crate) fn subscribe_to_status(
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

/// The wait, give or take a quarter of itself.
///
/// **Jitter, for the reason this tree already gives in the capture's reconnect
/// path**: retries that fall into lockstep with a broker restarting on a timer
/// miss it every time, and several towers coming back together should not
/// arrive as one. It comes from the clock rather than from `rand`, because a
/// dependency for ±25% on a retry delay is a dependency for nothing — this
/// needs successive waits to differ, not to be unguessable.
pub(crate) fn jittered(wait: Duration) -> Duration {
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
pub(crate) async fn say(
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
pub(crate) fn watch_the_record(
    tape: PathBuf,
    archive: PathBuf,
    status: broadcast::Sender<Live>,
    bounds: Arc<RwLock<tape::Bounds>>,
    frontiers: Arc<RwLock<Frontiers>>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TAPE_WATCH);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last = tape::Bounds::new();
        // Reported on the EDGE. A tape root that is not there is one log line,
        // not one a second for as long as the tower runs.
        let mut said_empty = false;
        // **One label cache for the watch's lifetime.** A bound reads each
        // segment's venue label from its footer; measured at 25.7 ms per kind
        // cold against 3.3 ms warm for 1,000 segments. A segment is immutable
        // once renamed, so a label read once stays read.
        let labels = Arc::new(galata_datawatch::tape::LabelCache::default());
        // And one frontier cache: a segment's newest arrival is decoded the
        // tick it appears and never again, where recomputing the frontier
        // from scratch re-read the whole history every time the tape moved.
        let frontier = Arc::new(shape::FrontierCache::default());
        let mut ticks: u64 = 0;
        loop {
            ticker.tick().await;
            ticks += 1;
            // Off the runtime: this opens files. Short is not the same as
            // non-blocking, and five of them share one thread with every open
            // SSE stream.
            let root = tape.clone();
            let cache = Arc::clone(&labels);
            let prune = ticks.is_multiple_of(60);
            let Ok(found) = tokio::task::spawn_blocking(move || {
                // Once a minute, forget segments compaction or replacement
                // removed. It only bounds memory: a removed segment is never
                // listed, so its entry is never consulted.
                if prune {
                    cache.prune();
                }
                tape::bounds(&root, &cache)
            })
            .await
            else {
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

            let tape_moved = moves(&last, &found);
            let refresh_tape = ticks == 1 || !tape_moved.is_empty();
            for moved in tape_moved {
                let _ = status.send(Live::Tape(moved));
            }
            last = found.clone();
            *bounds.write().await = found;

            // The archive's frontier is read from names alone; the tape's only when the tape moved.
            let (archive_root, tape_root) = (archive.clone(), tape.clone());
            let fc = Arc::clone(&frontier);
            let Ok((arrived, tape_edge)) = tokio::task::spawn_blocking(move || {
                let tape_edge = refresh_tape.then(|| shape::tape_frontier(&tape_root, &fc));
                (shape::archive_frontier(&archive_root), tape_edge)
            })
            .await
            else {
                continue;
            };
            let mut held = frontiers.write().await;
            for (venue, &edge) in &arrived {
                if held.archive.get(venue) != Some(&edge) {
                    let _ = status.send(Live::Archive(crate::ArchiveMoved {
                        venue: venue.clone(),
                        frontier_micros: edge,
                    }));
                }
            }
            held.archive = arrived;
            if let Some(edge) = tape_edge {
                held.tape = edge;
            }
        }
    });
}

/// Which kinds moved, given what was last seen and what is there now.
///
/// Separated from the watch so it can be held by tests: a loop that only ever
/// runs against a real tape is a loop whose backwards case is reasoned about
/// rather than exercised, and the backwards case is the one that would redraw
/// every chart if it were wrong.
pub(crate) fn moves(last: &tape::Bounds, found: &tape::Bounds) -> Vec<TapeMoved> {
    let mut moved = Vec::new();
    for (kind, venues) in found {
        for (venue, position) in venues {
            match last.get(kind).and_then(|seen| seen.get(venue)) {
                // Unchanged. The ordinary case, and it sends nothing.
                Some(previous) if previous == position => {}
                // **Backwards is not a move.** It means the store was replaced
                // underneath the tower, which is worth a line in the log and is
                // not worth a chart redraw.
                Some(previous) if previous > position => tracing::warn!(
                    %kind, %venue, previous, position,
                    "the tape's bound went backwards; the store was replaced"
                ),
                _ => moved.push(TapeMoved {
                    kind: kind.clone(),
                    venue: venue.clone(),
                    bound: *position,
                }),
            }
        }
    }
    moved
}
