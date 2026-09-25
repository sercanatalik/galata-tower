//! Reads shaped for what the screen draws: frontiers, the timeline, candles and
//! the board.
//!
//! Every figure here is folded from rows or read from segment names. Footer
//! statistics are never reported, for the reason `tape::Instrument::last_micros`
//! gives.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use arrow::array::{Array, BooleanArray, Decimal128Array, Int64Array, RecordBatch, StringArray};
use arrow::datatypes::DataType;
use galata_wire::Kind;
use serde::Serialize;

use crate::tape::{self, TapeError};

/// Two receipts further apart than this are two runs, not one.
pub const RUN_BREAK_MICROS: i64 = 60_000_000;

/// A bar received this long after its own time was sent to us late, by a backfill.
pub const BACKFILL_AFTER_MICROS: i64 = 5 * 60_000_000;

/// Backfilled rows this close together are one backfill, wide enough to join hourly datasets such as funding.
pub const BACKFILL_JOIN_MICROS: i64 = 2 * 3_600_000_000;

/// The most bars one candle read returns.
pub const CANDLE_CAP: usize = 5_000;

/// Receipt times per venue, as `venue -> micros`.
pub type Frontier = BTreeMap<String, i64>;

/// A half-open span of receipt time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Span {
    /// Start, in micros.
    pub from: i64,
    /// End, in micros.
    pub to: i64,
}

/// Sorted instants folded into runs, broken wherever two neighbours are further apart than `gap`.
pub fn runs(mut at: Vec<i64>, gap: i64) -> Vec<Span> {
    at.sort_unstable();
    let mut out: Vec<Span> = Vec::new();
    for t in at {
        match out.last_mut() {
            Some(run) if t - run.to <= gap => run.to = t,
            _ => out.push(Span { from: t, to: t }),
        }
    }
    out
}

/// Intervals merged where they touch or overlap.
pub fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_unstable_by_key(|s| (s.from, s.to));
    let mut out: Vec<Span> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(last) if s.from <= last.to => last.to = last.to.max(s.to),
            _ => out.push(s),
        }
    }
    out
}

/// The newest arrival each venue's archive holds, from segment names alone.
///
/// A `t-` cursor names arrival micros on our clock, so no parquet is opened.
pub fn archive_frontier(root: &Path) -> Frontier {
    let mut out = Frontier::new();
    for (venue, venue_dir) in levels(root, "venue=") {
        for (kind, kind_dir) in levels(&venue_dir, "kind=") {
            if kind == "failures" {
                continue;
            }
            let Some((_, newest)) = levels(&kind_dir, "date=").into_iter().last() else {
                continue;
            };
            for (cursor, _) in galata_segments::list_segments(&newest) {
                if let galata_segments::Cursor::Time { last_micros, .. } = cursor {
                    let seen = out.entry(venue.clone()).or_insert(i64::MIN);
                    *seen = (*seen).max(last_micros);
                }
            }
        }
    }
    out
}

/// Segments in each venue's partition for `date`, per kind.
pub fn archive_segments_on(root: &Path, date: &str) -> BTreeMap<(String, String), usize> {
    let mut out = BTreeMap::new();
    for (venue, venue_dir) in levels(root, "venue=") {
        for (kind, kind_dir) in levels(&venue_dir, "kind=") {
            let day = kind_dir.join(format!("date={date}"));
            if day.is_dir() {
                out.insert(
                    (venue.clone(), kind),
                    galata_segments::list_segments(&day).len(),
                );
            }
        }
    }
    out
}

/// Each venue's archive as runs of arrival time, from segment names alone.
pub fn archive_spans(root: &Path) -> BTreeMap<String, Vec<Span>> {
    let mut out = BTreeMap::new();
    for (venue, venue_dir) in levels(root, "venue=") {
        let mut spans = Vec::new();
        for (kind, kind_dir) in levels(&venue_dir, "kind=") {
            if kind == "failures" {
                continue;
            }
            for (_, day) in levels(&kind_dir, "date=") {
                for (cursor, _) in galata_segments::list_segments(&day) {
                    if let galata_segments::Cursor::Time {
                        first_micros,
                        last_micros,
                        ..
                    } = cursor
                    {
                        spans.push(Span {
                            from: first_micros,
                            to: last_micros + RUN_BREAK_MICROS,
                        });
                    }
                }
            }
        }
        let merged = merge(spans)
            .into_iter()
            .map(|s| Span {
                from: s.from,
                to: s.to - RUN_BREAK_MICROS,
            })
            .collect();
        out.insert(venue, merged);
    }
    out
}

/// `prefix<name>` directories under `dir`, sorted by name.
fn levels(dir: &Path, prefix: &str) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.strip_prefix(prefix)?.to_owned();
            Some((name, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Every row of one tape dataset, or none when the dataset is unwritten.
fn read_kind(
    root: &Path,
    kind: Kind,
    ticker: Option<String>,
) -> Result<Vec<RecordBatch>, TapeError> {
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];
    if !galata_datawatch::tape::reader::unwritten(root, &scopes).is_empty() {
        return Ok(Vec::new());
    }
    let unreadable = |error: &dyn std::fmt::Display| TapeError::Unreadable {
        detail: error.to_string(),
    };
    let reader =
        galata_datawatch::tape::reader::Reader::open(root, &scopes).map_err(|e| unreadable(&e))?;
    reader
        .view(galata_datawatch::tape::reader::Window {
            kind,
            from_micros: i64::MIN + 1,
            to_micros: i64::MAX,
            ticker,
        })
        .map_err(|e| unreadable(&e))
}

fn text<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray, TapeError> {
    column(batch, name, "Utf8")
}

fn int<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int64Array, TapeError> {
    column(batch, name, "Int64")
}

fn dec<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Decimal128Array, TapeError> {
    column(batch, name, "Decimal128")
}

fn column<'a, T: 'static>(
    batch: &'a RecordBatch,
    name: &str,
    want: &str,
) -> Result<&'a T, TapeError> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<T>())
        .ok_or_else(|| TapeError::Unrenderable {
            column: name.to_owned(),
            data_type: format!("expected {want}"),
        })
}

/// The newest arrival each venue's tape holds, across every served dataset.
pub fn tape_frontier(root: &Path) -> Frontier {
    let mut out = Frontier::new();
    for kind in tape::SERVED {
        let Ok(batches) = read_kind(root, kind, None) else {
            continue;
        };
        for batch in &batches {
            let (Ok(venue), Ok(recv)) = (text(batch, "venue"), int(batch, "recv_micros")) else {
                continue;
            };
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || recv.is_null(i) {
                    continue;
                }
                let seen = out.entry(venue.value(i).to_owned()).or_insert(i64::MIN);
                *seen = (*seen).max(recv.value(i));
            }
        }
    }
    out
}

/// One recorded gap interval.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GapSpan {
    /// The cause, as the capture wrote it.
    pub cause: String,
    /// Start, in micros.
    pub from: i64,
    /// End, in micros.
    pub to: i64,
}

/// One `(venue, dataset)` over time.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Lane {
    /// The venue.
    pub venue: String,
    /// The dataset.
    pub kind: String,
    /// Runs of receipt time that hold rows.
    pub held: Vec<Span>,
    /// Recorded gaps, merged within each cause.
    pub gaps: Vec<GapSpan>,
    /// Runs of bars that arrived late, sent by a backfill.
    pub backfilled: Vec<Span>,
    /// What the archive holds beyond this lane's tape, if anything.
    pub archive_only: Option<Span>,
}

/// One venue's archive over time.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ArchiveLane {
    /// The venue.
    pub venue: String,
    /// Runs of arrival time the archive holds segments for.
    pub spans: Vec<Span>,
}

/// The record over time, as the Overview draws it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Timeline {
    /// One lane per `(venue, dataset)` the tape holds, gaps excluded.
    pub lanes: Vec<Lane>,
    /// One lane per venue the archive holds.
    pub archive: Vec<ArchiveLane>,
}

/// The record as intervals. A gap is only ever what the record wrote down.
pub fn timeline(tape_root: &Path, archive_root: &Path) -> Result<Timeline, TapeError> {
    let mut held: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
    let mut late: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
    for kind in tape::SERVED {
        if kind == Kind::Gaps {
            continue;
        }
        for batch in &read_kind(tape_root, kind, None)? {
            let venue = text(batch, "venue")?;
            let recv = int(batch, "recv_micros")?;
            let at = int(batch, "at_micros").ok();
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || recv.is_null(i) {
                    continue;
                }
                let key = (venue.value(i).to_owned(), kind.as_str().to_owned());
                let r = recv.value(i);
                held.entry(key.clone()).or_default().push(r);
                if let Some(at) = at.filter(|a| !a.is_null(i))
                    && r - at.value(i) > BACKFILL_AFTER_MICROS
                {
                    late.entry(key).or_default().push(at.value(i));
                }
            }
        }
    }

    let mut gaps: BTreeMap<(String, String), BTreeMap<String, Vec<Span>>> = BTreeMap::new();
    for batch in &read_kind(tape_root, Kind::Gaps, None)? {
        let venue = text(batch, "venue")?;
        let series = text(batch, "series")?;
        let cause = text(batch, "cause")?;
        let from = int(batch, "from_micros")?;
        let to = int(batch, "to_micros")?;
        for i in 0..batch.num_rows() {
            gaps.entry((venue.value(i).to_owned(), series.value(i).to_owned()))
                .or_default()
                .entry(cause.value(i).to_owned())
                .or_default()
                .push(Span {
                    from: from.value(i),
                    to: to.value(i),
                });
        }
    }

    let archive = archive_frontier(archive_root);
    let lanes = held
        .into_iter()
        .map(|((venue, kind), at)| {
            let held = runs(at, RUN_BREAK_MICROS);
            let newest = held.last().map(|s| s.to);
            let key = (venue.clone(), kind.clone());
            let gaps = gaps
                .get(&key)
                .map(|by_cause| {
                    by_cause
                        .iter()
                        .flat_map(|(cause, spans)| {
                            merge(spans.clone()).into_iter().map(|s| GapSpan {
                                cause: cause.clone(),
                                from: s.from,
                                to: s.to,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let backfilled = late
                .remove(&key)
                .map(|at| runs(at, BACKFILL_JOIN_MICROS))
                .unwrap_or_default();
            let archive_only = match (newest, archive.get(&venue)) {
                (Some(tape), Some(&arch)) if arch > tape => Some(Span {
                    from: tape,
                    to: arch,
                }),
                _ => None,
            };
            Lane {
                venue,
                kind,
                held,
                gaps,
                backfilled,
                archive_only,
            }
        })
        .collect();

    Ok(Timeline {
        lanes,
        archive: archive_spans(archive_root)
            .into_iter()
            .map(|(venue, spans)| ArchiveLane { venue, spans })
            .collect(),
    })
}

/// A bar width the candle read accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    /// One minute.
    M1,
    /// Five minutes.
    M5,
    /// Fifteen minutes.
    M15,
    /// One hour.
    H1,
}

impl Interval {
    /// Every accepted spelling.
    pub const NAMES: &'static str = "1m, 5m, 15m, 1h";

    /// From its spelling.
    pub fn parse(name: &str) -> Option<Interval> {
        Some(match name {
            "1m" => Interval::M1,
            "5m" => Interval::M5,
            "15m" => Interval::M15,
            "1h" => Interval::H1,
            _ => return None,
        })
    }

    /// Its width in micros.
    pub fn micros(self) -> i64 {
        60_000_000
            * match self {
                Interval::M1 => 1,
                Interval::M5 => 5,
                Interval::M15 => 15,
                Interval::H1 => 60,
            }
    }
}

/// One bar. Prices and sizes are decimal strings.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Bar {
    /// The bar's open time, venue micros.
    pub at_micros: i64,
    /// First open.
    pub open: String,
    /// Highest high.
    pub high: String,
    /// Lowest low.
    pub low: String,
    /// Last close.
    pub close: String,
    /// Summed volume.
    pub volume: String,
    /// Whether any minute in it arrived by a backfill.
    pub backfilled: bool,
    /// Whether every minute in it had closed.
    pub is_final: bool,
}

/// One instrument's candles at one interval.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Candles {
    /// The venue.
    pub venue: String,
    /// The instrument.
    pub ticker: String,
    /// The interval, as asked.
    pub interval: String,
    /// Oldest first.
    pub bars: Vec<Bar>,
    /// Whether older bars were left out to stay under the cap.
    pub capped: bool,
}

/// One minute as the tape last stated it, in raw decimal units.
#[derive(Debug, Clone, Copy)]
struct Minute {
    at: i64,
    recv: i64,
    open: i128,
    high: i128,
    low: i128,
    close: i128,
    volume: i128,
    is_final: bool,
}

/// Candles folded to the last state per bar time, then resampled. No float is used.
pub fn candles(
    root: &Path,
    venue: &str,
    ticker: &str,
    interval: Interval,
    interval_name: &str,
    from: i64,
    to: i64,
) -> Result<Candles, TapeError> {
    if to <= from {
        return Err(TapeError::Backwards { from, to });
    }
    let mut folded: BTreeMap<i64, Minute> = BTreeMap::new();
    let mut scale: i8 = 18;
    for batch in &read_kind(root, Kind::Candles, Some(ticker.to_owned()))? {
        let v = text(batch, "venue")?;
        let t = text(batch, "ticker")?;
        let at = int(batch, "at_micros")?;
        let recv = int(batch, "recv_micros")?;
        let open = dec(batch, "open")?;
        let high = dec(batch, "high")?;
        let low = dec(batch, "low")?;
        let close = dec(batch, "close")?;
        let volume = dec(batch, "volume")?;
        let is_final = column::<BooleanArray>(batch, "is_final", "Boolean").ok();
        if let DataType::Decimal128(_, s) = open.data_type() {
            scale = *s;
        }
        for i in 0..batch.num_rows() {
            if v.value(i) != venue || t.value(i) != ticker || at.is_null(i) {
                continue;
            }
            let a = at.value(i);
            if a < from || a >= to {
                continue;
            }
            let row = Minute {
                at: a,
                recv: recv.value(i),
                open: open.value(i),
                high: high.value(i),
                low: low.value(i),
                close: close.value(i),
                volume: volume.value(i),
                is_final: is_final.is_some_and(|f| !f.is_null(i) && f.value(i)),
            };
            match folded.get(&a) {
                Some(seen) if seen.recv > row.recv => {}
                _ => {
                    folded.insert(a, row);
                }
            }
        }
    }
    let mut bars = resample(folded.into_values(), interval.micros(), scale);
    let capped = bars.len() > CANDLE_CAP;
    if capped {
        bars.drain(..bars.len() - CANDLE_CAP);
    }
    Ok(Candles {
        venue: venue.to_owned(),
        ticker: ticker.to_owned(),
        interval: interval_name.to_owned(),
        bars,
        capped,
    })
}

/// Minutes, ascending, into bars of `width` micros.
fn resample(minutes: impl Iterator<Item = Minute>, width: i64, scale: i8) -> Vec<Bar> {
    struct Open {
        at: i64,
        open: i128,
        high: i128,
        low: i128,
        close: i128,
        volume: i128,
        late: bool,
        is_final: bool,
    }
    let close_bar = |b: Open| Bar {
        at_micros: b.at,
        open: fixed(b.open, scale),
        high: fixed(b.high, scale),
        low: fixed(b.low, scale),
        close: fixed(b.close, scale),
        volume: fixed(b.volume, scale),
        backfilled: b.late,
        is_final: b.is_final,
    };
    let mut out = Vec::new();
    let mut current: Option<Open> = None;
    for m in minutes {
        let at = m.at - m.at.rem_euclid(width);
        let late = m.recv - m.at > BACKFILL_AFTER_MICROS;
        match current.as_mut() {
            Some(b) if b.at == at => {
                b.high = b.high.max(m.high);
                b.low = b.low.min(m.low);
                b.close = m.close;
                b.volume += m.volume;
                b.late |= late;
                b.is_final &= m.is_final;
            }
            _ => {
                if let Some(done) = current.take() {
                    out.push(close_bar(done));
                }
                current = Some(Open {
                    at,
                    open: m.open,
                    high: m.high,
                    low: m.low,
                    close: m.close,
                    volume: m.volume,
                    late,
                    is_final: m.is_final,
                });
            }
        }
    }
    if let Some(done) = current {
        out.push(close_bar(done));
    }
    out
}

/// A raw decimal at `scale`, spelled the way the tape spells it.
pub fn fixed(raw: i128, scale: i8) -> String {
    if scale <= 0 {
        return raw.to_string();
    }
    let scale = scale as u32;
    let unit = 10i128.pow(scale);
    let sign = if raw < 0 { "-" } else { "" };
    let abs = raw.unsigned_abs();
    let unit = unit as u128;
    format!(
        "{sign}{}.{:0width$}",
        abs / unit,
        abs % unit,
        width = scale as usize
    )
}

/// One `(venue, dataset)` cell of the Overview board.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Cell {
    /// The venue.
    pub venue: String,
    /// The dataset.
    pub kind: String,
    /// Rows the tape holds.
    pub tape_rows: usize,
    /// Segments in the archive's partition for today, or null where the archive keeps no such kind.
    pub archive_today: Option<usize>,
    /// The newest day the tape covers for it, if any.
    pub coverage_date: Option<String>,
    /// Micros held on that day.
    pub covered_micros: i64,
    /// Micros that day accounts for.
    pub window_micros: i64,
    /// Gap rows the record wrote against this dataset.
    pub gap_rows: usize,
}

/// Every `(venue, dataset)` the tape holds, as one read.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Datasets {
    /// The UTC day `archive_today` counts.
    pub today: String,
    /// Venue-major, dataset order as served.
    pub cells: Vec<Cell>,
}

/// The Overview's grid.
pub fn board(tape_root: &Path, archive_root: &Path, today: &str) -> Result<Datasets, TapeError> {
    let held = tape::instruments(tape_root);
    let mut rows: BTreeMap<(String, String), usize> = BTreeMap::new();
    for i in &held.instruments {
        *rows.entry((i.venue.clone(), i.kind.clone())).or_default() += i.rows;
    }
    let mut gap_rows: BTreeMap<(String, String), usize> = BTreeMap::new();
    for batch in &read_kind(tape_root, Kind::Gaps, None)? {
        let venue = text(batch, "venue")?;
        let series = text(batch, "series")?;
        for i in 0..batch.num_rows() {
            *gap_rows
                .entry((venue.value(i).to_owned(), series.value(i).to_owned()))
                .or_default() += 1;
        }
    }
    let covered = tape::covered_days(tape_root);
    let archive = archive_segments_on(archive_root, today);
    let venues: BTreeSet<String> = rows.keys().map(|(v, _)| v.clone()).collect();

    let mut cells = Vec::new();
    for venue in venues {
        for kind in tape::SERVED {
            let key = (venue.clone(), kind.as_str().to_owned());
            let Some(&tape_rows) = rows.get(&key) else {
                continue;
            };
            let day = covered
                .days
                .iter()
                .filter(|d| d.venue == venue && d.kind == key.1)
                .max_by(|a, b| a.date.cmp(&b.date));
            cells.push(Cell {
                venue: venue.clone(),
                kind: key.1.clone(),
                tape_rows,
                archive_today: archive.get(&key).copied(),
                coverage_date: day.map(|d| d.date.clone()),
                covered_micros: day.map_or(0, |d| d.covered_micros),
                window_micros: day.map_or(0, |d| d.window_micros),
                gap_rows: gap_rows.get(&key).copied().unwrap_or(0),
            });
        }
    }
    Ok(Datasets {
        today: today.to_owned(),
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minute(at: i64, o: i128, h: i128, l: i128, c: i128, v: i128) -> Minute {
        Minute {
            at,
            recv: at + 1,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            is_final: true,
        }
    }

    #[test]
    fn runs_break_where_receipts_are_far_apart() {
        let r = runs(vec![0, 10, 20, 200, 210], 60);
        assert_eq!(
            r,
            vec![Span { from: 0, to: 20 }, Span { from: 200, to: 210 }]
        );
    }

    #[test]
    fn merge_joins_touching_and_overlapping_spans() {
        let m = merge(vec![
            Span { from: 5, to: 10 },
            Span { from: 0, to: 5 },
            Span { from: 20, to: 30 },
            Span { from: 25, to: 26 },
        ]);
        assert_eq!(m, vec![Span { from: 0, to: 10 }, Span { from: 20, to: 30 }]);
    }

    #[test]
    fn fifteen_minutes_resample_to_one_bar() {
        let m = 60_000_000;
        let minutes = (0..15).map(|i| {
            minute(
                i * m,
                100 + i as i128,
                110 + i as i128,
                90 - i as i128,
                101 + i as i128,
                2,
            )
        });
        let bars = resample(minutes, 15 * m, 0);
        assert_eq!(bars.len(), 1);
        let b = &bars[0];
        assert_eq!(
            (
                b.open.as_str(),
                b.high.as_str(),
                b.low.as_str(),
                b.close.as_str(),
                b.volume.as_str()
            ),
            ("100", "124", "76", "115", "30")
        );
        assert!(!b.backfilled);
    }

    #[test]
    fn a_late_minute_marks_its_bar_backfilled() {
        let m = 60_000_000;
        let mut late = minute(0, 1, 1, 1, 1, 1);
        late.recv = BACKFILL_AFTER_MICROS + 1;
        let bars = resample([late, minute(m, 1, 1, 1, 1, 1)].into_iter(), 5 * m, 0);
        assert!(bars[0].backfilled);
    }

    #[test]
    fn decimals_are_spelled_at_their_scale() {
        assert_eq!(
            fixed(85_398_000_000_000_000_000_000, 18),
            "85398.000000000000000000"
        );
        assert_eq!(fixed(-5, 2), "-0.05");
        assert_eq!(fixed(7, 0), "7");
    }

    #[test]
    fn an_unknown_interval_is_refused() {
        assert!(Interval::parse("2m").is_none());
        assert_eq!(Interval::parse("15m"), Some(Interval::M15));
    }
}
