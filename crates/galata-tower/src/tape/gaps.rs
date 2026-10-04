//! Gap coverage: what the record says is missing, and why.
//!
//! The predecessor's sentence is the design: *"gaps by cause, coverage by day
//! are `SELECT`s the tape answers; the tower asks the reader, it does not keep
//! a second copy of the record in a browser."*

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, Int64Array, RecordBatch, StringArray};
use galata_wire::Kind;
use serde::Serialize;
use utoipa::ToSchema;

use crate::tape::TapeError;

/// What one cause accounts for, over a window.
#[derive(Debug, Serialize, ToSchema)]
pub struct Cause {
    /// The cause, as the capture wrote it — `downtime`, `crash_unflushed`.
    pub cause: String,
    /// **The union of this cause's intervals**, in venue micros.
    ///
    /// Not the sum of its rows. The tape writes one row per affected
    /// `(ticker, series)`, so one outage arrives two dozen times; summing them
    /// reported 32.6 days missing from a 32.6-hour window on the archive this
    /// was measured against.
    pub missing_micros: i64,
    /// How many distinct wall-clock intervals, after merging.
    pub intervals: usize,
    /// How many rows said so.
    ///
    /// Beside the duration, never instead of it. Twenty-four rows and one
    /// interval is not noise — it is how many instrument-series the outage
    /// touched, which is a fact about the outage and not a second measure of
    /// its length.
    pub rows: usize,
    /// The earliest start and the latest end, so a screen can place it.
    pub first_micros: i64,
    /// The latest end.
    pub last_micros: i64,
    /// Every `clipped` value seen, counted.
    ///
    /// **How loose the bound is.** A gap bounded by two observed sequence
    /// numbers and one bounded by a restart are different claims about what
    /// the record knows; the schema keeps this field so a consumer can tell,
    /// and a summary that dropped it would be the consumer that could not.
    pub clipped: BTreeMap<String, usize>,
    /// Which series it touched.
    pub series: Vec<String>,
    /// How many distinct tickers.
    pub tickers: usize,
}

/// The record's gaps over a window, by cause.
#[derive(Debug, Serialize, ToSchema)]
pub struct Coverage {
    /// The window's start, as asked for.
    pub from: i64,
    /// Its end.
    pub to: i64,
    /// Gap rows folded. Reported so a caller can see what the window held.
    pub rows: usize,
    /// The tape's durable bound per venue, as every read here reports it.
    pub bound: BTreeMap<String, i64>,
    /// One entry per cause found, most time missing first.
    pub causes: Vec<Cause>,
}

/// One gap row's five interesting columns, read straight from arrow.
struct Rows<'a> {
    series: &'a StringArray,
    ticker: &'a StringArray,
    cause: &'a StringArray,
    clipped: &'a StringArray,
    from: &'a Int64Array,
    to: &'a Int64Array,
}

impl<'a> Rows<'a> {
    /// **Typed columns, not formatted values.** [`super::rows`] renders every
    /// cell through `ArrayFormatter`, which is 46ms for a whole tape and the
    /// dominant cost of a tape read. The fold compares five columns and needs
    /// no string it does not compare.
    fn of(batch: &'a RecordBatch) -> Result<Rows<'a>, TapeError> {
        Ok(Rows {
            series: crate::column!(batch, "series", StringArray, "Utf8")?,
            ticker: crate::column!(batch, "ticker", StringArray, "Utf8")?,
            cause: crate::column!(batch, "cause", StringArray, "Utf8")?,
            clipped: crate::column!(batch, "clipped", StringArray, "Utf8")?,
            from: crate::column!(batch, "from_micros", Int64Array, "Int64")?,
            to: crate::column!(batch, "to_micros", Int64Array, "Int64")?,
        })
    }
}

/// What one cause is accumulating, before it is merged.
#[derive(Default)]
struct Pile {
    spans: Vec<(i64, i64)>,
    rows: usize,
    clipped: BTreeMap<String, usize>,
    series: std::collections::BTreeSet<String>,
    tickers: std::collections::BTreeSet<String>,
}

/// The union of a cause's intervals, in micros.
///
/// **This is the whole point of the route.** Sort by start and merge anything
/// that touches. One outage written once per `(ticker, series)` is one
/// interval; two disjoint outages are two.
pub fn union_micros(spans: &mut [(i64, i64)]) -> (i64, usize, i64, i64) {
    if spans.is_empty() {
        return (0, 0, 0, 0);
    }
    spans.sort_unstable();
    let first = spans[0].0;
    let mut last = spans[0].1;
    let mut total = 0;
    let mut merged = 0;
    let (mut start, mut end) = spans[0];
    for &(s, e) in spans.iter().skip(1) {
        last = last.max(e);
        if s <= end {
            // Touching or overlapping: one interval, widened.
            end = end.max(e);
        } else {
            total += end - start;
            merged += 1;
            start = s;
            end = e;
        }
    }
    total += end - start;
    merged += 1;
    (total, merged, first, last)
}

/// Gaps by cause, over a window.
///
/// The response is bounded by the number of distinct causes however many rows
/// were folded.
pub fn coverage(root: &Path, from: i64, to: i64) -> Result<Coverage, TapeError> {
    if to <= from {
        return Err(TapeError::Backwards { from, to });
    }
    let kind = Kind::Gaps;
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];

    // A tape that has written no gaps is not an unreadable store, and it is
    // not an error either: nothing missing is the good answer. The reader
    // refuses an unwritten scope with its own name for it, so the refusal IS
    // that answer — asking `unwritten` first only walked the kind twice.
    let reader = match galata_datawatch::tape::reader::Reader::open(root, &scopes) {
        Ok(reader) => reader,
        Err(galata_datawatch::tape::reader::ReadError::NoFrontier { .. }) => {
            return Ok(Coverage {
                from,
                to,
                rows: 0,
                bound: BTreeMap::new(),
                causes: Vec::new(),
            });
        }
        Err(error) => {
            return Err(TapeError::Unreadable {
                detail: error.to_string(),
            });
        }
    };
    let batches = reader
        .view(galata_datawatch::tape::reader::Window {
            kind,
            from_micros: from,
            to_micros: to,
            ticker: None,
        })
        .map_err(|error| TapeError::Unreadable {
            detail: error.to_string(),
        })?;

    let mut piles: BTreeMap<String, Pile> = BTreeMap::new();
    let mut seen = 0usize;
    for batch in &batches {
        let columns = Rows::of(batch)?;
        for i in 0..batch.num_rows() {
            seen += 1;
            let pile = piles.entry(columns.cause.value(i).to_owned()).or_default();
            pile.spans
                .push((columns.from.value(i), columns.to.value(i)));
            pile.rows += 1;
            *pile
                .clipped
                .entry(columns.clipped.value(i).to_owned())
                .or_default() += 1;
            pile.series.insert(columns.series.value(i).to_owned());
            pile.tickers.insert(columns.ticker.value(i).to_owned());
        }
    }

    let mut causes: Vec<Cause> = piles
        .into_iter()
        .map(|(cause, mut pile)| {
            let (missing_micros, intervals, first_micros, last_micros) =
                union_micros(&mut pile.spans);
            Cause {
                cause,
                missing_micros,
                intervals,
                rows: pile.rows,
                first_micros,
                last_micros,
                clipped: pile.clipped,
                series: pile.series.into_iter().collect(),
                tickers: pile.tickers.len(),
            }
        })
        .collect();
    // Most time missing first: the thing an operator is looking for.
    causes.sort_by_key(|cause| std::cmp::Reverse(cause.missing_micros));

    Ok(Coverage {
        from,
        to,
        rows: seen,
        bound: reader.bound().positions.clone(),
        causes,
    })
}

/// Every gap interval the tape states, bucketed by the day it falls in.
///
/// **A gap across midnight is charged to both days**, clipped at the boundary,
/// because it is missing from both.
pub(crate) fn collect_gaps(root: &Path, out: &mut BTreeMap<(String, String), Vec<(i64, i64)>>) {
    let scope = format!("kind={}", Kind::Gaps.as_str());
    let scopes = [scope.as_str()];
    // No gaps written and gaps unreadable both mean nothing to bucket, and
    // `open` refuses an unwritten scope itself: asking `unwritten` first
    // only walked the whole kind a second time.
    let Ok(reader) = galata_datawatch::tape::reader::Reader::open(root, &scopes) else {
        return;
    };
    let Ok(batches) = reader.view(galata_datawatch::tape::reader::Window {
        kind: Kind::Gaps,
        from_micros: i64::MIN + 1,
        to_micros: i64::MAX,
        ticker: None,
    }) else {
        return;
    };
    for batch in &batches {
        gaps_by_day(batch, out);
    }
}

/// One gaps batch bucketed by day — the per-batch half of [`collect_gaps`],
/// shared with the board's single pass over the tape.
pub(crate) fn gaps_by_day(
    batch: &arrow::array::RecordBatch,
    out: &mut BTreeMap<(String, String), Vec<(i64, i64)>>,
) {
    let venue = crate::column!(batch, "venue", StringArray, "Utf8");
    let from = crate::column!(batch, "from_micros", Int64Array, "Int64");
    let to = crate::column!(batch, "to_micros", Int64Array, "Int64");
    let (Ok(venue), Ok(from), Ok(to)) = (venue, from, to) else {
        return;
    };
    for i in 0..batch.num_rows() {
        if venue.is_null(i) || from.is_null(i) || to.is_null(i) {
            continue;
        }
        for (date, span) in super::coverage::split_by_day(from.value(i), to.value(i)) {
            out.entry((venue.value(i).to_owned(), date))
                .or_default()
                .push(span);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::union_micros;

    /// **The finding the whole route exists for.** The tape writes one row per
    /// affected `(ticker, series)`, so one outage arrives two dozen times.
    /// Against the real archive, summing those rows reported 32.6 DAYS missing
    /// from a 32.6-hour window — arithmetic over real data, absurd on its face,
    /// and it would have shipped.
    #[test]
    fn one_interval_written_many_times_counts_once() {
        let outage = (1_789_941_139_190_783, 1_790_058_447_399_168);
        let mut spans = vec![outage; 24];
        let (missing, intervals, first, last) = union_micros(&mut spans);

        assert_eq!(intervals, 1, "twenty-four rows, one outage");
        assert_eq!(missing, outage.1 - outage.0);
        assert_eq!(first, outage.0);
        assert_eq!(last, outage.1);

        let summed: i64 = spans.iter().map(|(a, b)| b - a).sum();
        assert_eq!(
            summed / missing,
            24,
            "the sum overstates by exactly the number of rows, which is why it is not used"
        );
    }

    /// Two outages that do not touch are two outages.
    #[test]
    fn disjoint_intervals_both_count() {
        let mut spans = vec![(100, 200), (500, 600)];
        let (missing, intervals, first, last) = union_micros(&mut spans);
        assert_eq!(intervals, 2);
        assert_eq!(missing, 200);
        assert_eq!((first, last), (100, 600));
    }

    /// Overlapping time is missing once, not twice.
    #[test]
    fn overlapping_intervals_count_once() {
        let mut spans = vec![(100, 300), (200, 400)];
        let (missing, intervals, ..) = union_micros(&mut spans);
        assert_eq!(intervals, 1);
        assert_eq!(missing, 300, "100..400, not 200 + 200");
    }

    /// Adjacent is one interval: the record is continuous across the join, and
    /// two touching absences are one absence.
    #[test]
    fn adjacent_intervals_merge() {
        let mut spans = vec![(100, 200), (200, 300)];
        let (missing, intervals, ..) = union_micros(&mut spans);
        assert_eq!(intervals, 1);
        assert_eq!(missing, 200);
    }

    /// Out of order in, right answer out — the rows arrive in whatever order
    /// the batches hold them.
    #[test]
    fn order_does_not_matter() {
        let mut forward = vec![(100, 200), (500, 600), (150, 250)];
        let mut backward = vec![(150, 250), (500, 600), (100, 200)];
        assert_eq!(union_micros(&mut forward), union_micros(&mut backward));
        assert_eq!(union_micros(&mut forward).0, 250, "100..250 and 500..600");
    }

    /// No gaps is the good answer, and it is zero rather than an error.
    #[test]
    fn nothing_missing_is_an_answer() {
        assert_eq!(union_micros(&mut Vec::new()), (0, 0, 0, 0));
    }
}
