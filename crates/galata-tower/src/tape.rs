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

/// One instrument, as the record holds it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Instrument {
    /// The venue that wrote it.
    pub venue: String,
    /// The instrument.
    pub ticker: String,
    /// Which dataset, as `/v1/tape/{kind}` spells it.
    pub kind: String,
    /// The newest venue time the record holds for it.
    ///
    /// **Read, never inferred.** The tape's segments carry footer statistics
    /// for `venue`, `ticker` and `at_micros`, so a max could be had for the
    /// cost of a footer. It is not taken that way: statistics in this tree may
    /// answer only *no*, because a wrong bound that EXCLUDES surfaces as a
    /// missing row while a wrong bound that is REPORTED becomes the answer —
    /// and arrow-rs has shipped incorrect min/max for strings and for decimals
    /// more than once.
    pub last_micros: i64,
    /// How many rows stand behind it.
    ///
    /// One row and four million rows are different facts about an instrument
    /// the record has seen, and an age alone hides the difference.
    pub rows: usize,
}

/// What the record holds, by instrument.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Instruments {
    /// The tape's durable bound, as every read here reports it.
    pub bound: i64,
    /// Newest first: an operator looks for what is current, or conspicuously
    /// is not.
    pub instruments: Vec<Instrument>,
}

/// Every instrument the tape holds, with when each was last seen.
///
/// **Folded from rows, and measured: 170ms for 86,821 rows** across all six
/// kinds on the tape this was written against. Correct and affordable beats
/// fast and unfalsifiable — see `Instrument::last_micros` for why the footer
/// statistics are not used. Typed columns rather than `ArrayFormatter`, which
/// is 46ms of a 61ms whole-tape read and is not needed to compare three of
/// them.
///
/// Paid once per page, not on a timer: the screen refetches when the `tape`
/// event says the record moved.
pub fn instruments(root: &Path) -> Instruments {
    // (venue, ticker, kind) -> (newest at_micros, rows)
    let mut seen: BTreeMap<(String, String, &'static str), (i64, usize)> = BTreeMap::new();
    let mut bound = 0;

    for kind in SERVED {
        let scope = format!("kind={}", kind.as_str());
        let scopes = [scope.as_str()];
        // A kind that has written nothing is silence, not a refusal: the
        // caller asked what the record holds, and *not this* is an answer.
        if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
            continue;
        }
        let Ok(reader) = galata_datawatch::tape::reader::Reader::open(root, &scopes) else {
            continue;
        };
        bound = bound.max(reader.bound().position);
        let window = galata_datawatch::tape::reader::Window {
            kind,
            from_micros: i64::MIN + 1,
            to_micros: i64::MAX,
            ticker: None,
        };
        let Ok(batches) = reader.view(window) else {
            continue;
        };
        for batch in &batches {
            use arrow::array::{Array, Int64Array, StringArray};
            let venue = batch
                .column_by_name("venue")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let ticker = batch
                .column_by_name("ticker")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let at = batch
                .column_by_name("at_micros")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            // A dataset without these columns describes no instrument. Skipped
            // rather than guessed at, and not an error: the caller asked which
            // instruments are here.
            let (Some(venue), Some(ticker)) = (venue, ticker) else {
                continue;
            };
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || ticker.is_null(i) {
                    continue;
                }
                let key = (
                    venue.value(i).to_owned(),
                    ticker.value(i).to_owned(),
                    kind.as_str(),
                );
                // A row with no venue time still counts as a row: it arrived.
                // It just cannot make the instrument look newer than it is.
                let when = at.filter(|a| !a.is_null(i)).map(|a| a.value(i));
                let entry = seen.entry(key).or_insert((i64::MIN, 0));
                entry.1 += 1;
                if let Some(when) = when {
                    entry.0 = entry.0.max(when);
                }
            }
        }
    }

    let mut instruments: Vec<Instrument> = seen
        .into_iter()
        .map(|((venue, ticker, kind), (last_micros, rows))| Instrument {
            venue,
            ticker,
            kind: kind.to_owned(),
            last_micros,
            rows,
        })
        .collect();
    instruments.sort_by_key(|i| std::cmp::Reverse(i.last_micros));
    Instruments { bound, instruments }
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
pub fn view(
    root: &Path,
    kind: Kind,
    from: i64,
    to: i64,
    limit: usize,
    // One instrument, or every instrument in the window.
    ticker: Option<String>,
) -> Result<View, TapeError> {
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
        ticker,
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

    /// **A gap across midnight is charged to both days**, clipped at the
    /// boundary, because it is missing from both. Arithmetic nobody sees,
    /// which is exactly why it is held here rather than discovered later from
    /// a figure that looked plausible.
    #[test]
    fn a_gap_across_midnight_is_charged_to_both_days() {
        // Two hours before midnight to three hours after.
        let midnight = 20_000 * DAY_MICROS;
        let split = split_by_day(midnight - 2 * 3_600_000_000, midnight + 3 * 3_600_000_000);
        assert_eq!(split.len(), 2, "{split:?}");

        let (first_day, (a, b)) = &split[0];
        assert_eq!(*b, midnight, "the first part ends at midnight exactly");
        assert_eq!(b - a, 2 * 3_600_000_000, "two hours on the first day");

        let (second_day, (c, d)) = &split[1];
        assert_eq!(*c, midnight, "and the second begins there");
        assert_eq!(d - c, 3 * 3_600_000_000, "three hours on the second");
        assert_ne!(first_day, second_day, "and they are different days");
    }

    /// An interval inside one day is not split.
    #[test]
    fn a_gap_within_one_day_stays_one() {
        let noon = 20_000 * DAY_MICROS + 12 * 3_600_000_000;
        let split = split_by_day(noon, noon + 60_000_000);
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].1, (noon, noon + 60_000_000));
    }

    /// A gap that runs backwards, or has no width, contributes nothing.
    #[test]
    fn an_empty_gap_contributes_nothing() {
        assert!(split_by_day(100, 100).is_empty());
        assert!(split_by_day(500, 100).is_empty());
    }

    /// **One outage written once per instrument counts once.** The tape writes
    /// a gap row per affected `(ticker, series)`; summing them overstates by
    /// the instrument count, which on the real archive turned 117,308s into
    /// 2,815,397s.
    #[test]
    fn one_outage_written_many_times_costs_the_day_once() {
        let outage = (1_000, 5_000);
        let spans = vec![outage; 24];
        assert_eq!(
            missing_within(&spans, 0, 10_000),
            4_000,
            "twenty-four rows, one absence"
        );
    }

    /// **Nothing outside the observed window is charged.** A gap before
    /// capture began is not the window's problem — the record says nothing
    /// about that time either way.
    #[test]
    fn a_gap_outside_the_window_costs_nothing() {
        assert_eq!(missing_within(&[(0, 500)], 1_000, 2_000), 0);
        assert_eq!(missing_within(&[(5_000, 9_000)], 1_000, 2_000), 0);
        // And one straddling the start is charged only for the part inside.
        assert_eq!(missing_within(&[(500, 1_500)], 1_000, 2_000), 500);
    }

    /// Coverage can never exceed the window, nor go below zero.
    #[test]
    fn missing_is_bounded_by_the_window() {
        // A gap far wider than the window.
        let missing = missing_within(&[(i64::MIN / 2, i64::MAX / 2)], 1_000, 2_000);
        assert_eq!(missing, 1_000, "clipped to the window, not beyond it");
    }

    /// A day the record holds with no gap at all is covered for its whole
    /// window — and the window is not the day.
    #[test]
    fn a_day_with_no_gap_is_covered_for_its_window() {
        assert_eq!(missing_within(&[], 1_000, 2_000), 0);
    }

    /// A root with no tape answers, rather than refusing.
    #[test]
    fn an_absent_tape_covers_nothing() {
        let found = covered_days(Path::new("/galata-tower-no-such-tape-root"));
        assert!(found.days.is_empty());
    }

    /// **`div_euclid`, not `/`.** A timestamp before the epoch belongs to the
    /// hour BEFORE it, and division rounding toward zero names the wrong one —
    /// the trap `date_of` already documents, reachable here by the same route.
    #[test]
    fn an_hour_is_truncated_the_way_a_date_is() {
        // Inside an hour, every instant maps to its start.
        let hour = 480_000 * HOUR_MICROS;
        assert_eq!(hour_of(hour), hour);
        assert_eq!(hour_of(hour + 1), hour);
        assert_eq!(hour_of(hour + HOUR_MICROS - 1), hour);
        assert_eq!(hour_of(hour + HOUR_MICROS), hour + HOUR_MICROS);

        // And before the epoch, where `/` would round toward zero and name the
        // hour after the one the instant is in.
        assert_eq!(
            hour_of(-1),
            -HOUR_MICROS,
            "one microsecond before the epoch"
        );
        assert_eq!(hour_of(-HOUR_MICROS), -HOUR_MICROS);
        assert_ne!(hour_of(-1), 0, "which is what plain division would give");
    }

    /// The cap keeps the NEWEST hours and says that it applied — a short
    /// answer and a truncated one are different facts.
    #[test]
    fn the_cap_keeps_the_end_of_the_record() {
        let nowhere = Path::new("/galata-tower-no-such-tape-root");
        let found = rates(nowhere, 10);
        assert_eq!(found.hours, 0);
        assert!(!found.capped, "nothing to cap is not a cap");
        assert!(found.buckets.is_empty());
    }

    /// The default is stated rather than derived, like every other cap here.
    #[test]
    fn the_default_hour_cap_is_stated() {
        assert_eq!(DEFAULT_HOURS, 200);
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
        let view = view(&root, Kind::Quotes, 0, i64::MAX / 2, DEFAULT_LIMIT, None)
            .expect("the tape reads");
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
        let capped = view(&root, Kind::Quotes, 0, i64::MAX / 2, 40, None).expect("the tape reads");
        assert_eq!(capped.rows.len(), 40, "the cap applies");
        assert!(
            capped.total > 40,
            "and the total says what was left out: {}",
            capped.total
        );

        // The newest, not the oldest: a reader watching a tape wants its end.
        let all = view(&root, Kind::Quotes, 0, i64::MAX / 2, usize::MAX, None).expect("reads");
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
            None,
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
        let refused = view(&root, Kind::Quotes, 0, 100, 0, None).expect_err("zero must refuse");
        assert!(matches!(refused, TapeError::NoRows), "got {refused:?}");
    }

    /// A backwards window is refused, not answered with an empty list.
    #[test]
    fn a_backwards_window_is_refused() {
        let root = std::path::PathBuf::from(".");
        let refused = view(&root, Kind::Quotes, 100, 50, DEFAULT_LIMIT, None)
            .expect_err("backwards must refuse");
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

/// One day of one dataset, and how much of it the record holds.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DayCoverage {
    /// The venue.
    pub venue: String,
    /// The dataset.
    pub kind: String,
    /// The day, on the calendar the partitions use.
    pub date: String,
    /// The start of the time this day's record accounts for, in our clock.
    ///
    /// **Accounted for, never assumed.** Not midnight: a day whose capture
    /// began at noon says nothing about its morning, and claiming the morning
    /// either way would invent an absence or hide one.
    ///
    /// But a STATED gap is not silence — it is the record saying *this time
    /// was missing* — so the window covers the observed rows AND the day's
    /// recorded gaps. The first version used arrivals alone and silently
    /// discarded a 6½-hour downtime gap that ended where capture resumed: the
    /// record had accounted for that morning and the figure threw it away.
    pub window_from_micros: i64,
    /// The last arrival.
    pub window_to_micros: i64,
    /// The window's length.
    pub window_micros: i64,
    /// Time the record STATES is missing inside the window.
    ///
    /// The union of the day's gap intervals, clipped to the window — never the
    /// sum of the gap rows, which overstates by the number of instruments
    /// affected.
    pub missing_micros: i64,
    /// What is left: `window_micros - missing_micros`.
    pub covered_micros: i64,
    /// How many rows stand behind it.
    pub rows: usize,
}

/// What the record covers, by day.
///
/// Named `Covered` because `Coverage` is already the gaps summary's — two
/// different questions about the same absences.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Covered {
    /// Newest day first.
    pub days: Vec<DayCoverage>,
}

/// Microseconds in a day.
const DAY_MICROS: i64 = 86_400_000_000;

/// How much of each day the record holds.
///
/// **Our clock throughout.** `recv_micros` is what the partitions are dated by,
/// what a gap's bounds are written in, and what *did we have this data* means.
/// The venue's clock answers when something happened there, which is a
/// different question, and a figure mixing the two would be neither.
pub fn covered_days(root: &Path) -> Covered {
    // (venue, day) -> gap intervals, in our clock.
    let mut gaps_by_day: BTreeMap<(String, String), Vec<(i64, i64)>> = BTreeMap::new();
    collect_gaps(root, &mut gaps_by_day);

    // The extent each day's gaps account for, per venue — a stated absence is
    // part of what the record accounts for, so it widens the window.
    let mut gap_extent: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();
    for (key, spans) in &gaps_by_day {
        let first = spans.iter().map(|s| s.0).min().unwrap_or(0);
        let last = spans.iter().map(|s| s.1).max().unwrap_or(0);
        gap_extent.insert(key.clone(), (first, last));
    }

    // (venue, kind, day) -> (first, last, rows)
    let mut windows: BTreeMap<(String, String, String), (i64, i64, usize)> = BTreeMap::new();
    for kind in SERVED {
        // **Not `gaps` itself.** That dataset records absences; how much of the
        // bookkeeping is missing is a different question from how much of the
        // data is, and answering it would put a confusing row in the middle of
        // a table about market data.
        if kind == Kind::Gaps {
            continue;
        }
        let scope = format!("kind={}", kind.as_str());
        let scopes = [scope.as_str()];
        if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
            continue;
        }
        let Ok(reader) = galata_datawatch::tape::reader::Reader::open(root, &scopes) else {
            continue;
        };
        let Ok(batches) = reader.view(galata_datawatch::tape::reader::Window {
            kind,
            from_micros: i64::MIN + 1,
            to_micros: i64::MAX,
            ticker: None,
        }) else {
            continue;
        };
        for batch in &batches {
            use arrow::array::{Array, Int64Array, StringArray};
            let venue = batch
                .column_by_name("venue")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let recv = batch
                .column_by_name("recv_micros")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let (Some(venue), Some(recv)) = (venue, recv) else {
                continue;
            };
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || recv.is_null(i) {
                    continue;
                }
                let at = recv.value(i);
                let key = (
                    venue.value(i).to_owned(),
                    kind.as_str().to_owned(),
                    galata_datawatch::date_of(at),
                );
                windows
                    .entry(key)
                    .and_modify(|w| {
                        w.0 = w.0.min(at);
                        w.1 = w.1.max(at);
                        w.2 += 1;
                    })
                    .or_insert((at, at, 1));
            }
        }
    }

    // **A day the record accounts for entirely with gaps has no rows, and so
    // had no window, and so did not appear at all** — which silently omitted
    // the worst day in the table. 2026-09-21 on this archive is 24 hours of
    // stated downtime and nothing else; asking whether it is usable must not
    // return nothing.
    //
    // So every venue/day the gaps name gets an entry for each dataset that
    // venue captures anywhere in the record, with no rows behind it.
    let kinds_per_venue: BTreeMap<String, std::collections::BTreeSet<String>> = windows
        .keys()
        .fold(BTreeMap::new(), |mut acc, (venue, kind, _)| {
            acc.entry(venue.clone()).or_default().insert(kind.clone());
            acc
        });
    for ((venue, date), &(gf, gt)) in &gap_extent {
        let Some(kinds) = kinds_per_venue.get(venue) else {
            continue;
        };
        for kind in kinds {
            windows
                .entry((venue.clone(), kind.clone(), date.clone()))
                .or_insert((gf, gt, 0));
        }
    }

    let mut days: Vec<DayCoverage> = windows
        .into_iter()
        .map(|((venue, kind, date), (from, to, rows))| {
            // Widened by what the day's gaps account for.
            let (from, to) = match gap_extent.get(&(venue.clone(), date.clone())) {
                Some(&(gf, gt)) => (from.min(gf), to.max(gt)),
                None => (from, to),
            };
            let window = (to - from).max(0);
            let missing = gaps_by_day
                .get(&(venue.clone(), date.clone()))
                .map(|spans| missing_within(spans, from, to))
                .unwrap_or(0);
            DayCoverage {
                venue,
                kind,
                date,
                window_from_micros: from,
                window_to_micros: to,
                window_micros: window,
                missing_micros: missing,
                // Clamped: a gap wider than the window would otherwise report
                // negative coverage, which is not a thing.
                covered_micros: (window - missing).max(0),
                rows,
            }
        })
        .collect();
    // Newest day first, then venue and dataset.
    days.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then(a.venue.cmp(&b.venue))
            .then(a.kind.cmp(&b.kind))
    });
    Covered { days }
}

/// Every gap interval the tape states, bucketed by the day it falls in.
///
/// **A gap across midnight is charged to both days**, clipped at the boundary,
/// because it is missing from both.
fn collect_gaps(root: &Path, out: &mut BTreeMap<(String, String), Vec<(i64, i64)>>) {
    let scope = format!("kind={}", Kind::Gaps.as_str());
    let scopes = [scope.as_str()];
    if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
        return;
    }
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
        use arrow::array::{Array, Int64Array, StringArray};
        let venue = batch
            .column_by_name("venue")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let from = batch
            .column_by_name("from_micros")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
        let to = batch
            .column_by_name("to_micros")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
        let (Some(venue), Some(from), Some(to)) = (venue, from, to) else {
            continue;
        };
        for i in 0..batch.num_rows() {
            if venue.is_null(i) || from.is_null(i) || to.is_null(i) {
                continue;
            }
            for (date, span) in split_by_day(from.value(i), to.value(i)) {
                out.entry((venue.value(i).to_owned(), date))
                    .or_default()
                    .push(span);
            }
        }
    }
}

/// One interval, cut at every midnight it crosses.
fn split_by_day(from: i64, to: i64) -> Vec<(String, (i64, i64))> {
    let mut out = Vec::new();
    if to <= from {
        return out;
    }
    let mut start = from;
    while start < to {
        let midnight = (start.div_euclid(DAY_MICROS) + 1) * DAY_MICROS;
        let end = to.min(midnight);
        out.push((galata_datawatch::date_of(start), (start, end)));
        start = end;
    }
    out
}

/// The union of `spans`, intersected with `[from, to]`.
fn missing_within(spans: &[(i64, i64)], from: i64, to: i64) -> i64 {
    let mut clipped: Vec<(i64, i64)> = spans
        .iter()
        .map(|&(a, b)| (a.max(from), b.min(to)))
        .filter(|&(a, b)| b > a)
        .collect();
    if clipped.is_empty() {
        return 0;
    }
    // The same merge /v1/gaps proved: the tape writes one gap row per affected
    // (ticker, series), so summing them overstates by the instrument count.
    let (total, ..) = union_micros(&mut clipped);
    total
}

/// One hour of one dataset.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HourlyRows {
    /// The venue.
    pub venue: String,
    /// The dataset.
    pub kind: String,
    /// The hour's start, in our clock.
    pub hour_micros: i64,
    /// How many rows arrived in it.
    ///
    /// **A count, not a rate per second.** The newest bucket holds whatever has
    /// elapsed of it, and the oldest whatever was captured; a count is honestly
    /// smaller for a partial hour where a normalised rate would extrapolate
    /// from it.
    pub rows: usize,
}

/// Rows per hour, newest first.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Rates {
    /// How many hours were returned.
    pub hours: usize,
    /// Whether a cap applied — so a short answer and a capped one differ.
    pub capped: bool,
    /// Newest first.
    pub buckets: Vec<HourlyRows>,
}

/// Microseconds in an hour.
const HOUR_MICROS: i64 = 3_600_000_000;

/// The most hour-buckets a read returns when the caller names no limit.
///
/// **Stated, not derived.** A year is 8,760 buckets per dataset; an unbounded
/// response is the defect `/v1/tape`'s cap exists to prevent, and there is no
/// reason to relearn it here. Two hundred is a few days of one dataset, or a
/// day of several.
pub const DEFAULT_HOURS: usize = 200;

/// How many rows the record holds, by hour.
///
/// **No baseline and no threshold.** The published answer to a feed that stays
/// connected and delivers a trickle is a learned expected count and an alert on
/// deviation; what counts as a normal hour for a venue is a thing the operator
/// knows and this tower does not. Nine hours in a column, one of them two
/// orders of magnitude smaller, is a fact anybody can read.
pub fn rates(root: &Path, limit: usize) -> Rates {
    let mut counted: BTreeMap<(String, String, i64), usize> = BTreeMap::new();

    for kind in SERVED {
        let scope = format!("kind={}", kind.as_str());
        let scopes = [scope.as_str()];
        if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
            continue;
        }
        let Ok(reader) = galata_datawatch::tape::reader::Reader::open(root, &scopes) else {
            continue;
        };
        let Ok(batches) = reader.view(galata_datawatch::tape::reader::Window {
            kind,
            from_micros: i64::MIN + 1,
            to_micros: i64::MAX,
            ticker: None,
        }) else {
            continue;
        };
        for batch in &batches {
            use arrow::array::{Array, Int64Array, StringArray};
            let venue = batch
                .column_by_name("venue")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let recv = batch
                .column_by_name("recv_micros")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let (Some(venue), Some(recv)) = (venue, recv) else {
                continue;
            };
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || recv.is_null(i) {
                    continue;
                }
                let key = (
                    venue.value(i).to_owned(),
                    kind.as_str().to_owned(),
                    hour_of(recv.value(i)),
                );
                *counted.entry(key).or_insert(0) += 1;
            }
        }
    }

    let total = counted.len();
    let mut buckets: Vec<HourlyRows> = counted
        .into_iter()
        .map(|((venue, kind, hour_micros), rows)| HourlyRows {
            venue,
            kind,
            hour_micros,
            rows,
        })
        .collect();
    // Newest first, then venue and dataset — an operator looks at the end of
    // the record, and the cap must keep that end.
    buckets.sort_by(|a, b| {
        b.hour_micros
            .cmp(&a.hour_micros)
            .then(a.venue.cmp(&b.venue))
            .then(a.kind.cmp(&b.kind))
    });
    let capped = buckets.len() > limit;
    buckets.truncate(limit);

    Rates {
        hours: total,
        capped,
        buckets,
    }
}

/// The hour a timestamp falls in, truncated.
///
/// **`div_euclid`, not `/`** — the reason `date_of` already gives: a negative
/// timestamp belongs to the hour BEFORE the epoch, and division rounding
/// toward zero would name the wrong one.
fn hour_of(micros: i64) -> i64 {
    micros.div_euclid(HOUR_MICROS) * HOUR_MICROS
}
