//! Coverage by day: how much of each day the record holds.
//!
//! **The question nothing else answered**: *is this day usable?* That a day
//! exists, that it wants compacting, and that some time is missing across the
//! whole record are three different facts, and none of them is this one.

use std::collections::BTreeMap;
use std::path::Path;

use arrow::array::{Array, Int64Array, StringArray};
use galata_wire::Kind;
use serde::Serialize;
use utoipa::ToSchema;

use crate::tape::SERVED;

/// Microseconds in a day.
pub(crate) const DAY_MICROS: i64 = 86_400_000_000;

/// Microseconds in an hour.
pub(crate) const HOUR_MICROS: i64 = 3_600_000_000;

/// One day of one dataset, and how much of it the record holds.
#[derive(Debug, Serialize, ToSchema)]
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
#[derive(Debug, Serialize, ToSchema)]
pub struct Covered {
    /// Newest day first.
    pub days: Vec<DayCoverage>,
}

/// How much of each day the record holds.
///
/// **Our clock throughout.** `recv_micros` is what the partitions are dated by,
/// what a gap's bounds are written in, and what *did we have this data* means.
/// The venue's clock answers when something happened there, which is a
/// different question, and a figure mixing the two would be neither.
pub fn covered_days(root: &Path) -> Covered {
    // (venue, day) -> gap intervals, in our clock.
    let mut gaps_by_day: BTreeMap<(String, String), Vec<(i64, i64)>> = BTreeMap::new();
    super::gaps::collect_gaps(root, &mut gaps_by_day);

    let mut windows = DayWindows::new();
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
            day_window(kind, batch, &mut windows);
        }
    }

    covered_from(windows, gaps_by_day)
}

/// (venue, kind, day) -> (first, last, rows): each day's window as folded.
pub(crate) type DayWindows = BTreeMap<(String, String, String), (i64, i64, usize)>;

/// One batch's arrivals folded into each day's window — the per-batch half of
/// [`covered_days`], shared with the board's single pass over the tape.
pub(crate) fn day_window(
    kind: Kind,
    batch: &arrow::array::RecordBatch,
    windows: &mut DayWindows,
) {
    let venue = crate::column!(batch, "venue", StringArray, "Utf8");
    let recv = crate::column!(batch, "recv_micros", Int64Array, "Int64");
    let (Ok(venue), Ok(recv)) = (venue, recv) else {
        return;
    };
    // Folded on a borrowed venue and the euclidean day first: probing the
    // global map built two owned strings and formatted a date per row, for
    // keys a batch repeats tens of thousands of times.
    let mut local: BTreeMap<(&str, i64), (i64, i64, usize)> = BTreeMap::new();
    for i in 0..batch.num_rows() {
        if venue.is_null(i) || recv.is_null(i) {
            continue;
        }
        let at = recv.value(i);
        local
            .entry((venue.value(i), at.div_euclid(DAY_MICROS)))
            .and_modify(|w| {
                w.0 = w.0.min(at);
                w.1 = w.1.max(at);
                w.2 += 1;
            })
            .or_insert((at, at, 1));
    }
    for ((venue, _), (first, last, rows)) in local {
        // `date_of` buckets by the same euclidean day, so any arrival of the
        // bucket names its date; formatted once per (venue, day) per batch.
        let key = (
            venue.to_owned(),
            kind.as_str().to_owned(),
            galata_datawatch::date_of(first),
        );
        windows
            .entry(key)
            .and_modify(|w| {
                w.0 = w.0.min(first);
                w.1 = w.1.max(last);
                w.2 += rows;
            })
            .or_insert((first, last, rows));
    }
}

/// The day table, from the folded windows and the stated gaps — the
/// post-fold half of [`covered_days`].
pub(crate) fn covered_from(
    mut windows: DayWindows,
    gaps_by_day: BTreeMap<(String, String), Vec<(i64, i64)>>,
) -> Covered {
    // The extent each day's gaps account for, per venue — a stated absence is
    // part of what the record accounts for, so it widens the window.
    let mut gap_extent: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();
    for (key, spans) in &gaps_by_day {
        let first = spans.iter().map(|s| s.0).min().unwrap_or(0);
        let last = spans.iter().map(|s| s.1).max().unwrap_or(0);
        gap_extent.insert(key.clone(), (first, last));
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

/// One interval, cut at every midnight it crosses.
pub(crate) fn split_by_day(from: i64, to: i64) -> Vec<(String, (i64, i64))> {
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
pub(crate) fn missing_within(spans: &[(i64, i64)], from: i64, to: i64) -> i64 {
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
    let (total, ..) = super::gaps::union_micros(&mut clipped);
    total
}

/// One hour of one dataset.
#[derive(Debug, Serialize, ToSchema)]
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
#[derive(Debug, Serialize, ToSchema)]
pub struct Rates {
    /// How many hours were returned.
    pub hours: usize,
    /// Whether a cap applied — so a short answer and a capped one differ.
    pub capped: bool,
    /// Newest first.
    pub buckets: Vec<HourlyRows>,
}

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
            let venue = crate::column!(batch, "venue", StringArray, "Utf8");
            let recv = crate::column!(batch, "recv_micros", Int64Array, "Int64");
            let (Ok(venue), Ok(recv)) = (venue, recv) else {
                continue;
            };
            // Counted on borrowed keys first; the owned strings are built
            // once per (venue, hour) a batch holds, not once per row.
            let mut local: BTreeMap<(&str, i64), usize> = BTreeMap::new();
            for i in 0..batch.num_rows() {
                if venue.is_null(i) || recv.is_null(i) {
                    continue;
                }
                *local
                    .entry((venue.value(i), hour_of(recv.value(i))))
                    .or_insert(0) += 1;
            }
            for ((venue, hour), rows) in local {
                *counted
                    .entry((venue.to_owned(), kind.as_str().to_owned(), hour))
                    .or_insert(0) += rows;
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
pub(crate) fn hour_of(micros: i64) -> i64 {
    micros.div_euclid(HOUR_MICROS) * HOUR_MICROS
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
