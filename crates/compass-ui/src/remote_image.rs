//! Images an extension names by `http(s)` URL: fetched once, kept on disk.
//!
//! The C++ fetches through a `QNetworkDiskCache` under its cache directory
//! with `PreferCache`: a URL fetched once is served from disk from then on,
//! whatever its headers say. Kept, since extension images are overwhelmingly
//! avatars and favicons whose URLs change when their content does. Compass
//! keeps its own cache (ADR-0017), under `$XDG_CACHE_HOME/compass/images`,
//! one file per URL named by the URL's SHA-256, with the extension of what
//! the bytes turned out to be.
//!
//! Only what the launcher can draw is kept: PNG, JPEG and SVG, recognised by
//! their bytes rather than by the URL or the server's `Content-Type`, both of
//! which lie often enough. Anything else is a failed fetch and the row keeps
//! its initial.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::Digest as _;

/// The largest image fetched, in bytes.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How long one fetch may take, connection to last byte.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// How much the cache may hold before the oldest files go, in bytes.
pub const CACHE_BUDGET: u64 = 256 * 1024 * 1024;

/// What the cache recognises.
const KINDS: [&str; 3] = ["png", "jpg", "svg"];

/// Whether `url` is one this module fetches.
#[must_use]
pub fn is_remote(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// The cache directory, or `None` without a cache home.
#[must_use]
pub fn cache_dir() -> Option<PathBuf> {
    compass_core::xdg_dirs::cache_home().map(|home| home.join("compass").join("images"))
}

fn stem(url: &str) -> String {
    hex(&sha2::Sha256::digest(url.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The file `url` is cached in under `dir`, if it has been fetched.
#[must_use]
pub fn cached_in(dir: &Path, url: &str) -> Option<PathBuf> {
    let stem = stem(url);
    KINDS
        .iter()
        .map(|kind| dir.join(format!("{stem}.{kind}")))
        .find(|path| path.is_file())
}

/// What `bytes` are, as a file extension, when the launcher can draw them.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("jpg");
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    (head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")))
        .then_some("svg")
}

/// Keeps `bytes` for `url` under `dir`, returning the file.
///
/// # Errors
///
/// A sentence: not an image the launcher draws, or the write failed.
pub fn store(dir: &Path, url: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let kind = sniff(bytes).ok_or_else(|| format!("{url} is not a PNG, JPEG or SVG image"))?;
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join(format!("{}.{kind}", stem(url)));
    // Written beside and renamed over, so a reader never sees half a file.
    let mut partial =
        tempfile::NamedTempFile::new_in(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    partial
        .write_all(bytes)
        .map_err(|err| format!("{}: {err}", dir.display()))?;
    partial
        .persist(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(path)
}

/// Removes the least recently written files until `dir` holds at most
/// `budget` bytes.
pub fn prune(dir: &Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let meta = entry.metadata().ok().filter(std::fs::Metadata::is_file)?;
            Some((meta.modified().ok()?, meta.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Fetches `url` into the cache, or finds it there. Blocking: call it off
/// the UI thread.
///
/// # Errors
///
/// A sentence naming the URL and what went wrong.
pub fn fetch(url: &str) -> Result<PathBuf, String> {
    let dir = cache_dir().ok_or("no cache directory to keep images in")?;
    if let Some(path) = cached_in(&dir, url) {
        return Ok(path);
    }
    let bytes = download(url)?;
    let path = store(&dir, url, &bytes)?;
    prune(&dir, CACHE_BUDGET);
    Ok(path)
}

/// One task per URL, each fetching on a blocking thread and answering
/// [`crate::message::Message::ExtensionImageFetched`].
pub fn fetch_tasks(urls: Vec<String>) -> iced::Task<crate::message::Message> {
    iced::Task::batch(urls.into_iter().map(|url| {
        iced::Task::perform(
            async move {
                let result = {
                    let url = url.clone();
                    tokio::task::spawn_blocking(move || fetch(&url))
                        .await
                        .unwrap_or_else(|err| Err(format!("the image fetch failed: {err}")))
                };
                (url, result)
            },
            |(url, result)| crate::message::Message::ExtensionImageFetched { url, result },
        )
    }))
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent("Compass (vicinae)")
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|err| format!("{url}: {err}"))?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|err| format!("{url}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn only_http_and_https_are_remote() {
        assert!(is_remote("https://example.com/a.png"));
        assert!(is_remote("HTTP://example.com/a.png"));
        assert!(!is_remote("file:///tmp/a.png"));
        assert!(!is_remote("data:image/png;base64,AAAA"));
        assert!(!is_remote("htt"));
    }

    #[test]
    fn the_bytes_decide_the_kind_not_the_url() {
        assert_eq!(sniff(PNG), Some("png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(
            sniff(b"  <svg xmlns='http://www.w3.org/2000/svg'/>"),
            Some("svg")
        );
        assert_eq!(sniff(b"<?xml version='1.0'?>\n<svg/>"), Some("svg"));
        assert_eq!(
            sniff(b"<!doctype html><html>"),
            None,
            "an error page is not an image"
        );
        assert_eq!(sniff(b"GIF89a"), None, "the launcher cannot draw a GIF");
    }

    #[test]
    fn a_stored_image_is_found_again_and_a_different_url_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = "https://example.com/avatar";
        assert_eq!(cached_in(dir.path(), url), None);
        let path = store(dir.path(), url, PNG).expect("stored");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        assert_eq!(cached_in(dir.path(), url), Some(path));
        assert_eq!(cached_in(dir.path(), "https://example.com/other"), None);
        assert!(store(dir.path(), url, b"<html>").is_err());
    }

    #[test]
    fn pruning_removes_the_oldest_until_the_budget_holds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join("old.png");
        let new = dir.path().join("new.png");
        std::fs::write(&old, [0u8; 100]).unwrap();
        let past = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();
        std::fs::write(&new, [0u8; 100]).unwrap();
        prune(dir.path(), 150);
        assert!(!old.exists(), "the oldest goes first");
        assert!(new.exists());
    }
}
