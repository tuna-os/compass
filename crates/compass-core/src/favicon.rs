//! Which service a site's favicon is fetched from: `FaviconService`.
//!
//! A port of `src/server/src/favicon`, minus the fetching and the cache,
//! which are the window's (`compass_ui::remote_image`). The C++ asks one of
//! two web services for a domain's icon, chosen by `favicon_service` in the
//! configuration: `twenty` (its default), `google`, or `none` to fetch
//! nothing and show the placeholder the caller falls back to.

/// A favicon service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Service {
    /// `https://twenty-icons.com`, the C++ default.
    #[default]
    Twenty,
    /// Google's `s2/favicons`.
    Google,
    /// Favicons are not fetched.
    None,
}

/// The size asked for, in pixels: the first of the C++ requesters' sizes
/// (`{128, 64, 32, 16}`), which is the only one they ever ask for, the
/// fallback to the smaller ones not being connected.
pub const SIZE: u32 = 128;

impl Service {
    /// The service `favicon_service` names (`twenty`, `google`, `none`);
    /// `None` for anything else, which the C++ logs and ignores, keeping the
    /// service it had.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "twenty" => Some(Self::Twenty),
            "google" => Some(Self::Google),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// The service a configuration's top-level `favicon_service` names, the
    /// default when it names none or an unknown one.
    #[must_use]
    pub fn from_config(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(serde_json::Value::as_str)
            .and_then(Self::from_id)
            .unwrap_or_default()
    }

    /// The image URL for `domain`'s favicon, or `None` when favicons are off
    /// or there is no domain.
    #[must_use]
    pub fn url(self, domain: &str) -> Option<String> {
        if domain.is_empty() {
            return None;
        }
        match self {
            Self::Twenty => Some(format!("https://twenty-icons.com/{domain}/{SIZE}")),
            Self::Google => Some(format!(
                "https://www.google.com/s2/favicons?domain={domain}&sz={SIZE}"
            )),
            Self::None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_service_asks_for_the_cpps_url_and_none_asks_nothing() {
        assert_eq!(
            Service::Twenty.url("example.com").as_deref(),
            Some("https://twenty-icons.com/example.com/128")
        );
        assert_eq!(
            Service::Google.url("example.com").as_deref(),
            Some("https://www.google.com/s2/favicons?domain=example.com&sz=128")
        );
        assert_eq!(Service::None.url("example.com"), None);
        assert_eq!(Service::Twenty.url(""), None);
    }

    #[test]
    fn the_configuration_names_the_service_and_twenty_is_the_default() {
        let read = |value: serde_json::Value| Service::from_config(Some(&value));
        assert_eq!(read(serde_json::json!("google")), Service::Google);
        assert_eq!(read(serde_json::json!("none")), Service::None);
        assert_eq!(read(serde_json::json!("bing")), Service::Twenty);
        assert_eq!(Service::from_config(None), Service::Twenty);
    }
}
