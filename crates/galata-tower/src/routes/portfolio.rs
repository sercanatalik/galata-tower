//! Portfolio routes: report passthrough and statistics derivation.

use axum::extract::{Query, State};
use axum::response::Response;

use crate::routes::{blocking_json, blocking_json_infallible};
use crate::{portfolio, Tower};

/// The ledger's fold report for a venue, passed through as the ledger wrote it.
///
/// Accounts by alias only: the tower never reads the ledger's root, holds no
/// address and no key. An absent report is an empty state, not an error.
#[utoipa::path(
    get,
    path = "/v1/portfolio",
    params(portfolio::PortfolioQuery),
    responses(
        (status = 200, description = "The fold report and its age, or why there is none", body = portfolio::Portfolio),
    ),
)]
pub(crate) async fn portfolio_report(
    State(tower): State<Tower>,
    Query(query): Query<portfolio::PortfolioQuery>,
) -> Response {
    let dir = tower.reports.clone();
    blocking_json_infallible(move || portfolio::read_report(&dir, &query.venue)).await
}

/// Volatility, correlation and beta derived from the tape on request, each
/// with its n and backfilled share. The floor and z have no defaults.
#[utoipa::path(
    get,
    path = "/v1/statistics",
    params(portfolio::StatisticsQuery),
    responses(
        (status = 200, description = "The derivation, with the tape bound it read to", body = portfolio::Derived),
        (status = 400, description = "A missing floor or z, an unknown horizon, or a window that runs backwards", body = String),
    ),
)]
pub(crate) async fn statistics(
    State(tower): State<Tower>,
    Query(query): Query<portfolio::StatisticsQuery>,
) -> Response {
    let tape = tower.tape.clone();
    blocking_json(move || portfolio::derive(&tape, &query)).await
}
