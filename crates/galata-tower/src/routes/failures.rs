//! Failures route: what the record could not parse.

use axum::extract::State;
use axum::response::Response;

use crate::routes::blocking_json_infallible;
use crate::{Tower, failures};

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
pub(crate) async fn failures(State(tower): State<Tower>) -> Response {
    let root = tower.archive.clone();
    blocking_json_infallible(move || failures::failures(&root)).await
}
