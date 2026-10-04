//! Route handlers, grouped by domain.
//!
//! Each sub-module holds the handlers for one area of the API. The shared
//! helpers [`blocking_json`] and [`blocking_json_infallible`] remove the
//! `spawn_blocking` + match boilerplate that every handler repeats.

pub(crate) mod about;
pub(crate) mod failures;
pub(crate) mod portfolio;
pub(crate) mod shape;
pub(crate) mod signals;
pub(crate) mod status;
pub(crate) mod tape;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// A read that panicked or was cancelled.
pub(crate) fn unfinished(join: tokio::task::JoinError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("the read did not finish: {join}"),
    )
        .into_response()
}

/// A blocking read that succeeds or fails with a refusal.
///
/// The closure runs on a blocking thread. A `TapeError` (or any `Display`)
/// becomes a `400`; a panic or cancellation becomes a `500` via
/// [`unfinished`].
pub(crate) async fn blocking_json<T, E>(
    f: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Response
where
    T: Serialize + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(found)) => Json(found).into_response(),
        Ok(Err(refusal)) => (StatusCode::BAD_REQUEST, refusal.to_string()).into_response(),
        Err(join) => unfinished(join),
    }
}

/// A blocking read that cannot fail with a refusal.
///
/// The closure runs on a blocking thread. A panic or cancellation becomes a
/// `500` via [`unfinished`].
pub(crate) async fn blocking_json_infallible<T>(f: impl FnOnce() -> T + Send + 'static) -> Response
where
    T: Serialize + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(found) => Json(found).into_response(),
        Err(join) => unfinished(join),
    }
}
