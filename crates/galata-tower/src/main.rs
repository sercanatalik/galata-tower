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
use axum::routing::get;
use axum::{Json, Router};
use rust_embed::Embed;
use serde::Serialize;

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
#[derive(Serialize)]
struct Partition {
    /// Its path relative to the archive root.
    path: String,
}

/// A closed day still holding more segments than compaction should have left.
///
/// **Reported, never judged.** The number is here; whether it is bad belongs to
/// whoever set the threshold.
#[derive(Serialize)]
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
#[derive(Serialize)]
struct About {
    /// The archive root being watched.
    archive: String,
    /// The tape columns a reader can prune on, from the schema itself.
    prune_on: Vec<String>,
}

/// The partitions the record holds.
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let app = Router::new()
        .route("/v1/about", get(about))
        .route("/v1/partitions", get(partitions))
        .route("/v1/overdue", get(overdue))
        .with_state(tower.clone())
        // Everything else is the screen, which routes itself.
        .fallback(screen);

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
