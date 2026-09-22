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
const SERVED: [Kind; 5] = [
    Kind::Quotes,
    Kind::Trades,
    Kind::Candles,
    Kind::Funding,
    Kind::Marks,
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

#[cfg(test)]
mod tests {
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
