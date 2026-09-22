//! What does asking "has the tape grown?" cost, against reading the rows?
use std::path::Path;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new("../galata-datawatch/var/tape");
    let kind = galata_wire::Kind::Quotes;
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];

    for label in ["warm-up", "measured"] {
        let t = Instant::now();
        let mut bound = 0;
        for _ in 0..20 {
            let reader = galata_datawatch::tape::reader::Reader::open(root, &scopes)?;
            bound = reader.bound().position;
        }
        let open_only = t.elapsed() / 20;

        let t = Instant::now();
        let mut rows = 0;
        for _ in 0..20 {
            let view = galata_tower_lib_probe(root, kind)?;
            rows = view;
        }
        let full = t.elapsed() / 20;
        println!(
            "{label}: open+bound {open_only:?} (bound {bound}) | full view {full:?} ({rows} rows)"
        );
    }
    Ok(())
}

fn galata_tower_lib_probe(
    root: &Path,
    kind: galata_wire::Kind,
) -> Result<usize, Box<dyn std::error::Error>> {
    let scope = format!("kind={}", kind.as_str());
    let scopes = [scope.as_str()];
    let reader = galata_datawatch::tape::reader::Reader::open(root, &scopes)?;
    let window = galata_datawatch::tape::reader::Window {
        kind,
        from_micros: 0,
        to_micros: 9_000_000_000_000_000,
        ticker: None,
    };
    let batches = reader.view(window)?;
    Ok(batches.iter().map(|b| b.num_rows()).sum())
}
