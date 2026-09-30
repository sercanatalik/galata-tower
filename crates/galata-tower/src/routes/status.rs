//! Status SSE and latest prices routes.

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use tokio_stream::Stream;

use crate::routes::blocking_json_infallible;
use crate::{board_event, event_for, Tower};
use std::time::Duration;

/// Every venue's status, as it arrives.
///
/// SSE rather than a WebSocket: the traffic is one-directional, and the
/// browser's `EventSource` reconnects, backs off and honours `retry:` without a
/// line of client code.
#[utoipa::path(
    get,
    path = "/v1/status",
    responses((status = 200, description = "A server-sent event stream: a board frame on connect, then status, broker, tape and lagged events", body = crate::Snapshot)),
)]
pub(crate) async fn status(
    State(tower): State<Tower>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut rx = tower.status.subscribe();
    let broker = tower.broker.read().await.clone();
    let bounds = tower.bounds.read().await.clone();
    let opening = board_event(&*tower.board.read().await, broker, bounds);

    let stream = async_stream::stream! {
        yield Ok(opening);
        loop {
            match rx.recv().await {
                Ok(live) => yield Ok(event_for(Ok(live))),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    yield Ok(event_for(Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(missed))));
                    let broker = tower.broker.read().await.clone();
                    let bounds = tower.bounds.read().await.clone();
                    yield Ok(board_event(&*tower.board.read().await, broker, bounds));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Every venue's newest prices, read from the archive's tail.
///
/// Not the tape, which may be days behind, and not the bus, which the tower may not read market data from.
#[utoipa::path(
    get,
    path = "/v1/latest",
    responses((status = 200, description = "The newest quote and trade per instrument, from the archive", body = crate::latest::Latest)),
)]
pub(crate) async fn latest_prices(State(tower): State<Tower>) -> Response {
    let frontier = tower.frontiers.read().await.archive.clone();
    let (archive, normalisers, shared) = (
        tower.archive.clone(),
        std::sync::Arc::clone(&tower.normalisers),
        std::sync::Arc::clone(&tower.latest),
    );
    blocking_json_infallible(move || {
        (*shared.get(|| crate::latest::latest(&archive, &frontier, &normalisers))).clone()
    })
    .await
    .map(|response| response)
}
