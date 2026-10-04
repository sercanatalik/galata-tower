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

mod coverage;
mod gaps;
mod view;

/// Read a typed column from a record batch, or refuse loudly.
///
/// **Loudly, rather than wrongly.** A type rendered by guesswork is how a
/// figure comes to disagree with the venue's. This macro removes the
/// `column_by_name` + `downcast_ref` + `ok_or_else` boilerplate that every
/// column read repeats.
///
/// # Example
///
/// ```ignore
/// let venue = column!(batch, "venue", StringArray, "Utf8")?;
/// let recv = column!(batch, "recv_micros", Int64Array, "Int64")?;
/// ```
#[macro_export]
macro_rules! column {
    ($batch:expr, $name:expr, $ty:ty, $want:literal) => {
        $batch
            .column_by_name($name)
            .and_then(|c| c.as_any().downcast_ref::<$ty>())
            .ok_or_else(|| $crate::tape::TapeError::Unrenderable {
                column: $name.to_owned(),
                data_type: format!("expected {}", $want),
            })
    };
}

// Re-export the public types from the sub-modules.
#[allow(unused_imports)]
pub use coverage::{Covered, DEFAULT_HOURS, DayCoverage, HourlyRows, Rates, covered_days, rates};
pub(crate) use coverage::{DayWindows, covered_from, day_window};
pub(crate) use gaps::gaps_by_day;
#[allow(unused_imports)]
pub use gaps::{Cause, Coverage, coverage, union_micros};
#[allow(unused_imports)]
pub use view::{DEFAULT_LIMIT, View, rows, view};

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, Int64Array, RecordBatch, StringArray};
use galata_wire::Kind;
use serde::Serialize;
use utoipa::ToSchema;

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
pub(crate) const SERVED: [Kind; 6] = [
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

/// Each kind's durable bound, **per venue**: kind → venue → stream sequence.
///
/// Per venue because each venue numbers its stream from its own capture
/// process's boot, so one position cannot bound two venues — galata-datawatch's
/// `sequence-is-per-venue` reproduced a durable row hidden behind another
/// venue's numbering. A max or a min over venues would be a number the screen
/// shows and nobody can act on, so the map is reported whole.
/// The whole of time, for the reads that genuinely fold every row.
pub(crate) const ALL_TIME: (i64, i64) = (i64::MIN + 1, i64::MAX);

/// One tape dataset's rows inside a window, or none when it is unwritten.
///
/// The window reaches the reader, which prunes date partitions and row
/// groups by it — a read of a day costs a day, not the history. `ALL_TIME`
/// is for callers whose answer really is over every row.
pub(crate) fn read_kind(
    root: &Path,
    kind: Kind,
    ticker: Option<String>,
    (from_micros, to_micros): (i64, i64),
) -> Result<Vec<RecordBatch>, TapeError> {
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];
    let unreadable = |error: &dyn std::fmt::Display| TapeError::Unreadable {
        detail: error.to_string(),
    };
    // The reader refuses an unwritten scope with its own name for it, so the
    // refusal IS the "has anything written?" answer — asking `unwritten`
    // first only walked the whole kind a second time (28ms a second over
    // 2,230 candle partitions, measured 2026-09-26; see `tape::bounds`).
    let reader = match galata_datawatch::tape::reader::Reader::open(root, &scopes) {
        Ok(reader) => reader,
        Err(galata_datawatch::tape::reader::ReadError::NoFrontier { .. }) => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(unreadable(&e)),
    };
    reader
        .view(galata_datawatch::tape::reader::Window {
            kind,
            from_micros,
            to_micros,
            ticker,
        })
        .map_err(|e| unreadable(&e))
}

pub type Bounds = BTreeMap<String, BTreeMap<String, i64>>;

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
pub fn bounds(root: &Path, labels: &galata_datawatch::tape::LabelCache) -> Bounds {
    let mut found = BTreeMap::new();
    for kind in SERVED {
        let scope = format!("kind={}", kind.as_str());
        let scopes = [scope.as_str()];
        // A kind that has written nothing, a root that is not there, or a
        // store that will not open is silence here: `open_cached` refuses an
        // unwritten scope itself, so asking `unwritten` first only walked the
        // whole kind a second time (28 ms a second over 2,230 candle
        // partitions, measured 2026-09-26). The caller decides whether
        // silence is worth a log line; doing it here would do it once a
        // second.
        // Through the watch's cache: footers and directory listings are read
        // once, and this runs every second over a tape that only ever grows.
        if let Ok(reader) =
            galata_datawatch::tape::reader::Reader::open_cached(root, &scopes, labels)
        {
            found.insert(kind.as_str().to_owned(), reader.bound().positions.clone());
        }
    }
    found
}

/// One instrument, as the record holds it.
#[derive(Debug, Serialize, ToSchema)]
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
#[derive(Debug, Serialize, ToSchema)]
pub struct Instruments {
    /// Each kind's durable bound per venue — what bounded the rows each kind
    /// was read at.
    #[schema(inline)]
    pub bounds: Bounds,
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
    let mut bounds = Bounds::new();

    for kind in SERVED {
        let scope = format!("kind={}", kind.as_str());
        let scopes = [scope.as_str()];
        // A kind that has written nothing is silence, not a refusal: the
        // caller asked what the record holds, and *not this* is an answer.
        // `open` refuses an unwritten scope itself, so asking `unwritten`
        // first only walked the whole kind a second time — see `bounds`.
        let Ok(reader) = galata_datawatch::tape::reader::Reader::open(root, &scopes) else {
            continue;
        };
        bounds.insert(kind.as_str().to_owned(), reader.bound().positions.clone());
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
            // Folded on borrowed keys first: probing the global map built two
            // owned strings per row, the dominant share of the 170ms fold,
            // for keys a batch repeats tens of thousands of times.
            let mut local: BTreeMap<(&str, &str), (i64, usize)> = BTreeMap::new();
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || ticker.is_null(i) {
                    continue;
                }
                // A row with no venue time still counts as a row: it arrived.
                // It just cannot make the instrument look newer than it is.
                let when = at.filter(|a| !a.is_null(i)).map(|a| a.value(i));
                let entry = local
                    .entry((venue.value(i), ticker.value(i)))
                    .or_insert((i64::MIN, 0));
                entry.1 += 1;
                if let Some(when) = when {
                    entry.0 = entry.0.max(when);
                }
            }
            for ((venue, ticker), (newest, rows)) in local {
                let entry = seen
                    .entry((venue.to_owned(), ticker.to_owned(), kind.as_str()))
                    .or_insert((i64::MIN, 0));
                entry.0 = entry.0.max(newest);
                entry.1 += rows;
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
    Instruments {
        bounds,
        instruments,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tape_root() -> Option<std::path::PathBuf> {
        let root = std::path::PathBuf::from("../../../galata-datawatch/var/tape");
        root.is_dir().then_some(root)
    }

    /// The watch's cache pays for a footer once. Against the real tape, when
    /// there is one: a second pass over an unchanged tape reads none.
    #[test]
    fn a_quiet_tape_is_watched_without_rereading_footers() {
        let Some(root) = tape_root() else {
            eprintln!("SKIPPED: no tape");
            return;
        };
        let labels = galata_datawatch::tape::LabelCache::default();
        let first = bounds(&root, &labels);
        let read = labels.footer_reads();
        assert!(read > 0, "the first pass read no label");
        let second = bounds(&root, &labels);
        assert_eq!(
            labels.footer_reads(),
            read,
            "an unchanged tape was read again"
        );
        assert_eq!(first, second);
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

    /// An unknown dataset names what there is.
    #[test]
    fn an_unknown_kind_says_what_the_tape_holds() {
        let refused = kind_of("nonsense").expect_err("unknown must refuse");
        let said = refused.to_string();
        assert!(said.contains("nonsense"), "{said}");
        assert!(said.contains("quotes"), "and what there is: {said}");
    }
}
