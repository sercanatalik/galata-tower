//! Reading the tape, and serialising it without rounding it.
//!
//! # Why not `arrow-json`
//!
//! Every price and size in the tape is `Decimal128(38, 18)` — the type chosen
//! because `f64` loses precision, held in galata-datawatch by a guard that
//! forbids a float column in the vocabulary.
//!
//! `arrow-json`'s writer sends decimals through `RawArrayFormatter`, described
//! in that crate's own source as *"a newtype wrapper around `JsonArrayFormatter`
//! that skips surrounding the value with `\"`"*. The formatted value is right;
//! emitting it unquoted is what makes it a JSON **number**, which `JSON.parse`
//! turns into a double. Every price would be rounded before any code on the far
//! side could decline to round it.
//!
//! And the browser-side guard would not see it. `check-no-float-money.sh`
//! forbids `Number(` and `parseFloat(` outside `money.ts`, and there would be
//! neither: the float arrives already made. That blind spot — a float that was
//! never converted here because it was never a string there — is why this
//! module exists rather than a call to `arrow_json::ArrayWriter`.
//!
//! `ArrayFormatter` is still what formats. It produces the decimal string
//! without going through a float, and it is what `arrow-json` uses. Only the
//! quoting changes.

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, RecordBatch};
use arrow::datatypes::DataType;
use arrow::util::display::{ArrayFormatter, FormatOptions};
use galata_wire::Kind;
use serde::Serialize;
use serde_json::{Map, Value};

/// The kinds the tape holds.
///
/// The strings are `Kind::as_str`'s — *"the discriminator written to disk and
/// to a subject"* — so a rename there reaches here. Only the membership is
/// stated, and `galata_datawatch::tape::schema::schema_for` is what confirms
/// it: a kind with a schema is a kind the tape can hold.
///
/// **A kind with a schema is served unless this says otherwise.** This was
/// five entries with no statement of why those five, and `gaps` — the one
/// dataset the whole tree exists to write — was missing from it by omission
/// rather than by decision. The remaining absences are deliberate:
/// `Kind::Book` has a schema and no rows in any tape here; `Unparsed` and
/// `Reorgs` are bytes and chain surgery rather than a dataset a screen reads.
/// Adding one is adding a line here.
const SERVED: [Kind; 6] = [
    Kind::Quotes,
    Kind::Trades,
    Kind::Candles,
    Kind::Funding,
    Kind::Marks,
    Kind::Gaps,
];

/// Why a read was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TapeError {
    /// A dataset the tape does not hold.
    #[error("{asked} is not a dataset this tape holds; it holds {known}")]
    UnknownKind {
        /// What was asked for.
        asked: String,
        /// What there is.
        known: String,
    },
    /// A cap of no rows.
    ///
    /// **A refusal, in the manner of a backwards window.** Asking for nothing
    /// is a question that should not have been asked.
    #[error("a limit of zero asks for no rows")]
    NoRows,
    /// A window whose end is not after its start.
    ///
    /// **A refusal, not an empty list.** An empty array is an answer; a
    /// backwards window is a question that should not have been asked.
    #[error("the window runs backwards: {from} to {to}")]
    Backwards {
        /// Start.
        from: i64,
        /// End.
        to: i64,
    },
    /// The store could not be read.
    #[error("the tape could not be read: {detail}")]
    Unreadable {
        /// What the reader said.
        detail: String,
    },
    /// The dataset exists and has written nothing yet.
    ///
    /// **Distinct from unreadable, because the store distinguishes them.**
    /// `tape::reader::unwritten` exists so a caller can tell *nothing has
    /// happened yet* from *this scope is missing while the others are live*,
    /// and flattening the two would throw away the difference the store went
    /// to trouble to keep.
    #[error("the tape holds no {kind} yet")]
    NothingWritten {
        /// Which dataset.
        kind: String,
    },
    /// A column type this tower cannot render.
    ///
    /// **Loudly, rather than wrongly.** A type rendered by guesswork is how a
    /// figure comes to disagree with the venue's.
    #[error("column {column} is {data_type}, which this tower does not render")]
    Unrenderable {
        /// Which column.
        column: String,
        /// Its type.
        data_type: String,
    },
}

/// Every served kind's durable bound, for the kinds that have written anything.
///
/// **The cheap half.** `Reader::open` plus `bound()` reads parquet footers and
/// nothing else: 53µs against the real tape, where decoding the whole of it
/// into a view is 2.85ms. That ratio is the whole reason the tower watches this
/// on every browser's behalf instead of each browser asking — see
/// `examples/cost-of-a-bound.rs`, which is how those two numbers were got and
/// is kept runnable so the next person can check them rather than trust them.
///
/// A kind that has written nothing is absent from the map. That is an answer,
/// not a failure: [`view`] refuses it because a caller asked for ROWS, and this
/// caller asked whether anything had arrived.
pub fn bounds(root: &Path) -> BTreeMap<String, i64> {
    let mut found = BTreeMap::new();
    for kind in SERVED {
        let scope = format!("kind={}", kind.as_str());
        let scopes = [scope.as_str()];
        if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
            continue;
        }
        // A root that is not there, or a store that will not open, is silence
        // here. The caller decides whether silence is worth a log line; doing
        // it here would do it once a second.
        if let Ok(reader) = galata_datawatch::tape::reader::Reader::open(root, &scopes) {
            found.insert(kind.as_str().to_owned(), reader.bound().position);
        }
    }
    found
}

/// A kind, from the name the tape writes for it.
pub fn kind_of(name: &str) -> Result<Kind, TapeError> {
    SERVED
        .iter()
        .find(|kind| kind.as_str() == name)
        .copied()
        .ok_or_else(|| TapeError::UnknownKind {
            asked: name.to_owned(),
            known: SERVED
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        })
}

/// The most rows a read returns when the caller names no limit.
///
/// **Stated, not derived, and it does NOT fit a minute.** An earlier draft of
/// this comment claimed it did; a sixty-second window of this tape is 1,343
/// rows, and a test written on that claim failed. At roughly 290 bytes a row
/// the cap is about 290 KiB — the point is to bound an unbounded read, not to
/// accommodate any particular window. A caller who wants a minute asks for it
/// and gets it; a caller who forgets gets 290 KiB instead of eleven megabytes.
///
/// It applies whether or not it is asked for, because a protection that must be
/// requested protects nobody: the caller who forgets is the caller who needed it.
pub const DEFAULT_LIMIT: usize = 1_000;

/// A window's rows, and how far the store is durable.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct View {
    /// The dataset.
    pub kind: String,
    /// How far the store has durably written, in the tape's own sequence.
    ///
    /// **Returned with the rows on purpose.** Forty rows from a quiet hour and
    /// forty rows from a store that stopped there look identical without it.
    pub bound: i64,
    /// How many rows the window holds, before any cap.
    ///
    /// **Always present, not only when a cap applied.** A field that appears
    /// conditionally is one callers learn to ignore, and this is the only thing
    /// distinguishing a capped read from a quiet window.
    pub total: usize,
    /// The rows, each a map of column to value. Decimals are strings.
    ///
    /// When a cap applies these are the NEWEST in the window — a reader
    /// watching a tape wants its end — and `total` is what makes that
    /// truncation visible rather than inferred.
    #[schema(value_type = Vec<Object>)]
    pub rows: Vec<Value>,
}

/// One value, typed for JSON.
///
/// The formatter does the formatting; this decides the JSON shape. A decimal
/// becomes a **string**, which is the whole point of the module.
fn value_at(
    data_type: &DataType,
    column: &str,
    array: &dyn Array,
    formatter: &ArrayFormatter<'_>,
    row: usize,
) -> Result<Value, TapeError> {
    if array.is_null(row) {
        return Ok(Value::Null);
    }
    let rendered = formatter.value(row).to_string();
    Ok(match data_type {
        // THE POINT. Quoted, so the browser parses text and `money.ts` decides
        // what to do with it.
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => Value::String(rendered),
        // Counts, sequences and timestamps: a JSON number is exact for these
        // up to 2^53, and every one of them here is far below it.
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => rendered
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or(Value::String(rendered)),
        DataType::Boolean => Value::Bool(rendered == "true"),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Value::String(rendered),
        DataType::Timestamp(_, _) | DataType::Date32 | DataType::Date64 => Value::String(rendered),
        other => {
            return Err(TapeError::Unrenderable {
                column: column.to_owned(),
                data_type: other.to_string(),
            });
        }
    })
}

/// The newest `limit` rows, formatted.
///
/// Only the tail is formatted. The batches are walked from the end until
/// enough rows are in hand, so a cap of forty over a window of forty thousand
/// does the work of forty.
fn newest(batches: &[RecordBatch], limit: usize) -> Result<Vec<Value>, TapeError> {
    let total: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    let skip = total.saturating_sub(limit);
    let mut seen = 0usize;
    let mut tail: Vec<&RecordBatch> = Vec::new();
    let mut offset_in_first = 0usize;
    for batch in batches {
        let next = seen + batch.num_rows();
        if next > skip {
            if tail.is_empty() {
                offset_in_first = skip.saturating_sub(seen);
            }
            tail.push(batch);
        }
        seen = next;
    }
    let mut out = rows(&tail.into_iter().cloned().collect::<Vec<_>>())?;
    if offset_in_first > 0 && offset_in_first <= out.len() {
        out.drain(..offset_in_first);
    }
    Ok(out)
}

/// Batches as rows, decimals as strings.
pub fn rows(batches: &[RecordBatch]) -> Result<Vec<Value>, TapeError> {
    let options = FormatOptions::new().with_display_error(true);
    let mut out = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        let formatters = batch
            .columns()
            .iter()
            .map(|column| ArrayFormatter::try_new(column, &options))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| TapeError::Unreadable {
                detail: error.to_string(),
            })?;
        for row in 0..batch.num_rows() {
            let mut object = Map::new();
            for (index, field) in schema.fields().iter().enumerate() {
                let value = value_at(
                    field.data_type(),
                    field.name(),
                    batch.column(index).as_ref(),
                    &formatters[index],
                    row,
                )?;
                object.insert(field.name().clone(), value);
            }
            out.push(Value::Object(object));
        }
    }
    Ok(out)
}

/// A window of one dataset, as the durable bound permits.
pub fn view(root: &Path, kind: Kind, from: i64, to: i64, limit: usize) -> Result<View, TapeError> {
    if limit == 0 {
        return Err(TapeError::NoRows);
    }
    if to <= from {
        return Err(TapeError::Backwards { from, to });
    }
    // A SCOPE IS A DIRECTORY DIRECTLY UNDER THE ROOT, so it is `kind=quotes`
    // and not `quotes` -- the same spelling `examples/superseded.rs` uses.
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];

    // Asked before opening, because `Bound::of` refuses either way and this is
    // the only place the two can be told apart. A dataset that has written
    // nothing is not an unreadable store.
    if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
        return Err(TapeError::NothingWritten {
            kind: kind.as_str().to_owned(),
        });
    }

    // Opened per request, so the bound is what is durable NOW rather than at
    // boot: a tower that had been running a day would otherwise serve a day-old
    // ceiling.
    let reader = galata_datawatch::tape::reader::Reader::open(root, &scopes).map_err(|error| {
        TapeError::Unreadable {
            detail: error.to_string(),
        }
    })?;
    let window = galata_datawatch::tape::reader::Window {
        kind,
        from_micros: from,
        to_micros: to,
    };
    let batches = reader.view(window).map_err(|error| TapeError::Unreadable {
        detail: error.to_string(),
    })?;
    // Counted with `num_rows`, which is a field read. The formatting loop is
    // the expensive part — 46ms for a whole tape against 6ms to decode it — so
    // the total costs approximately nothing, which is what makes it affordable
    // to report on every read.
    let total: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    Ok(View {
        kind: kind.as_str().to_owned(),
        bound: reader.bound().position,
        total,
        rows: newest(&batches, limit)?,
    })
}

/// What one cause accounts for, over a window.
#[derive(Debug, Serialize, utoipa::ToSchema)]
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
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Coverage {
    /// The window's start, as asked for.
    pub from: i64,
    /// Its end.
    pub to: i64,
    /// Gap rows folded. Reported so a caller can see what the window held.
    pub rows: usize,
    /// The tape's durable bound, as every read here reports it.
    pub bound: i64,
    /// One entry per cause found, most time missing first.
    pub causes: Vec<Cause>,
}

/// One gap row's five interesting columns, read straight from arrow.
struct Rows<'a> {
    series: &'a arrow::array::StringArray,
    ticker: &'a arrow::array::StringArray,
    cause: &'a arrow::array::StringArray,
    clipped: &'a arrow::array::StringArray,
    from: &'a arrow::array::Int64Array,
    to: &'a arrow::array::Int64Array,
}

impl<'a> Rows<'a> {
    /// **Typed columns, not formatted values.** [`rows`] renders every cell
    /// through `ArrayFormatter`, which is 46ms for a whole tape and the
    /// dominant cost of a tape read. The fold compares five columns and needs
    /// no string it does not compare.
    fn of(batch: &'a RecordBatch) -> Result<Rows<'a>, TapeError> {
        fn text<'b>(
            batch: &'b RecordBatch,
            name: &str,
        ) -> Result<&'b arrow::array::StringArray, TapeError> {
            batch
                .column_by_name(name)
                .and_then(|c| c.as_any().downcast_ref::<arrow::array::StringArray>())
                .ok_or_else(|| TapeError::Unrenderable {
                    column: name.to_owned(),
                    data_type: "expected Utf8".to_owned(),
                })
        }
        fn number<'b>(
            batch: &'b RecordBatch,
            name: &str,
        ) -> Result<&'b arrow::array::Int64Array, TapeError> {
            batch
                .column_by_name(name)
                .and_then(|c| c.as_any().downcast_ref::<arrow::array::Int64Array>())
                .ok_or_else(|| TapeError::Unrenderable {
                    column: name.to_owned(),
                    data_type: "expected Int64".to_owned(),
                })
        }
        Ok(Rows {
            series: text(batch, "series")?,
            ticker: text(batch, "ticker")?,
            cause: text(batch, "cause")?,
            clipped: text(batch, "clipped")?,
            from: number(batch, "from_micros")?,
            to: number(batch, "to_micros")?,
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
fn union_micros(spans: &mut [(i64, i64)]) -> (i64, usize, i64, i64) {
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
/// The predecessor's sentence is the design: *"gaps by cause, coverage by day
/// are `SELECT`s the tape answers; the tower asks the reader, it does not keep
/// a second copy of the record in a browser."* The response is bounded by the
/// number of distinct causes however many rows were folded.
pub fn coverage(root: &Path, from: i64, to: i64) -> Result<Coverage, TapeError> {
    if to <= from {
        return Err(TapeError::Backwards { from, to });
    }
    let kind = Kind::Gaps;
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];

    // A tape that has written no gaps is not an unreadable store, and it is
    // not an error either: nothing missing is the good answer.
    if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
        return Ok(Coverage {
            from,
            to,
            rows: 0,
            bound: 0,
            causes: Vec::new(),
        });
    }

    let reader = galata_datawatch::tape::reader::Reader::open(root, &scopes).map_err(|error| {
        TapeError::Unreadable {
            detail: error.to_string(),
        }
    })?;
    let batches = reader
        .view(galata_datawatch::tape::reader::Window {
            kind,
            from_micros: from,
            to_micros: to,
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
        bound: reader.bound().position,
        causes,
    })
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

    /// A window that runs backwards is refused here as `view` refuses it.
    #[test]
    fn a_backwards_window_is_refused_by_the_summary_too() {
        let nowhere = Path::new("/galata-tower-no-such-tape-root");
        assert!(matches!(
            coverage(nowhere, 500, 100),
            Err(TapeError::Backwards { .. })
        ));
    }

    /// A tape that has written no gaps is not a failure: nothing missing is
    /// what an operator hopes to read.
    #[test]
    fn a_tape_with_no_gaps_reports_none() {
        let nowhere = Path::new("/galata-tower-no-such-tape-root");
        let found = coverage(nowhere, 0, i64::MAX).expect("silence is an answer");
        assert_eq!(found.rows, 0);
        assert!(found.causes.is_empty());
    }

    use super::*;

    /// The tape galata-tape-rebuild writes beside the archive.
    fn tape_root() -> Option<std::path::PathBuf> {
        let root = std::path::PathBuf::from("../../../galata-datawatch/var/tape");
        root.is_dir().then_some(root)
    }

    /// **The claim this module exists for**, checked against real rows rather
    /// than against the code that writes them.
    ///
    /// `arrow-json` would send these unquoted and `JSON.parse` would round
    /// them. Nothing on the browser side could catch it: the guard there
    /// forbids `Number(` and `parseFloat(`, and neither would appear, because
    /// the float would arrive already made.
    #[test]
    fn a_price_crosses_as_a_string() {
        let Some(root) = tape_root() else {
            eprintln!(
                "SKIPPED: no tape at ../galata-datawatch/var/tape. \
                 Run `galata-tape-rebuild hyperliquid <date>` there to make one."
            );
            return;
        };
        // The whole of 2026-09-20 in venue micros, and well past it.
        let view =
            view(&root, Kind::Quotes, 0, i64::MAX / 2, DEFAULT_LIMIT).expect("the tape reads");
        assert!(!view.rows.is_empty(), "the tape held no quotes to check");

        let row = view.rows[0].as_object().expect("a row is an object");
        let price = row
            .keys()
            .find(|k| k.contains("price") || k.contains("bid") || k.contains("ask"))
            .unwrap_or_else(|| {
                panic!(
                    "no price-like column in {:?}",
                    row.keys().collect::<Vec<_>>()
                )
            });

        assert!(
            row[price].is_string(),
            "{price} must be a JSON string, not a number: got {}",
            row[price]
        );
        // And the right string: a value, not a rounded one.
        let text = row[price].as_str().unwrap();
        assert!(
            text.parse::<f64>().is_ok(),
            "{price} must still look like a number inside the quotes: {text}"
        );
    }

    /// **A capped read says what it left out.** Forty rows back, and a total
    /// that is very much larger — which is the only thing distinguishing this
    /// from a quiet window.
    #[test]
    fn a_capped_read_reports_the_total_it_capped() {
        let Some(root) = tape_root() else {
            eprintln!("SKIPPED: no tape");
            return;
        };
        let capped = view(&root, Kind::Quotes, 0, i64::MAX / 2, 40).expect("the tape reads");
        assert_eq!(capped.rows.len(), 40, "the cap applies");
        assert!(
            capped.total > 40,
            "and the total says what was left out: {}",
            capped.total
        );

        // The newest, not the oldest: a reader watching a tape wants its end.
        let all = view(&root, Kind::Quotes, 0, i64::MAX / 2, usize::MAX).expect("reads");
        assert_eq!(
            all.total, capped.total,
            "the total does not depend on the cap"
        );
        let last = all.rows.last().expect("rows");
        assert_eq!(
            capped.rows.last(),
            Some(last),
            "a capped read must end where the window ends"
        );
    }

    /// An uncapped-but-small window reports a total equal to what it returned.
    #[test]
    fn an_uncapped_window_reports_what_it_returned() {
        let Some(root) = tape_root() else {
            eprintln!("SKIPPED: no tape");
            return;
        };
        // FIVE seconds. Sixty was the first attempt and it is 1,343 rows —
        // above the default cap, which is how the comment claiming a minute
        // fits was found to be wrong.
        let view = view(
            &root,
            Kind::Quotes,
            1_789_938_867_000_000,
            1_789_938_872_000_000,
            DEFAULT_LIMIT,
        )
        .expect("reads");
        assert!(!view.rows.is_empty(), "the slice held rows");
        assert!(
            view.rows.len() < DEFAULT_LIMIT,
            "the window must be under the cap for this to test anything: {}",
            view.rows.len()
        );
        assert_eq!(view.total, view.rows.len(), "nothing was capped");
    }

    /// A cap of nothing is a refusal, like a backwards window.
    #[test]
    fn a_limit_of_zero_is_refused() {
        let root = std::path::PathBuf::from(".");
        let refused = view(&root, Kind::Quotes, 0, 100, 0).expect_err("zero must refuse");
        assert!(matches!(refused, TapeError::NoRows), "got {refused:?}");
    }

    /// A backwards window is refused, not answered with an empty list.
    #[test]
    fn a_backwards_window_is_refused() {
        let root = std::path::PathBuf::from(".");
        let refused =
            view(&root, Kind::Quotes, 100, 50, DEFAULT_LIMIT).expect_err("backwards must refuse");
        assert!(
            matches!(refused, TapeError::Backwards { .. }),
            "got {refused:?}"
        );
    }

    /// An unknown dataset names what there is.
    #[test]
    fn an_unknown_kind_says_what_the_tape_holds() {
        let refused = kind_of("nonsense").expect_err("unknown must refuse");
        let said = refused.to_string();
        assert!(said.contains("nonsense"), "{said}");
        assert!(said.contains("quotes"), "and what there is: {said}");
    }
}
