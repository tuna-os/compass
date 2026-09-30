//! Currency exchange rates for the calculator: the European Central Bank's
//! daily reference rates.
//!
//! The C++ Numen backend fetches rates from the Vicinae API
//! (`NumenVicinaeCurrencyProvider`); a fork has no such API, so Compass reads
//! the ECB's free daily file ([`ECB_DAILY_URL`]): about thirty currencies
//! against the euro, published once a working day. What is kept is plain
//! data — the ECB's date, when it was fetched, and each currency's rate per
//! euro — cached as JSON under Compass's cache directory
//! ([`cache_path`]) so a launcher started offline still converts.
//!
//! Fetching is the caller's (the engine's HTTP client); [`RateCache::refresh`]
//! takes the fetch as a closure, so the refresh rules are tested without a
//! network.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The ECB's daily reference rates.
pub const ECB_DAILY_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";

/// Overrides [`ECB_DAILY_URL`], for tests and mirrors.
pub const URL_ENV: &str = "COMPASS_EXCHANGE_RATES_URL";

/// Set, the engine never refreshes the rates on its own; Refresh Exchange
/// Rates still does. The C++'s `Environment::isAutoRateRefreshDisabled`.
pub const DISABLE_AUTO_REFRESH_ENV: &str = "COMPASS_DISABLE_AUTO_RATE_REFRESH";

/// The currency every rate is quoted against.
pub const BASE_CURRENCY: &str = "EUR";

/// How old fetched rates may get before they are fetched again. The ECB
/// publishes once a working day.
pub const MAX_AGE_SECS: i64 = 24 * 60 * 60;

/// The cache file's name under Compass's cache directory.
const CACHE_FILE: &str = "exchange-rates.json";

/// Why the ECB's file was not understood.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// Not XML.
    #[error("the exchange rates are not XML: {0}")]
    Xml(String),
    /// XML without a dated `Cube`.
    #[error("the exchange rates carry no date")]
    NoDate,
    /// A rate that is not a positive number.
    #[error("the exchange rate for {currency} is not a number: {rate}")]
    BadRate {
        /// The currency.
        currency: String,
        /// What the file said.
        rate: String,
    },
    /// A dated `Cube` without rates.
    #[error("the exchange rates list no currency")]
    Empty,
}

/// One day's rates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExchangeRates {
    /// The ECB's reference date, `YYYY-MM-DD`.
    pub date: String,
    /// When they were fetched, in seconds since the epoch.
    pub fetched_at: i64,
    /// Units of each currency per euro, by ISO 4217 code, the euro included
    /// at 1.
    pub rates: BTreeMap<String, f64>,
}

impl ExchangeRates {
    /// Units of `currency` per euro, whatever its case.
    #[must_use]
    pub fn rate(&self, currency: &str) -> Option<f64> {
        self.rates
            .get(currency)
            .or_else(|| self.rates.get(&currency.to_ascii_uppercase()))
            .copied()
    }

    /// Whether these are due to be fetched again at `now`: older than
    /// [`MAX_AGE_SECS`], or from a clock that has since gone back.
    #[must_use]
    pub fn is_stale(&self, now: i64) -> bool {
        now < self.fetched_at || now - self.fetched_at >= MAX_AGE_SECS
    }
}

/// Reads the ECB's `eurofxref-daily.xml`, fetched at `fetched_at`.
///
/// # Errors
///
/// [`ParseError`] when the file is not the ECB's shape.
pub fn parse_ecb(xml: &str, fetched_at: i64) -> Result<ExchangeRates, ParseError> {
    let document =
        roxmltree::Document::parse(xml).map_err(|error| ParseError::Xml(error.to_string()))?;
    let day = document
        .descendants()
        .find(|node| node.has_tag_name("Cube") && node.attribute("time").is_some())
        .ok_or(ParseError::NoDate)?;
    let date = day.attribute("time").unwrap_or_default().to_owned();
    let mut rates = BTreeMap::new();
    for cube in day.children().filter(|node| node.has_tag_name("Cube")) {
        let (Some(currency), Some(text)) = (cube.attribute("currency"), cube.attribute("rate"))
        else {
            continue;
        };
        let rate = text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|rate| rate.is_finite() && *rate > 0.0)
            .ok_or_else(|| ParseError::BadRate {
                currency: currency.to_owned(),
                rate: text.to_owned(),
            })?;
        rates.insert(currency.to_ascii_uppercase(), rate);
    }
    if rates.is_empty() {
        return Err(ParseError::Empty);
    }
    rates.insert(BASE_CURRENCY.to_owned(), 1.0);
    Ok(ExchangeRates {
        date,
        fetched_at,
        rates,
    })
}

/// Where the rates are cached under `cache_home` (`$XDG_CACHE_HOME`).
#[must_use]
pub fn cache_path(cache_home: &Path) -> PathBuf {
    cache_home.join("compass").join(CACHE_FILE)
}

/// The URL to fetch from: `override_url` ([`URL_ENV`]) when set and not
/// empty, else [`ECB_DAILY_URL`].
#[must_use]
pub fn source_url(override_url: Option<&str>) -> String {
    override_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(ECB_DAILY_URL)
        .to_owned()
}

/// The rates in hand and their cache file.
#[derive(Debug, Default)]
pub struct RateCache {
    path: Option<PathBuf>,
    current: Option<ExchangeRates>,
}

impl RateCache {
    /// A cache kept in `path`, starting from what the file holds. A missing
    /// or unreadable file is no rates, not an error.
    #[must_use]
    pub fn load(path: Option<PathBuf>) -> Self {
        let current = path.as_deref().and_then(|path| {
            let bytes = std::fs::read(path).ok()?;
            match serde_json::from_slice::<ExchangeRates>(&bytes) {
                Ok(rates) if !rates.rates.is_empty() => Some(rates),
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "ignoring unreadable exchange rates");
                    None
                }
            }
        });
        Self { path, current }
    }

    /// The rates in hand.
    #[must_use]
    pub fn current(&self) -> Option<&ExchangeRates> {
        self.current.as_ref()
    }

    /// Whether the rates should be fetched at `now`: there are none, or
    /// they are stale.
    #[must_use]
    pub fn needs_refresh(&self, now: i64) -> bool {
        self.current
            .as_ref()
            .is_none_or(|rates| rates.is_stale(now))
    }

    /// Fetches the ECB's file with `fetch` and, when it reads, keeps and
    /// caches it. A failed fetch or an unreadable answer leaves the rates in
    /// hand as they were.
    ///
    /// # Errors
    ///
    /// The sentence to show: why the fetch failed, or why its answer was not
    /// understood.
    pub fn refresh(
        &mut self,
        fetch: impl FnOnce() -> Result<Vec<u8>, String>,
        now: i64,
    ) -> Result<&ExchangeRates, String> {
        let bytes = fetch()?;
        let xml = String::from_utf8_lossy(&bytes);
        let rates = parse_ecb(&xml, now).map_err(|error| error.to_string())?;
        if let Some(path) = &self.path
            && let Err(error) = save(path, &rates)
        {
            tracing::warn!(%error, path = %path.display(), "could not cache the exchange rates");
        }
        Ok(self.current.insert(rates))
    }
}

fn save(path: &Path, rates: &ExchangeRates) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(rates).map_err(std::io::Error::other)?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, json)?;
    std::fs::rename(&partial, path)
}
