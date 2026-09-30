//! The calculator's exchange rates, engine side: the ECB's daily reference
//! rates fetched through the engine's HTTP client, cached under Compass's
//! cache directory, refreshed daily and on Refresh Exchange Rates (IPC v21).
//!
//! Ports what `NumenVicinaeCurrencyProvider` and `NumenCalculatorBackend`
//! do with rates — load the cache at start, fetch when it is stale, refresh
//! on a timer and on demand — over a different source: the ECB rather than
//! the Vicinae API (`compass_core::exchange_rates`). The launcher computes,
//! so it asks for the rates ([`compass_ipc::Request::ExchangeRates`]) each
//! time its window opens.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use compass_core::exchange_rates::{self, ExchangeRates, RateCache};
use compass_ipc::{ErrorKind, ExchangeRateTable, ProtocolError, Request, Response};

/// The largest rates file accepted. The ECB's is under two kilobytes.
const MAX_BYTES: u64 = 256 * 1024;

/// How often the engine looks whether the rates are due, as the C++'s
/// hourly `m_rateRefreshTimer`; they are fetched once a day
/// ([`exchange_rates::MAX_AGE_SECS`]).
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// Where the rates come from and what is held.
#[derive(Debug, Default)]
pub struct ExchangeRateService {
    /// The rates file's URL; `None` has no source, and every refresh fails.
    url: Option<String>,
    cache: tokio::sync::Mutex<Option<RateCache>>,
    cache_path: Option<PathBuf>,
}

/// Seconds since the epoch.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The wire form of `rates`.
#[must_use]
pub fn table(rates: &ExchangeRates) -> ExchangeRateTable {
    ExchangeRateTable {
        date: rates.date.clone(),
        fetched_at: rates.fetched_at,
        rates: rates
            .rates
            .iter()
            .map(|(code, rate)| (code.clone(), rate.to_string()))
            .collect(),
    }
}

impl ExchangeRateService {
    /// The ECB (or [`exchange_rates::URL_ENV`]'s URL), cached under
    /// `$XDG_CACHE_HOME/compass`.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::new(
            Some(exchange_rates::source_url(
                std::env::var(exchange_rates::URL_ENV).ok().as_deref(),
            )),
            compass_core::xdg_dirs::cache_home().map(|home| exchange_rates::cache_path(&home)),
        )
    }

    /// Rates fetched from `url` and cached in `cache_path`. Nothing is read
    /// until they are first asked for.
    #[must_use]
    pub fn new(url: Option<String>, cache_path: Option<PathBuf>) -> Self {
        Self {
            url,
            cache: tokio::sync::Mutex::default(),
            cache_path,
        }
    }

    async fn with_cache<T>(&self, f: impl FnOnce(&mut RateCache) -> T) -> T {
        let mut slot = self.cache.lock().await;
        let cache = match &mut *slot {
            Some(cache) => cache,
            None => {
                let path = self.cache_path.clone();
                let loaded = tokio::task::spawn_blocking(move || RateCache::load(path))
                    .await
                    .unwrap_or_default();
                slot.insert(loaded)
            }
        };
        f(cache)
    }

    /// The rates held: fetched, or read back from the cache.
    pub async fn current(&self) -> Option<ExchangeRates> {
        self.with_cache(|cache| cache.current().cloned()).await
    }

    /// Whether the rates are due at `now`.
    pub async fn needs_refresh(&self, now: i64) -> bool {
        self.with_cache(|cache| cache.needs_refresh(now)).await
    }

    /// Fetches the rates now, whatever their age. A failure keeps the rates
    /// held.
    ///
    /// # Errors
    ///
    /// Why the rates could not be fetched or read.
    pub async fn refresh(&self) -> Result<ExchangeRates, String> {
        let fetched = match &self.url {
            Some(url) => crate::stores::get(url.clone(), MAX_BYTES).await,
            None => Err("there is no exchange rate source".to_owned()),
        };
        let at = now();
        self.with_cache(move |cache| cache.refresh(move || fetched, at).cloned())
            .await
    }

    /// Answers [`Request::ExchangeRates`] and
    /// [`Request::RefreshExchangeRates`].
    pub async fn answer(&self, request: Request) -> Response {
        match request {
            Request::RefreshExchangeRates => match self.refresh().await {
                Ok(rates) => Response::ExchangeRates {
                    rates: Some(table(&rates)),
                },
                Err(reason) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not refresh the exchange rates: {reason}"),
                )),
            },
            _ => Response::ExchangeRates {
                rates: self.current().await.as_ref().map(table),
            },
        }
    }
}

/// Keeps the rates fresh for as long as the engine runs: fetched at start
/// when there are none or they are stale, then looked at hourly. Nothing is
/// done when [`exchange_rates::DISABLE_AUTO_REFRESH_ENV`] is set to anything
/// but the empty string, as the C++'s `isAutoRateRefreshDisabled` (which
/// counts the empty string as set too).
pub async fn run(service: Arc<ExchangeRateService>) {
    if compass_xdg::brand::env_var_os(exchange_rates::DISABLE_AUTO_REFRESH_ENV)
        .is_some_and(|value| !value.is_empty())
    {
        tracing::info!("automatic exchange rate refresh is disabled");
        return;
    }
    loop {
        if service.needs_refresh(now()).await {
            match service.refresh().await {
                Ok(rates) => tracing::info!(date = %rates.date, "fetched the exchange rates"),
                Err(error) => tracing::warn!(%error, "could not fetch the exchange rates"),
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}
