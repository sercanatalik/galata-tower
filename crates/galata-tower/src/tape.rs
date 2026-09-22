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
    /// The rows, each a map of column to value. Decimals are strings.
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
pub fn view(root: &Path, kind: Kind, from: i64, to: i64) -> Result<View, TapeError> {
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
    Ok(View {
        kind: kind.as_str().to_owned(),
        bound: reader.bound().position,
        rows: rows(&batches)?,
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
        let view = view(&root, Kind::Quotes, 0, i64::MAX / 2).expect("the tape reads");
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

    /// A backwards window is refused, not answered with an empty list.
    #[test]
    fn a_backwards_window_is_refused() {
        let root = std::path::PathBuf::from(".");
        let refused = view(&root, Kind::Quotes, 100, 50).expect_err("backwards must refuse");
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
