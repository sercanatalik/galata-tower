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

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use rust_embed::Embed;
use serde::Serialize;
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

/// Where the record lives, and the address to serve on.
#[derive(Clone, Debug)]
struct Tower {
    /// The archive root: `var/archive` in a datawatch deployment.
    archive: PathBuf,
}

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
        .with_state(tower)
        .split_for_parts()
}

/// The document as it is committed, byte for byte.
///
/// Printed by the binary that serves the routes, never by a separate example:
/// the predecessor's fixtures came from a `cargo run` in a different
/// repository, and that distance is what let them go stale.
fn dump_openapi() -> Result<String, Box<dyn std::error::Error>> {
    let (_, api) = router(Tower {
        archive: PathBuf::from("."),
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
    let tower = Tower {
        archive: PathBuf::from(archive),
    };

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
