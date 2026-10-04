//! Tape reading routes: windows, gaps, instruments, coverage and rates.

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::Tower;
use crate::routes::{blocking_json, blocking_json_infallible};
use crate::tape;

/// The window a caller asks for, in venue micros — the clock the tape is
/// sorted and dated by.
#[derive(Deserialize, IntoParams)]
pub(crate) struct WindowQuery {
    /// Start, inclusive.
    pub(crate) from: i64,
    /// End, exclusive.
    pub(crate) to: i64,
    /// The most rows to return. Defaults to `tape::DEFAULT_LIMIT`.
    pub(crate) limit: Option<usize>,
    /// One instrument, or every instrument in the window.
    ///
    /// Matched whole — `BTC` is not `BTCUSD`. Omitted means every, which is
    /// what a table of the newest rows wants by default.
    pub(crate) ticker: Option<String>,
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
pub(crate) async fn tape_view(
    State(tower): State<Tower>,
    UrlPath(kind): UrlPath<String>,
    Query(window): Query<WindowQuery>,
) -> Response {
    let kind = match tape::kind_of(&kind) {
        Ok(kind) => kind,
        Err(refusal) => return (StatusCode::BAD_REQUEST, refusal.to_string()).into_response(),
    };
    let root = tower.tape.clone();
    blocking_json(move || {
        tape::view(
            &root,
            kind,
            window.from,
            window.to,
            window.limit.unwrap_or(tape::DEFAULT_LIMIT),
            window.ticker,
        )
    })
    .await
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
pub(crate) async fn gaps(
    State(tower): State<Tower>,
    Query(window): Query<WindowQuery>,
) -> Response {
    let root = tower.tape.clone();
    blocking_json(move || tape::coverage(&root, window.from, window.to)).await
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
pub(crate) async fn instruments(State(tower): State<Tower>) -> Response {
    let root = tower.tape.clone();
    blocking_json_infallible(move || tape::instruments(&root)).await
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
pub(crate) async fn coverage(State(tower): State<Tower>) -> Response {
    let root = tower.tape.clone();
    blocking_json_infallible(move || tape::covered_days(&root)).await
}

/// What a caller may narrow the hourly counts by.
#[derive(Deserialize, IntoParams)]
pub(crate) struct RatesQuery {
    /// The most hour-buckets to return, newest first. Defaults to
    /// [`tape::DEFAULT_HOURS`].
    pub(crate) hours: Option<usize>,
}

/// How many rows the record holds, by hour.
///
/// **The failure nothing else here sees.** A socket that stays open and
/// delivers a trickle records no gap, leaves coverage at its full window, and
/// keeps every instrument's last-seen current — *"the TCP connection remains
/// nominally established, nothing arrives."* The partial case is worse than
/// the total one, because the total one is a gap.
///
/// No baseline, no threshold, no flag. What a normal hour holds for a venue is
/// the operator's knowledge, and an hour beside its neighbours is a shape they
/// can read.
#[utoipa::path(
    get,
    path = "/v1/rates",
    params(RatesQuery),
    responses((status = 200, description = "Rows per venue, dataset and hour, newest first", body = tape::Rates)),
)]
pub(crate) async fn rates(State(tower): State<Tower>, Query(query): Query<RatesQuery>) -> Response {
    let root = tower.tape.clone();
    let limit = query.hours.unwrap_or(tape::DEFAULT_HOURS);
    blocking_json_infallible(move || tape::rates(&root, limit)).await
}
