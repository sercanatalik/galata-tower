//! Shape routes: timeline, candles and board.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::routes::blocking_json;
use crate::{Tower, shape};

/// Which candles to read.
#[derive(Deserialize, IntoParams)]
pub(crate) struct CandleQuery {
    /// The venue.
    pub(crate) venue: String,
    /// The instrument.
    pub(crate) ticker: String,
    /// 1m, 5m, 15m or 1h.
    pub(crate) interval: String,
    /// Start, venue micros, inclusive. Defaults to the beginning.
    pub(crate) from: Option<i64>,
    /// End, venue micros, exclusive. Defaults to the end.
    pub(crate) to: Option<i64>,
}

/// The record over time: held runs, recorded gaps, backfills, and what only the archive holds.
#[utoipa::path(
    get,
    path = "/v1/timeline",
    responses(
        (status = 200, description = "Each venue and dataset as intervals", body = shape::Timeline),
        (status = 400, description = "The tape could not be read", body = String),
    ),
)]
pub(crate) async fn timeline(State(tower): State<Tower>) -> Response {
    let (tape, archive) = (tower.tape.clone(), tower.archive.clone());
    blocking_json(move || shape::timeline(&tape, &archive)).await
}

/// One instrument's candles, folded and resampled on the server.
#[utoipa::path(
    get,
    path = "/v1/candles",
    params(CandleQuery),
    responses(
        (status = 200, description = "Bars, oldest first, each marked if a backfill sent it", body = shape::Candles),
        (status = 400, description = "An unknown interval, or a window that runs backwards", body = String),
    ),
)]
pub(crate) async fn candles(
    State(tower): State<Tower>,
    Query(query): Query<CandleQuery>,
) -> Response {
    let Some(interval) = shape::Interval::parse(&query.interval) else {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "{} is not an interval; ask for one of {}",
                query.interval,
                shape::Interval::NAMES
            ),
        )
            .into_response();
    };
    let tape = tower.tape.clone();
    blocking_json(move || {
        shape::candles(
            &tape,
            &query.venue,
            &query.ticker,
            interval,
            &query.interval,
            query.from.unwrap_or(i64::MIN + 1),
            query.to.unwrap_or(i64::MAX),
        )
    })
    .await
}

/// Every venue and dataset the tape holds, as the Overview's grid.
#[utoipa::path(
    get,
    path = "/v1/board",
    responses(
        (status = 200, description = "Tape rows, archive segments today, coverage and gap rows per venue and dataset", body = shape::Datasets),
        (status = 400, description = "The tape could not be read", body = String),
    ),
)]
pub(crate) async fn board(State(tower): State<Tower>) -> Response {
    let (tape, archive, today) = (
        tower.tape.clone(),
        tower.archive.clone(),
        crate::today_utc(),
    );
    blocking_json(move || shape::board(&tape, &archive, &today)).await
}
