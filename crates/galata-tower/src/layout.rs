//! The tape's layout problems, checked once per change to the tape.
//!
//! `galata_datawatch::tape::check_layout` compares sequence ranges per label,
//! so it opens every segment's footer: 10,063 of them on 2026-09-28, 4.9 s on
//! an idle tower. `/v1/about` asked on every request, and on the async runtime,
//! which stalled every request scheduled on the same worker (the tower stopped
//! answering for about a minute under nine concurrent reads).
//!
//! **Keyed by the segment paths.** A segment's name is its sequence range, so a
//! rebuild that writes or removes one changes the set of paths; listing names
//! opens nothing. An unchanged tape answers from the last check.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Mutex;

/// The last check, and the key it is valid for.
pub type Cache = Mutex<Option<(u64, Vec<String>)>>;

/// A hash of every segment path under the tape, sorted: what a check is valid for.
pub fn key(tape: &Path) -> u64 {
    let mut paths: Vec<_> = galata_segments::partitions(tape)
        .into_iter()
        .flat_map(|partition| {
            galata_segments::list_segments(&partition)
                .into_iter()
                .map(|(_, path)| path)
        })
        .collect();
    paths.sort();
    let mut hasher = DefaultHasher::new();
    paths.hash(&mut hasher);
    hasher.finish()
}

/// The tape's layout problems: from the cache while the tape is unchanged, from `check` when it is not.
pub fn problems(
    tape: &Path,
    cache: &Cache,
    check: impl FnOnce(&Path) -> Vec<String>,
) -> Vec<String> {
    let now = key(tape);
    if let Some((held, found)) = cache
        .lock()
        .expect("the layout cache is never poisoned: nothing panics holding it")
        .as_ref()
        && *held == now
    {
        return found.clone();
    }
    let found = check(tape);
    *cache
        .lock()
        .expect("the layout cache is never poisoned: nothing panics holding it") =
        Some((now, found.clone()));
    found
}

/// The check itself, in `galata-datawatch`'s words.
pub fn check(tape: &Path) -> Vec<String> {
    galata_datawatch::tape::check_layout(tape)
        .iter()
        .map(|problem| problem.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("galata-tower-layout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("kind=quotes/date=2026-09-28")).unwrap();
        dir
    }

    fn segment(tape: &Path, name: &str) {
        std::fs::write(
            tape.join("kind=quotes/date=2026-09-28").join(name),
            b"not parquet",
        )
        .unwrap();
    }

    #[test]
    fn an_unchanged_tape_is_not_checked_twice() {
        let tape = scratch("unchanged");
        segment(&tape, "s-1_2.parquet");
        let cache = Cache::default();
        let checks = Cell::new(0);
        let count = |_: &Path| {
            checks.set(checks.get() + 1);
            vec!["a problem".to_string()]
        };
        assert_eq!(problems(&tape, &cache, count), vec!["a problem"]);
        assert_eq!(problems(&tape, &cache, count), vec!["a problem"]);
        assert_eq!(checks.get(), 1);
    }

    #[test]
    fn a_new_segment_is_checked() {
        let tape = scratch("changed");
        segment(&tape, "s-1_2.parquet");
        let cache = Cache::default();
        assert!(
            problems(&tape, &cache, check).len() == 1,
            "one unlabelled segment"
        );
        segment(&tape, "s-3_4.parquet");
        // The second segment is reported: the key moved, so the tape was checked again.
        assert_eq!(problems(&tape, &cache, check).len(), 2);
    }
}
