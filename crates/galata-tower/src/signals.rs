//! Market-data signals, as galata-datawatch stored them (`kind=signals`, Tier
//! 16): each horizon's newest figures, **served as stored**.
//!
//! The tower computes nothing here. A signal is a record the flow wrote when a
//! bar of its width closed, under the model its horizon declares; recomputing
//! it would be a second figure under the same name. What the tower adds is
//! only the one comparison a reader of a stored figure needs and cannot make
//! from the figure alone: whether it is **stale**, against the tower's clock.
//!
//! ```text
//!   tape/kind=signals/
//!     date=2026-09-28/   newest first; a horizon settles at the first date
//!     date=2026-09-27/   that holds it (rows are dated by asof, so no older
//!     …                  partition can hold a newer one); 14 days at most
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use arrow::array::{Array, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// How far back the read looks: twice the widest horizon declared (1w). A
/// horizon not written in 14 days is not being computed, and is not shown.
const LOOKBACK_DAYS: i64 = 14;
const DAY_MICROS: i64 = 86_400_000_000;
/// The flow's cadence: a figure is due when its next bar closes, and the run
/// that writes it comes within 30 minutes of that.
pub const GRACE_MICROS: i64 = 30 * 60 * 1_000_000;

/// Which signal.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SignalsQuery {
    /// The signal's name, as the flow writes it: `varcov`.
    pub signal: String,
}

/// Every horizon's newest figures for one signal.
#[derive(Debug, Serialize, ToSchema)]
pub struct Signals {
    /// The signal.
    pub signal: String,
    /// One entry per horizon found, narrowest first.
    pub horizons: Vec<HorizonFigures>,
    /// The tower's clock when it read: what `stale` was judged against.
    pub read_micros: i64,
}

/// One horizon's newest run, as stored.
#[derive(Debug, Serialize, ToSchema)]
pub struct HorizonFigures {
    /// `5m`, `4h`, `1w`.
    pub horizon: String,
    /// The bar width, when the horizon is one the tower can read.
    pub width_micros: Option<i64>,
    /// The close the figures stand on.
    pub asof_micros: i64,
    /// When they were computed.
    pub computed_micros: i64,
    /// Past asof + width + 30 minutes by the tower's clock: the next bar has
    /// closed and no figure for it has been written.
    pub stale: bool,
    /// How it was computed: `gjr-t/dcc`, `ewma`.
    pub model: String,
    /// The model's parameters, as JSON, verbatim.
    pub params: String,
    /// False where nothing was estimated (EWMA).
    pub fitted: bool,
    /// Every instrument named by a cell, sorted.
    pub tickers: Vec<String>,
    /// One per pair and measure.
    pub cells: Vec<SignalCell>,
}

/// A value, or why there is none.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SignalCell {
    /// `covariance` or `correlation`.
    pub measure: String,
    /// The first instrument.
    pub ticker_i: String,
    /// The second; absent for a measure about one instrument.
    pub ticker_j: Option<String>,
    /// The figure, in the dataset's units (a covariance per bar).
    pub value: Option<f64>,
    /// Why there is no figure.
    pub absent: Option<String>,
    /// The effective sample it rests on.
    pub n_eff: Option<f64>,
}

/// A horizon's bar width. `w` is a week here, which the tape's bar widths never are.
pub fn width_micros(horizon: &str) -> Option<i64> {
    match horizon.strip_suffix('w') {
        Some(n) => n.parse::<i64>().ok().map(|n| n * 7 * DAY_MICROS),
        None => galata_datawatch::derive::tape::width_micros(horizon),
    }
}

/// One row, taken from a batch by column name.
struct Row {
    horizon: String,
    asof: i64,
    computed: i64,
    model: String,
    params: String,
    fitted: bool,
    cell: SignalCell,
}

/// Each horizon's newest figures for `signal`, read from `tape`.
pub fn latest(tape: &Path, signal: &str, now_micros: i64) -> Signals {
    let root = tape.join(format!("kind={}", galata_wire::Kind::Signals));
    let oldest = galata_datawatch::calendar::date_of(now_micros - LOOKBACK_DAYS * DAY_MICROS);
    let mut dates: Vec<String> = std::fs::read_dir(&root)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_prefix("date=")
                        .map(str::to_string)
                })
                .filter(|date| *date >= oldest)
                .collect()
        })
        .unwrap_or_default();
    dates.sort_unstable_by(|a, b| b.cmp(a));

    let mut settled: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for date in dates {
        let mut here: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        for (_, path) in galata_segments::list_segments(&root.join(format!("date={date}"))) {
            // A segment that will not read is skipped, not fatal: the others
            // still hold figures, and the watch reports a broken segment.
            let Ok(batches) = galata_segments::read_segment(&path) else {
                continue;
            };
            for batch in &batches {
                for row in rows(batch, signal) {
                    if !settled.contains_key(&row.horizon) {
                        here.entry(row.horizon.clone()).or_default().push(row);
                    }
                }
            }
        }
        for (horizon, rows) in here {
            // The newest asof in the newest partition holding the horizon,
            // and of that asof the newest run, should one have been repeated.
            let best = rows
                .iter()
                .map(|r| (r.asof, r.computed))
                .max()
                .expect("a horizon came from a row");
            settled.insert(
                horizon,
                rows.into_iter()
                    .filter(|r| (r.asof, r.computed) == best)
                    .collect(),
            );
        }
    }

    let mut horizons: Vec<HorizonFigures> = settled
        .into_iter()
        .map(|(horizon, rows)| {
            let first = &rows[0];
            let width = width_micros(&horizon);
            let tickers: BTreeSet<String> = rows
                .iter()
                .flat_map(|r| {
                    std::iter::once(r.cell.ticker_i.clone()).chain(r.cell.ticker_j.clone())
                })
                .collect();
            HorizonFigures {
                stale: width.is_some_and(|w| now_micros > first.asof + w + GRACE_MICROS),
                width_micros: width,
                asof_micros: first.asof,
                computed_micros: first.computed,
                model: first.model.clone(),
                params: first.params.clone(),
                fitted: first.fitted,
                tickers: tickers.into_iter().collect(),
                cells: rows.iter().map(|r| r.cell.clone()).collect(),
                horizon,
            }
        })
        .collect();
    horizons.sort_by_key(|h| h.width_micros.unwrap_or(i64::MAX));
    Signals {
        signal: signal.to_string(),
        horizons,
        read_micros: now_micros,
    }
}

/// The widest history one request may read: 90 days of asof partitions.
pub const MAX_HISTORY_DAYS: i64 = 90;

/// One signal's history for one pair.
#[derive(Debug, Deserialize, IntoParams)]
pub struct HistoryQuery {
    /// The signal: `varcov`, `beta`.
    pub signal: String,
    /// The horizon: `4h`.
    pub horizon: String,
    /// The measure: `correlation`, `covariance`, `beta`.
    pub measure: String,
    /// The first instrument, or `*` for a figure about all of them.
    pub ticker_i: String,
    /// The second, where the measure is about a pair.
    pub ticker_j: Option<String>,
    /// How many days of asofs, back from now. 30 when absent; at most 90.
    pub days: Option<i64>,
}

/// One point of a history: a value, or why there is none.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryPoint {
    /// The close the figure stands on.
    pub asof_micros: i64,
    /// When its latest computation was made.
    pub computed_micros: i64,
    /// The figure.
    pub value: Option<f64>,
    /// Why there is no figure.
    pub absent: Option<String>,
}

/// A stored signal's history, as stored.
#[derive(Debug, Serialize, ToSchema)]
pub struct History {
    /// Oldest first; one point per asof.
    pub points: Vec<HistoryPoint>,
    /// The days read.
    pub days: i64,
}

/// One series from the date partitions of the last `days`: per asof, its latest computation.
pub fn history(tape: &Path, query: &HistoryQuery, now_micros: i64) -> Result<History, String> {
    let days = query.days.unwrap_or(30);
    if !(1..=MAX_HISTORY_DAYS).contains(&days) {
        return Err(format!(
            "days={days}: a history reads 1 to {MAX_HISTORY_DAYS} days"
        ));
    }
    let root = tape.join(format!("kind={}", galata_wire::Kind::Signals));
    let oldest = galata_datawatch::calendar::date_of(now_micros - days * DAY_MICROS);
    let mut dates: Vec<String> = std::fs::read_dir(&root)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_prefix("date=")
                        .map(str::to_string)
                })
                .filter(|date| *date >= oldest)
                .collect()
        })
        .unwrap_or_default();
    dates.sort_unstable();
    let mut by_asof: BTreeMap<i64, Row> = BTreeMap::new();
    for date in dates {
        for (_, path) in galata_segments::list_segments(&root.join(format!("date={date}"))) {
            let Ok(batches) = galata_segments::read_segment(&path) else {
                continue;
            };
            for batch in &batches {
                for row in rows(batch, &query.signal) {
                    if row.horizon != query.horizon
                        || row.cell.measure != query.measure
                        || row.cell.ticker_i != query.ticker_i
                        || row.cell.ticker_j != query.ticker_j
                    {
                        continue;
                    }
                    let newer = by_asof
                        .get(&row.asof)
                        .is_none_or(|kept| row.computed > kept.computed);
                    if newer {
                        by_asof.insert(row.asof, row);
                    }
                }
            }
        }
    }
    Ok(History {
        points: by_asof
            .into_values()
            .map(|r| HistoryPoint {
                asof_micros: r.asof,
                computed_micros: r.computed,
                value: r.cell.value,
                absent: r.cell.absent,
            })
            .collect(),
        days,
    })
}

fn rows(batch: &RecordBatch, signal: &str) -> Vec<Row> {
    let text = |name: &str| {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned())
    };
    let int = |name: &str| {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>().cloned())
    };
    let float = |name: &str| {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>().cloned())
    };
    let boolean = |name: &str| {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<BooleanArray>().cloned())
    };
    let (
        Some(sig),
        Some(horizon),
        Some(measure),
        Some(ti),
        Some(tj),
        Some(value),
        Some(absent),
        Some(n_eff),
    ) = (
        text("signal"),
        text("horizon"),
        text("measure"),
        text("ticker_i"),
        text("ticker_j"),
        float("value"),
        text("absent"),
        float("n_eff"),
    )
    else {
        return Vec::new();
    };
    let (Some(asof), Some(computed), Some(model), Some(params), Some(fitted)) = (
        int("asof_micros"),
        int("computed_micros"),
        text("model"),
        text("params"),
        boolean("fitted"),
    ) else {
        return Vec::new();
    };
    let opt_text = |a: &StringArray, i: usize| (!a.is_null(i)).then(|| a.value(i).to_string());
    let opt_float = |a: &Float64Array, i: usize| (!a.is_null(i)).then(|| a.value(i));
    (0..batch.num_rows())
        .filter(|&i| sig.value(i) == signal)
        .map(|i| Row {
            horizon: horizon.value(i).to_string(),
            asof: asof.value(i),
            computed: computed.value(i),
            model: model.value(i).to_string(),
            params: params.value(i).to_string(),
            fitted: fitted.value(i),
            cell: SignalCell {
                measure: measure.value(i).to_string(),
                ticker_i: ti.value(i).to_string(),
                ticker_j: opt_text(&tj, i),
                value: opt_float(&value, i),
                absent: opt_text(&absent, i),
                n_eff: opt_float(&n_eff, i),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const HOUR: i64 = 3_600_000_000;
    const MIDNIGHT: i64 = 1_790_553_600_000_000; // 2026-09-28T00:00Z

    /// One run's rows for `horizon` at `asof`: a BTC/ETH correlation and two
    /// variances, or all three absent with `absent`.
    fn run(tape: &Path, horizon: &str, asof: i64, computed: i64, absent: Option<&str>) {
        let rows = [
            ("covariance", "BTC", Some("BTC")),
            ("covariance", "ETH", Some("ETH")),
            ("correlation", "BTC", Some("ETH")),
        ];
        let n = rows.len();
        let text = |v: &str| Arc::new(StringArray::from(vec![v; n])) as Arc<dyn Array>;
        let batch = RecordBatch::try_new(
            galata_datawatch::signals::schema(),
            vec![
                text("varcov"),
                text(horizon),
                Arc::new(StringArray::from(
                    rows.iter().map(|r| r.0).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    rows.iter().map(|r| r.1).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    rows.iter().map(|r| r.2).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(vec![1; n])),
                Arc::new(Float64Array::from(vec![
                    if absent.is_some() {
                        None
                    } else {
                        Some(0.5)
                    };
                    n
                ])),
                Arc::new(StringArray::from(vec![absent; n])),
                Arc::new(Float64Array::from(vec![Some(32.3); n])),
                Arc::new(Int64Array::from(vec![asof; n])),
                Arc::new(Int64Array::from(vec![asof; n])),
                Arc::new(Int64Array::from(vec![computed; n])),
                Arc::new(Int64Array::from(vec![None::<i64>; n])),
                Arc::new(Int64Array::from(vec![None::<i64>; n])),
                text("ewma"),
                text("{\"lam\":0.94}"),
                Arc::new(BooleanArray::from(vec![false; n])),
                Arc::new(BooleanArray::from(vec![false; n])),
                text("abc"),
                text("run"),
            ],
        )
        .unwrap();
        galata_datawatch::signals::write(tape, computed, "run", "abc", &batch).unwrap();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "galata-tower-signals-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_newest_asof_per_horizon() {
        let tape = scratch("newest");
        run(&tape, "4h", MIDNIGHT, MIDNIGHT + 1, None);
        run(
            &tape,
            "4h",
            MIDNIGHT + 4 * HOUR,
            MIDNIGHT + 4 * HOUR + 1,
            None,
        );
        run(&tape, "1d", MIDNIGHT, MIDNIGHT + 2, None);
        // The day before: an older 1d, which the newer partition settles first.
        run(
            &tape,
            "1d",
            MIDNIGHT - 24 * HOUR,
            MIDNIGHT - 24 * HOUR + 1,
            None,
        );
        let s = latest(&tape, "varcov", MIDNIGHT + 5 * HOUR);
        let by: BTreeMap<_, _> = s.horizons.iter().map(|h| (h.horizon.as_str(), h)).collect();
        assert_eq!(by["4h"].asof_micros, MIDNIGHT + 4 * HOUR);
        assert_eq!(by["1d"].asof_micros, MIDNIGHT);
        assert_eq!(by["4h"].cells.len(), 3);
        assert_eq!(
            s.horizons
                .iter()
                .map(|h| h.horizon.as_str())
                .collect::<Vec<_>>(),
            vec!["4h", "1d"]
        );
        assert_eq!(by["4h"].tickers, vec!["BTC", "ETH"]);
    }

    #[test]
    fn a_week_old_weekly_figure_is_current() {
        let tape = scratch("stale");
        let now = MIDNIGHT + 4 * 24 * HOUR;
        run(&tape, "1w", MIDNIGHT, MIDNIGHT + 1, None);
        run(
            &tape,
            "5m",
            now - 40 * 60_000_000,
            now - 40 * 60_000_000 + 1,
            None,
        );
        let s = latest(&tape, "varcov", now);
        let by: BTreeMap<_, _> = s.horizons.iter().map(|h| (h.horizon.as_str(), h)).collect();
        assert!(
            !by["1w"].stale,
            "a weekly figure four days old is the current one"
        );
        assert!(
            by["5m"].stale,
            "a 5m figure 40 minutes old has missed its next bar"
        );
    }

    #[test]
    fn an_absent_cell_keeps_its_reason() {
        let tape = scratch("absent");
        run(
            &tape,
            "1d",
            MIDNIGHT,
            MIDNIGHT + 1,
            Some("265 returns, under min_obs=500"),
        );
        let s = latest(&tape, "varcov", MIDNIGHT + HOUR);
        for cell in &s.horizons[0].cells {
            assert_eq!(cell.value, None);
            assert_eq!(
                cell.absent.as_deref(),
                Some("265 returns, under min_obs=500")
            );
        }
    }

    fn query(days: Option<i64>) -> HistoryQuery {
        HistoryQuery {
            signal: "varcov".into(),
            horizon: "4h".into(),
            measure: "correlation".into(),
            ticker_i: "BTC".into(),
            ticker_j: Some("ETH".into()),
            days,
        }
    }

    #[test]
    fn a_repeated_computation_is_one_point() {
        let tape = scratch("history-repeat");
        run(&tape, "4h", MIDNIGHT, MIDNIGHT + 1, None);
        run(
            &tape,
            "4h",
            MIDNIGHT + 4 * HOUR,
            MIDNIGHT + 4 * HOUR + 1,
            None,
        );
        run(
            &tape,
            "4h",
            MIDNIGHT + 4 * HOUR,
            MIDNIGHT + 4 * HOUR + 9,
            None,
        );
        let h = history(&tape, &query(None), MIDNIGHT + 5 * HOUR).unwrap();
        assert_eq!(
            h.points.iter().map(|p| p.asof_micros).collect::<Vec<_>>(),
            vec![MIDNIGHT, MIDNIGHT + 4 * HOUR]
        );
        assert_eq!(h.points[1].computed_micros, MIDNIGHT + 4 * HOUR + 9);
    }

    #[test]
    fn an_absent_figure_is_a_point_with_its_reason() {
        let tape = scratch("history-absent");
        run(
            &tape,
            "4h",
            MIDNIGHT,
            MIDNIGHT + 1,
            Some("265 returns, under min_obs=500"),
        );
        let h = history(&tape, &query(None), MIDNIGHT + HOUR).unwrap();
        assert_eq!(h.points.len(), 1);
        assert_eq!(h.points[0].value, None);
        assert_eq!(
            h.points[0].absent.as_deref(),
            Some("265 returns, under min_obs=500")
        );
    }

    #[test]
    fn a_window_wider_than_ninety_days_is_refused() {
        let tape = scratch("history-wide");
        assert!(
            history(&tape, &query(Some(120)), MIDNIGHT)
                .unwrap_err()
                .contains("90")
        );
        assert!(history(&tape, &query(Some(0)), MIDNIGHT).is_err());
    }

    #[test]
    fn no_signals_is_an_empty_list() {
        let tape = scratch("none");
        let s = latest(&tape, "varcov", MIDNIGHT);
        assert!(s.horizons.is_empty());
    }

    #[test]
    fn a_partition_older_than_the_lookback_is_not_read() {
        let tape = scratch("lookback");
        run(
            &tape,
            "1d",
            MIDNIGHT - 20 * 24 * HOUR,
            MIDNIGHT - 20 * 24 * HOUR + 1,
            None,
        );
        assert!(latest(&tape, "varcov", MIDNIGHT).horizons.is_empty());
    }
}
