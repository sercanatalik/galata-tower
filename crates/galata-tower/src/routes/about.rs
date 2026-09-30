//! About, partitions and overdue routes.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;
use crate::layout;
use crate::routes::unfinished;
use crate::{Partition, Root, Tower};

/// What a caller may narrow the overdue listing by.
#[derive(Deserialize, IntoParams)]
pub(crate) struct OverdueQuery {
    /// The most segments a closed partition may hold. Defaults to
    /// `COMPACTED_TO`. Zero lists every closed partition holding anything,
    /// which shows the shape of the store rather than only its problems.
    pub(crate) max_segments: Option<usize>,
}

/// How many segments a closed partition may hold before it is worth naming.
///
/// **One, and that is not arbitrary.** Compaction leaves one segment per
/// partition, so *more than one* is exactly *not compacted since it closed*.
/// Anything above this would be a judgement about how much neglect is
/// acceptable, which is the operator's — which is why it is a parameter.
pub(crate) const COMPACTED_TO: usize = 1;

/// What this tower reads.
#[utoipa::path(
    get,
    path = "/v1/about",
    responses((status = 200, description = "The archive root and the tape's prune columns", body = crate::About)),
)]
pub(crate) async fn about(State(tower): State<Tower>) -> Response {
    let (tape, cache) = (tower.tape.clone(), std::sync::Arc::clone(&tower.layout));
    let tape_problems =
        match tokio::task::spawn_blocking(move || layout::problems(&tape, &cache, layout::check))
            .await
        {
            Ok(found) => found,
            Err(join) => return unfinished(join),
        };
    Json(crate::About {
        archive: Root::of(&tower.archive, "GALATA_ARCHIVE"),
        tape: Root::of(&tower.tape, "GALATA_TAPE"),
        prune_on: galata_datawatch::tape::schema::PRUNE_ON
            .iter()
            .map(|column| (*column).to_owned())
            .collect(),
        tape_problems,
        frontiers: tower.frontiers.read().await.by_venue(),
    })
    .into_response()
}

/// The partitions the record holds.
///
/// A walk that cannot run is a refusal naming the root, never an empty list.
#[utoipa::path(
    get,
    path = "/v1/partitions",
    responses(
        (status = 200, description = "Every partition in the archive", body = Vec<Partition>),
        (status = 500, description = "The archive root could not be listed", body = String),
    ),
)]
pub(crate) async fn partitions(State(tower): State<Tower>) -> Response {
    let root = tower.archive.clone();
    let listing = std::sync::Arc::clone(&tower.listing);
    let walked = tokio::task::spawn_blocking(move || {
        std::fs::read_dir(&root)?;
        let found = crate::cached_partitions(&root, &listing)
            .into_iter()
            .map(|dir| {
                let segments = galata_segments::list_segments(&dir).len();
                (dir, segments)
            })
            .collect::<Vec<_>>();
        Ok::<_, std::io::Error>(found)
    })
    .await;
    let found = match walked {
        Ok(Ok(found)) => found,
        Ok(Err(refusal)) => return refuse_root(&tower.archive, &refusal.to_string()),
        Err(join) => return refuse_root(&tower.archive, &join.to_string()),
    };
    Json(
        found
            .into_iter()
            .map(|(path, segments)| Partition {
                path: path
                    .strip_prefix(&tower.archive)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
                segments,
            })
            .collect::<Vec<_>>(),
    )
    .into_response()
}

/// A 500 that names the root and what went wrong.
fn refuse_root(root: &std::path::Path, why: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!(
            "the archive root {} could not be listed: {why}",
            root.display()
        ),
    )
        .into_response()
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
    responses((status = 200, description = "Closed days still holding segments", body = Vec<crate::Overdue>)),
)]
pub(crate) async fn overdue(
    State(tower): State<Tower>,
    Query(query): Query<OverdueQuery>,
) -> Json<Vec<crate::Overdue>> {
    let root = tower.archive.clone();
    let today = crate::today_utc();
    let max_segments = query.max_segments.unwrap_or(COMPACTED_TO);
    let found = tokio::task::spawn_blocking(move || {
        galata_segments::overdue_closed(&root, &today, max_segments)
    })
    .await
    .unwrap_or_default();
    Json(
        found
            .into_iter()
            .map(|(path, segments)| crate::Overdue {
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
