//! The Portfolio view's two reads: the ledger's fold report, and statistics
//! derived from the tape.
//!
//! **Neither is computed here.** The fold report is the ledger's, written after
//! each events pass and **passed through unchanged**: re-shaping it would be a
//! second statement of the fold's vocabulary, free to drift from the first.
//! The statistics are `galata_datawatch::derive`'s, over the tape root this
//! tower already reads. The tower never reads `var/ledger`, holds no address
//! and no key: the report, which names accounts by alias only, is the door the
//! ledger opened for it.

use std::path::Path;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// The ledger's fold report for one venue, as the ledger wrote it.
#[derive(Debug, Serialize, ToSchema)]
pub struct Portfolio {
    /// The report, verbatim: accounts by alias, books, breaks, skews, snapshot
    /// checks, cash per dex, and why there is no equity. Absent when the
    /// ledger has not written one.
    #[schema(value_type = Object)]
    pub report: Option<serde_json::Value>,
    /// When the file was written, our clock. Its age is shown beside it.
    pub written_micros: Option<i64>,
    /// Why there is no report, where there is none.
    pub reason: Option<String>,
}

/// Which venue's report.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PortfolioQuery {
    /// The venue.
    pub venue: String,
}

/// The fold report for a venue, or why there is none. **Never an error for an
/// absent file**: a ledger that has not folded yet is an empty state.
pub fn read_report(dir: &Path, venue: &str) -> Portfolio {
    let path = dir.join(format!("ledger-fold-{venue}.json"));
    let written_micros = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_micros() as i64);
    match std::fs::read_to_string(&path) {
        Err(_) => Portfolio {
            report: None,
            written_micros: None,
            reason: Some(format!(
                "no fold report at {}: the ledger has not run the fold for {venue}",
                path.display()
            )),
        },
        Ok(text) => match serde_json::from_str(&text) {
            Ok(value) => Portfolio {
                report: Some(value),
                written_micros,
                reason: None,
            },
            // The ledger writes through a rename, so a half-written file is
            // not expected; an unreadable one is said, not guessed at.
            Err(e) => Portfolio {
                report: None,
                written_micros,
                reason: Some(format!("the fold report would not parse: {e}")),
            },
        },
    }
}

/// A statistics request. **The floor and z have no defaults**, as in the
/// library: the reader chooses them.
#[derive(Debug, Deserialize, IntoParams)]
pub struct StatisticsQuery {
    /// The venue.
    pub venue: String,
    /// The bucket, spelled as a bar width: `30m`, `1h`.
    pub horizon: String,
    /// Start of the window, venue time, microseconds.
    pub from: i64,
    /// End, exclusive.
    pub to: i64,
    /// Returns a cell needs before it is a figure.
    pub min_observations: usize,
    /// Standard errors the interval on ρ spans.
    pub z: f64,
    /// The instrument betas are taken against.
    pub reference: Option<String>,
}

/// The statistics, as the library derived them: every cell with its n and
/// backfilled share, absent cells naming why, and the tape bound read to.
#[derive(Debug, Serialize, ToSchema)]
pub struct Derived {
    /// `galata_datawatch::derive::tape::Derived`, verbatim.
    #[schema(value_type = Object)]
    pub derived: serde_json::Value,
}

/// Derive, or say why the request cannot be.
pub fn derive(tape: &Path, query: &StatisticsQuery) -> Result<Derived, String> {
    let width = galata_datawatch::derive::tape::width_micros(&query.horizon).ok_or_else(|| {
        format!(
            "{} is not a horizon; spell it as a bar width, 30m or 1h",
            query.horizon
        )
    })?;
    if query.to <= query.from {
        return Err("the window runs backwards: to must be after from".to_string());
    }
    if !(query.z.is_finite() && query.z > 0.0) {
        return Err("z must be a positive number of standard errors".to_string());
    }
    let derived = galata_datawatch::derive::tape::from_tape(
        tape,
        &query.venue,
        &galata_datawatch::derive::Horizon {
            bucket_secs: width / 1_000_000,
            from_micros: query.from,
            to_micros: query.to,
            min_observations: query.min_observations,
            z: query.z,
            reference: query.reference.clone(),
        },
    )
    .map_err(|e| e.to_string())?;
    serde_json::to_value(derived)
        .map(|derived| Derived { derived })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh directory under the system's temporary one, as the tower's other
    /// tests make theirs: no dependency for a directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("galata-tower-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_report_yet_is_said_plainly() {
        let dir = scratch("no-report");
        let p = read_report(&dir, "hyperliquid");
        assert!(p.report.is_none());
        assert!(p.reason.unwrap().contains("has not run the fold"));
    }

    #[test]
    fn the_report_is_passed_through_unchanged() {
        let dir = scratch("report");
        let written = r#"{"venue":"hyperliquid","accounts":{"main":{"breaks":[{"ticker":"BTC","folded":"2","stated":"3"}]}}}"#;
        std::fs::write(dir.join("ledger-fold-hyperliquid.json"), written).unwrap();
        let p = read_report(&dir, "hyperliquid");
        let expected: serde_json::Value = serde_json::from_str(written).unwrap();
        assert_eq!(
            p.report.unwrap(),
            expected,
            "byte for byte, as the ledger wrote it"
        );
        assert!(p.written_micros.is_some());
    }

    #[test]
    fn a_backwards_window_or_an_unknown_horizon_is_refused() {
        let tape = PathBuf::from("/nonexistent");
        let q = |horizon: &str, from, to| StatisticsQuery {
            venue: "hyperliquid".into(),
            horizon: horizon.into(),
            from,
            to,
            min_observations: 20,
            z: 2.0,
            reference: None,
        };
        assert!(
            derive(&tape, &q("30q", 0, 1))
                .unwrap_err()
                .contains("not a horizon")
        );
        assert!(
            derive(&tape, &q("30m", 5, 1))
                .unwrap_err()
                .contains("backwards")
        );
    }
}
