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

use std::net::SocketAddr;
use std::path::PathBuf;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use galata_broker::{BrokerIdentity, NatsSubscriber};
use rust_embed::Embed;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::{Stream, StreamExt};
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// The screen, embedded at compile time.
///
/// **One binary serves the API and the screen**, so a deployment runs no node
/// process. The folder must exist when this compiles, which is why `ui/dist`
/// carries a placeholder in the tree rather than being generated on demand.
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../ui/dist"]
struct Screen;

/// Where the record lives, and the live status every browser shares.
#[derive(Clone)]
struct Tower {
    /// The archive root: `var/archive` in a datawatch deployment.
    archive: PathBuf,
    /// One subscription's snapshots, fanned to every connected screen.
    status: broadcast::Sender<Arc<Snapshot>>,
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

/// Closed days still holding more segments than compaction should have left.
///
/// `max_segments` is the caller's threshold, because what counts as too many is
/// an operator's judgement and this reports rather than judges.
#[utoipa::path(
    get,
    path = "/v1/overdue",
    responses((status = 200, description = "Closed days still holding segments", body = Vec<Overdue>)),
)]
async fn overdue(State(tower): State<Tower>) -> Json<Vec<Overdue>> {
    let root = tower.archive.clone();
    // A listing walks the store, so it does not belong on the async executor.
    let found = tokio::task::spawn_blocking(move || {
        let today = "9999-99-99"; // Every dated partition is closed against this.
        galata_segments::overdue_closed(&root, today, 1)
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
fn subscribe_to_status(addr: String, status: broadcast::Sender<Arc<Snapshot>>) {
    tokio::spawn(async move {
        let identity = BrokerIdentity::new(
            "reader",
            std::env::var(galata_broker::password_var("reader")).unwrap_or_default(),
            galata_broker::password_var("reader"),
        );
        let mut subscriber = match NatsSubscriber::connect(&addr, &identity, "status.>").await {
            Ok(subscriber) => subscriber,
            Err(refusal) => {
                // Named, not silent, and the password is not in it: the
                // identity type cannot print it.
                tracing::warn!(%refusal, "no status stream; the record is still served");
                return;
            }
        };
        tracing::info!("subscribed to status.>");
        while let Some((subject, payload)) = subscriber.next_addressed().await {
            // The venue is the subject's second token. A subject that does not
            // carry one is not a status subject, and is skipped rather than
            // guessed at.
            let Some(venue) = subject.split('.').nth(1) else {
                continue;
            };
            let body = serde_json::from_slice(&payload).unwrap_or(serde_json::Value::Null);
            // `send` fails only when nobody is listening, which is the ordinary
            // case for a tower with no browser open.
            let _ = status.send(Arc::new(Snapshot {
                subject: subject.clone(),
                venue: venue.to_owned(),
                body,
            }));
        }
        tracing::warn!("the status subscription ended");
    });
}

/// Every venue's status, as it arrives.
///
/// SSE rather than a WebSocket: the traffic is one-directional, and the
/// browser's `EventSource` reconnects, backs off and honours `retry:` without a
/// line of client code.
#[utoipa::path(
    get,
    path = "/v1/status",
    responses((status = 200, description = "A server-sent event stream of venue status snapshots", body = Snapshot)),
)]
async fn status(
    State(tower): State<Tower>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    // `BroadcastStream` surfaces the lag as a stream item rather than hiding
    // it, which is the whole reason it is used here instead of a hand-written
    // loop: the gap arrives as something that must be handled.
    let stream =
        BroadcastStream::new(tower.status.subscribe()).map(|received| Ok(event_for(received)));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// One received item as the event the browser sees.
///
/// Separated from the handler so the lag path can be exercised: forcing a real
/// receiver to fall behind through an HTTP connection is awkward, and a branch
/// that is only ever reasoned about is a branch that is not held.
fn event_for(received: Result<Arc<Snapshot>, BroadcastStreamRecvError>) -> Event {
    match received {
        Ok(snapshot) => Event::default()
            .event("status")
            .json_data(&*snapshot)
            .unwrap_or_else(|_| Event::default().event("status").data("{}")),
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

/// The document, described once by the routes that answer it.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "galata-tower",
        description = "A read API over the galata-datawatch record. It watches the record, not the worker.",
    ),
    components(schemas(About, Partition, Overdue))
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
        status,
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
    let tower = Tower {
        archive: PathBuf::from(archive),
        status: status.clone(),
    };

    // Loopback by default, like everything else here: reaching another machine
    // is a deployment decision, made by setting this.
    let broker = std::env::var("GALATA_BROKER").unwrap_or_else(|_| "127.0.0.1:4222".to_owned());
    subscribe_to_status(broker, status);

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
        let rendered = format!("{:?}", event_for(Ok(snapshot("hyperliquid"))));
        assert!(rendered.contains("status"), "{rendered}");
        assert!(rendered.contains("hyperliquid"), "{rendered}");
    }

    /// **A gap is an event, never an absence**, and it says how far.
    #[tokio::test]
    async fn a_reader_that_falls_behind_is_told_how_many_it_missed() {
        // Capacity 1 so the lag is forced rather than waited for.
        let (tx, rx) = broadcast::channel::<Arc<Snapshot>>(1);
        for venue in ["a", "b", "c", "d"] {
            let _ = tx.send(snapshot(venue));
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
}
