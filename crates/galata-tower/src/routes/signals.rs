//! Signal routes: latest stored figures and history.

use axum::extract::{Query, State};
use axum::response::Response;

use crate::routes::{blocking_json, blocking_json_infallible};
use crate::{signals, Tower};
use std::time::{SystemTime, UNIX_EPOCH};

/// Each horizon's newest stored figures for a signal, and whether each is
/// stale by the tower's clock. Nothing is computed from them.
#[utoipa::path(
    get,
    path = "/v1/signals",
    params(signals::SignalsQuery),
    responses(
        (status = 200, description = "Every horizon's newest figures, as stored; an empty list when none is", body = signals::Signals),
    ),
)]
pub(crate) async fn stored_signals(
    State(tower): State<Tower>,
    Query(query): Query<signals::SignalsQuery>,
) -> Response {
    let tape = tower.tape.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or_default();
    blocking_json_infallible(move || signals::latest(&tape, &query.signal, now)).await
}

/// One stored signal's history for one pair and horizon: per asof, its
/// latest computation, as stored. At most 90 days.
#[utoipa::path(
    get,
    path = "/v1/signal-history",
    params(signals::HistoryQuery),
    responses(
        (status = 200, description = "One point per asof, oldest first, a value or why there is none", body = signals::History),
        (status = 400, description = "A window outside 1 to 90 days", body = String),
    ),
)]
pub(crate) async fn signal_history(
    State(tower): State<Tower>,
    Query(query): Query<signals::HistoryQuery>,
) -> Response {
    let tape = tower.tape.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or_default();
    blocking_json(move || signals::history(&tape, &query, now)).await
}
