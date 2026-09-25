//! Current prices from the archive's tail.
//!
//! The newest payloads on disk are replayed through the venue's own normaliser.
//! Nothing here subscribes to a bus; the archive is the record, and it is on
//! disk seconds after the venue said it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use galata_datawatch::adapters::{self, AdapterConfig};
use galata_datawatch::config::{Adapters, Config};
use galata_datawatch::venue::Adapter;
use galata_wire::Event;
use serde::Serialize;

use crate::shape::Frontier;

/// The first window searched, before it widens.
const FIRST_WINDOW: Duration = Duration::from_secs(120);
/// The widest window searched.
const WIDEST_WINDOW: Duration = Duration::from_secs(900);
/// How long one answer is shared.
const SHARED_FOR: Duration = Duration::from_secs(1);

/// Accepts every declared venue; the tower builds normalisers only for those compiled in.
struct Lenient;

impl Adapters for Lenient {
    fn supplies(&self, _venue: &str, _series: galata_wire::Series) -> bool {
        true
    }
    fn known(&self, _venue: &str) -> bool {
        true
    }
    fn known_names(&self) -> Vec<&'static str> {
        adapters::known()
    }
}

/// A venue's normaliser and the instruments it declares.
pub struct Venue {
    adapter: Box<dyn Adapter>,
    tickers: Vec<String>,
}

/// What the configuration yielded: normalisers, and why any venue has none.
pub struct Normalisers {
    venues: BTreeMap<String, Venue>,
    /// Why a declared venue has no normaliser.
    pub refusals: Vec<String>,
}

impl Normalisers {
    /// None at all, for printing the contract.
    pub fn none() -> Normalisers {
        Normalisers {
            venues: BTreeMap::new(),
            refusals: Vec::new(),
        }
    }

    /// Built from the same document the capture reads.
    pub fn load(path: &Path) -> Normalisers {
        let config = Config::load_from(path, &Lenient).map_err(|e| e.to_string());
        let config = match config {
            Ok(config) => config,
            Err(refusal) => {
                return Normalisers {
                    venues: BTreeMap::new(),
                    refusals: vec![format!(
                        "{} could not be read ({refusal}); set GALATA_CONFIG",
                        path.display()
                    )],
                };
            }
        };
        let mut venues = BTreeMap::new();
        let mut refusals = Vec::new();
        for (name, declared) in &config.venue {
            if !adapters::known().contains(&name.as_str()) {
                refusals.push(format!("no normaliser is compiled in for {name}"));
                continue;
            }
            match AdapterConfig::for_replay(name, declared).and_then(adapters::build) {
                Ok(adapter) => {
                    venues.insert(
                        name.clone(),
                        Venue {
                            adapter,
                            tickers: declared
                                .instruments
                                .iter()
                                .map(|i| i.ticker.clone())
                                .collect(),
                        },
                    );
                }
                Err(refusal) => refusals.push(format!("{name}: {refusal}")),
            }
        }
        Normalisers { venues, refusals }
    }
}

/// One instrument's newest quote and trade. Prices and sizes are decimal strings.
#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct Price {
    /// The instrument.
    pub ticker: String,
    /// Best bid.
    pub bid: Option<String>,
    /// Best ask.
    pub ask: Option<String>,
    /// Bid size.
    pub bid_size: Option<String>,
    /// Ask size.
    pub ask_size: Option<String>,
    /// The quote's venue time.
    pub quote_at_micros: Option<i64>,
    /// When the quote arrived, our clock.
    pub quote_recv_micros: Option<i64>,
    /// Last trade price.
    pub last: Option<String>,
    /// Last trade size.
    pub last_size: Option<String>,
    /// The trade's venue time.
    pub trade_at_micros: Option<i64>,
    /// When the trade arrived, our clock.
    pub trade_recv_micros: Option<i64>,
}

/// One venue's newest prices.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct VenuePrices {
    /// The venue.
    pub venue: String,
    /// The newest arrival its archive holds, our clock.
    pub frontier_micros: i64,
    /// How far back from the frontier was searched.
    pub window_secs: u64,
    /// Every declared instrument, priced or not.
    pub prices: Vec<Price>,
}

/// Every venue's newest prices, and why any venue has none.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Latest {
    /// One entry per venue with a normaliser and an archive.
    pub venues: Vec<VenuePrices>,
    /// Why a declared venue is not priced.
    pub refusals: Vec<String>,
}

/// An answer shared by every browser for a second.
#[derive(Default)]
pub struct Shared(Mutex<Option<(Instant, Arc<Latest>)>>);

impl Shared {
    /// The shared answer, recomputed only once it is a second old.
    pub fn get(&self, compute: impl FnOnce() -> Latest) -> Arc<Latest> {
        let mut held = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, answer)) = held.as_ref()
            && at.elapsed() < SHARED_FOR
        {
            return Arc::clone(answer);
        }
        let answer = Arc::new(compute());
        *held = Some((Instant::now(), Arc::clone(&answer)));
        answer
    }
}

/// Every venue's newest prices, read from the archive's tail.
pub fn latest(archive: &Path, frontier: &Frontier, normalisers: &Normalisers) -> Latest {
    let mut venues = Vec::new();
    let mut refusals = normalisers.refusals.clone();
    for (name, venue) in &normalisers.venues {
        let Some(&edge) = frontier.get(name) else {
            refusals.push(format!("the archive holds nothing for {name}"));
            continue;
        };
        let mut window = FIRST_WINDOW;
        let prices = loop {
            let prices = read_window(archive, name, venue, edge, window);
            let complete = venue
                .tickers
                .iter()
                .all(|t| prices.get(t).is_some_and(|p| p.bid.is_some()));
            if complete || window >= WIDEST_WINDOW {
                break prices;
            }
            window = (window * 2).min(WIDEST_WINDOW);
        };
        venues.push(VenuePrices {
            venue: name.clone(),
            frontier_micros: edge,
            window_secs: window.as_secs(),
            prices: venue
                .tickers
                .iter()
                .map(|t| {
                    prices.get(t).cloned().unwrap_or(Price {
                        ticker: t.clone(),
                        ..Price::default()
                    })
                })
                .collect(),
        });
    }
    Latest { venues, refusals }
}

/// The newest quote and trade per ticker in `[edge - window, edge]`.
fn read_window(
    archive: &Path,
    name: &str,
    venue: &Venue,
    edge: i64,
    window: Duration,
) -> BTreeMap<String, Price> {
    let quotes = format!("venue={name}/kind=quotes");
    let trades = format!("venue={name}/kind=trades");
    let scopes = [quotes.as_str(), trades.as_str()];
    let from = edge - window.as_micros() as i64;
    let Ok(payloads) = galata_datawatch::replay::read_range(archive, Some(&scopes), from, edge + 1)
    else {
        return BTreeMap::new();
    };
    let mut out: BTreeMap<String, Price> = BTreeMap::new();
    for replayed in &payloads {
        let Ok(envelopes) = venue.adapter.normalise(replayed.payload()) else {
            continue;
        };
        for e in envelopes {
            let Some(ticker) = e.ticker().map(ToString::to_string) else {
                continue;
            };
            let price = out.entry(ticker.clone()).or_insert_with(|| Price {
                ticker,
                ..Price::default()
            });
            match &e.event {
                Event::Quote(q) if price.quote_recv_micros.is_none_or(|r| r <= e.recv_micros) => {
                    price.bid = q.bid_px.map(|n| n.to_string());
                    price.ask = q.ask_px.map(|n| n.to_string());
                    price.bid_size = q.bid_sz.map(|n| n.to_string());
                    price.ask_size = q.ask_sz.map(|n| n.to_string());
                    price.quote_at_micros = e.at_micros;
                    price.quote_recv_micros = Some(e.recv_micros);
                }
                Event::Trade(t) if price.trade_recv_micros.is_none_or(|r| r <= e.recv_micros) => {
                    price.last = Some(t.price.to_string());
                    price.last_size = Some(t.size.to_string());
                    price.trade_at_micros = e.at_micros;
                    price.trade_recv_micros = Some(e.recv_micros);
                }
                _ => {}
            }
        }
    }
    out
}
