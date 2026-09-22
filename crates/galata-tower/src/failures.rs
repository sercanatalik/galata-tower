//! The record's parse failures, read back.
//!
//! **The archive writes these and nothing read them.** When a payload arrives
//! and will not normalise, the record keeps the bytes in the main segment and
//! a row in a `failures/` sibling beside them, because *"filtering a record by
//! parse success discards exactly the evidence a normalisation defect is
//! diagnosed from."* Searching both workspaces for a reader of that directory
//! found two tests and one line in `replay.rs` that skips it.
//!
//! **A failure is not a gap.** A gap is a known absence with a cause and
//! bounds, and `/v1/gaps` reports it. A failure is a payload that ARRIVED and
//! produced no row: the record looks complete, the rows are simply not there,
//! and nothing says so.

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, Int64Array, RecordBatch, StringArray, UInt64Array};
use serde::Serialize;

/// One kind of failure, and how much of it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Failing {
    /// The venue whose payload it was.
    pub venue: String,
    /// The partition it landed under.
    ///
    /// **Not the same as `channel`**, and both are reported for that reason: a
    /// `bbo` channel lands under a `quotes` kind, and a live run produced
    /// `kind=bbo/failures/` beside `kind=quotes/` — a failure row in a
    /// partition its payload is not in. Reporting one without the other
    /// reintroduces the confusion the sequence exists to resolve.
    pub kind: String,
    /// The channel the payload arrived on.
    pub channel: String,
    /// What went wrong, in the decoder's own words.
    pub error: String,
    /// How many payloads failed this way.
    pub failures: usize,
    /// When the first and last of them arrived, by our clock.
    pub first_micros: i64,
    /// The last arrival.
    pub last_micros: i64,
    /// The payload sequences, at both ends.
    ///
    /// **The join, offered rather than performed.** The bytes are in the main
    /// segment under the same sequence; the tower says where to look instead
    /// of serving venue payloads over HTTP.
    pub first_seq: u64,
    /// The last sequence.
    pub last_seq: u64,
}

/// What the record says failed to normalise.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Failures {
    /// How many partitions were examined.
    ///
    /// **So that *none found* and *nothing looked at* are different answers.**
    /// An empty list against zero partitions is a wrong archive root; against
    /// thirteen it is a clean record.
    pub partitions: usize,
    /// How many carried a `failures/` directory at all.
    pub with_failures: usize,
    /// One entry per distinct failure, most frequent first.
    pub failing: Vec<Failing>,
}

/// Every parse failure the archive holds.
///
/// Walks `venue=*/kind=*/date=*/failures/`. A partition without that directory
/// is the ordinary case and contributes nothing — a clean record opens no
/// parquet at all.
pub fn failures(root: &Path) -> Failures {
    // (venue, kind, channel, error) -> (count, first/last micros, first/last seq)
    let mut folded: BTreeMap<(String, String, String, String), Tally> = BTreeMap::new();
    let mut partitions = 0;
    let mut with_failures = 0;

    for (kind, dir) in partition_dirs(root) {
        partitions += 1;
        let failures_dir = dir.join("failures");
        if !failures_dir.is_dir() {
            continue;
        }
        with_failures += 1;
        for (_, path) in galata_segments::list_segments(&failures_dir) {
            let Ok(batches) = galata_segments::read_segment(&path) else {
                continue;
            };
            for batch in &batches {
                fold(&batch_columns(batch), batch, &kind, &mut folded);
            }
        }
    }

    let mut failing: Vec<Failing> = folded
        .into_iter()
        .map(|((venue, kind, channel, error), t)| Failing {
            venue,
            kind,
            channel,
            error,
            failures: t.count,
            first_micros: t.first_micros,
            last_micros: t.last_micros,
            first_seq: t.first_seq,
            last_seq: t.last_seq,
        })
        .collect();
    // Most frequent first: an operator wants the thing that is failing most.
    failing.sort_by_key(|f| std::cmp::Reverse(f.failures));

    Failures {
        partitions,
        with_failures,
        failing,
    }
}

/// What one group is accumulating.
struct Tally {
    count: usize,
    first_micros: i64,
    last_micros: i64,
    first_seq: u64,
    last_seq: u64,
}

/// The four columns this reads, where the batch has them.
struct Columns<'a> {
    seq: Option<&'a UInt64Array>,
    recv: Option<&'a Int64Array>,
    venue: Option<&'a StringArray>,
    channel: Option<&'a StringArray>,
    error: Option<&'a StringArray>,
}

/// Typed, never through `ArrayFormatter` — the same measured choice
/// `/v1/gaps` makes, for the same reason: formatting is the dominant cost of a
/// read and nothing here needs a rendered string.
fn batch_columns(batch: &RecordBatch) -> Columns<'_> {
    fn text<'b>(batch: &'b RecordBatch, name: &str) -> Option<&'b StringArray> {
        batch
            .column_by_name(name)?
            .as_any()
            .downcast_ref::<StringArray>()
    }
    Columns {
        seq: batch
            .column_by_name("seq")
            .and_then(|c| c.as_any().downcast_ref::<UInt64Array>()),
        recv: batch
            .column_by_name("recv_micros")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>()),
        venue: text(batch, "venue"),
        channel: text(batch, "channel"),
        error: text(batch, "error"),
    }
}

/// One batch into the tally.
fn fold(
    columns: &Columns<'_>,
    batch: &RecordBatch,
    kind: &str,
    folded: &mut BTreeMap<(String, String, String, String), Tally>,
) {
    let (Some(venue), Some(channel), Some(error)) = (columns.venue, columns.channel, columns.error)
    else {
        // Not a failures batch. Skipped rather than guessed at: the caller
        // asked what failed, and a batch that cannot say is not an answer.
        return;
    };
    for i in 0..batch.num_rows() {
        if venue.is_null(i) || channel.is_null(i) || error.is_null(i) {
            continue;
        }
        let seq = columns
            .seq
            .filter(|c| !c.is_null(i))
            .map_or(0, |c| c.value(i));
        let recv = columns
            .recv
            .filter(|c| !c.is_null(i))
            .map_or(0, |c| c.value(i));
        let key = (
            venue.value(i).to_owned(),
            kind.to_owned(),
            channel.value(i).to_owned(),
            error.value(i).to_owned(),
        );
        folded
            .entry(key)
            .and_modify(|t| {
                t.count += 1;
                t.first_micros = t.first_micros.min(recv);
                t.last_micros = t.last_micros.max(recv);
                t.first_seq = t.first_seq.min(seq);
                t.last_seq = t.last_seq.max(seq);
            })
            .or_insert(Tally {
                count: 1,
                first_micros: recv,
                last_micros: recv,
                first_seq: seq,
                last_seq: seq,
            });
    }
}

/// Every `venue=*/kind=*/date=*` directory, with the kind it names.
///
/// Walked rather than globbed so a level that is not there stops the descent
/// quietly: an archive root with nothing under it yields nothing, which is an
/// answer.
fn partition_dirs(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut found = Vec::new();
    for venue in children(root) {
        for kind_dir in children(&venue) {
            let Some(kind) = kind_dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("kind="))
            else {
                continue;
            };
            for date in children(&kind_dir) {
                found.push((kind.to_owned(), date));
            }
        }
    }
    found
}

/// A directory's subdirectories, sorted, or none.
fn children(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A scratch archive root that removes itself.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "galata-tower-failures-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch root");
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Write failure rows the way the archive writes them.
    ///
    /// **The real schema and the real segment writer.** `Archive::fail` is
    /// `pub(crate)` on purpose — `check-ingest-callers.sh` holds the one path
    /// — so this builds the six documented columns and writes them through
    /// `galata-segments`, rather than routing around a guard to get test data.
    /// `(seq, recv_micros, venue, channel, error)` — in the column order the
    /// archive writes, so a swapped pair in a fixture is visible here rather
    /// than encoded into an assertion. An earlier draft had exactly that.
    fn write_failures(root: &Path, kind: &str, rows: &[(u64, i64, &str, &str, &str)]) {
        use arrow::array::{Int64Array, StringBuilder, UInt16Array, UInt64Array};
        use arrow::datatypes::{DataType, Field, Schema};

        let schema = Arc::new(Schema::new(vec![
            Field::new("seq", DataType::UInt64, false),
            Field::new("recv_micros", DataType::Int64, false),
            Field::new("venue", DataType::Utf8, false),
            Field::new("channel", DataType::Utf8, false),
            Field::new("error", DataType::Utf8, false),
            Field::new("schema_version", DataType::UInt16, false),
        ]));
        let mut venue = StringBuilder::new();
        let mut channel = StringBuilder::new();
        let mut error = StringBuilder::new();
        for (_, _, v, c, e) in rows {
            venue.append_value(v);
            channel.append_value(c);
            error.append_value(e);
        }
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.0))),
                Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.1))),
                Arc::new(venue.finish()),
                Arc::new(channel.finish()),
                Arc::new(error.finish()),
                Arc::new(UInt16Array::from_iter_values(rows.iter().map(|_| 1u16))),
            ],
        )
        .expect("the failure schema");

        let dir = root
            .join("venue=hyperliquid")
            .join(format!("kind={kind}"))
            .join("date=2026-09-22")
            .join("failures");
        let first = rows.iter().map(|r| r.1).min().unwrap_or(0);
        let last = rows.iter().map(|r| r.1).max().unwrap_or(0);
        galata_segments::write_segment(
            &dir,
            galata_segments::Cursor::Time {
                first_micros: first,
                last_micros: last,
                pid: std::process::id(),
                seq: 1,
            },
            &batch,
            galata_segments::Codec::Zstd,
        )
        .expect("the segment writes");
    }

    /// **The surface, against a failure that had to be manufactured.** The
    /// real archive holds none — which is the argument for building this
    /// before the first parse defect, and the reason this test writes one.
    #[test]
    fn what_failed_is_read_back_and_folded() {
        let scratch = Scratch::new("folded");
        write_failures(
            &scratch.0,
            "quotes",
            &[
                // A `bbo` channel under a `quotes` kind — the real shape the
                // Failure type carries both fields for.
                (9000, 1_000, "hyperliquid", "bbo", "unknown field `px2`"),
                (9001, 2_000, "hyperliquid", "bbo", "unknown field `px2`"),
                (9002, 3_000, "hyperliquid", "bbo", "invalid type: string"),
            ],
        );

        let found = failures(&scratch.0);
        assert_eq!(found.partitions, 1, "one date partition was walked");
        assert_eq!(found.with_failures, 1);
        assert_eq!(
            found.failing.len(),
            2,
            "two distinct errors: {:?}",
            found.failing
        );

        // Most frequent first.
        let top = &found.failing[0];
        assert_eq!(top.failures, 2);
        assert_eq!(top.error, "unknown field `px2`");
        assert_eq!((top.first_micros, top.last_micros), (1_000, 2_000));
        assert_eq!((top.first_seq, top.last_seq), (9000, 9001));
        // The partition's kind, not the row's channel — they are not the same
        // thing, and both are reported.
        assert_eq!(top.venue, "hyperliquid");
        // **The two that are not the same thing**: the payload arrived on a
        // `bbo` channel and was partitioned under a `quotes` kind, which is
        // the live case the Failure type carries both fields for.
        assert_eq!(top.kind, "quotes", "from the partition path");
        assert_eq!(top.channel, "bbo", "from the row");
    }

    /// **A clean record is not an empty answer.** Zero failures against
    /// thirteen partitions is a different statement from zero against zero,
    /// which is a wrong archive root.
    #[test]
    fn a_clean_record_says_how_much_it_looked_at() {
        let scratch = Scratch::new("clean");
        // A partition with payloads and no failures sibling.
        std::fs::create_dir_all(
            scratch
                .0
                .join("venue=hyperliquid")
                .join("kind=quotes")
                .join("date=2026-09-22"),
        )
        .expect("a partition");

        let found = failures(&scratch.0);
        assert_eq!(found.partitions, 1, "it looked at one");
        assert_eq!(found.with_failures, 0, "and none had failed");
        assert!(found.failing.is_empty());
    }

    /// A root that is not there yields nothing, and is not an error.
    #[test]
    fn an_absent_archive_is_silence() {
        let found = failures(Path::new("/galata-tower-no-such-archive"));
        assert_eq!(found.partitions, 0);
        assert!(found.failing.is_empty());
    }
}
