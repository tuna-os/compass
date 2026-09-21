//! The desktop's interface typeface, as the launcher receives it.
//!
//! The launcher must not hard-code its typeface. GNOME's interface font is
//! `org.gnome.desktop.interface font-name` (a Pango description such as
//! `Cantarell 11`), and the user can change it to any installed family.
//! Inside a Flatpak the portal `org.freedesktop.portal.Settings` proxies that
//! key; outside it `gsettings get org.gnome.desktop.interface font-name`
//! reads it directly. Either way the family travels as a single string and
//! is applied to every `text` and `text_input` in the window.
//!
//! The stack is still needed: a family named by the desktop may not be
//! installed on a minimal image that still runs the launcher. So the
//! primary family is tried first and the historic `FONT_STACK` remains as
//! fallbacks, which is also what the browser surrogate under `tools/design/`
//! needs for close-enough metrics.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::design::FONT_STACK;

/// A Pango font description of the form `"Family [Style] Size"`.
///
/// Examples: `Cantarell 11`, `Adwaita Sans 11`, `Cantarell Bold 11`,
/// `Noto Sans CJK JP 9`.
#[derive(Debug, Clone, PartialEq)]
pub struct FontDescription {
    /// Family and optional style, without the trailing size.
    pub family: String,
    /// Point size, if one was present.
    pub size: Option<f32>,
}

impl FontDescription {
    /// Parse a Pango description.
    ///
    /// The size is the last whitespace-separated token that parses as a
    /// number. Everything before it is the family/style. A single-token
    /// string with no size yields that token as the family.
    #[must_use]
    pub fn parse(description: &str) -> Self {
        let trimmed = description
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .trim();
        if trimmed.is_empty() {
            return Self {
                family: String::new(),
                size: None,
            };
        }
        let mut parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            return Self {
                family: String::new(),
                size: None,
            };
        }
        let last = parts.last().copied().unwrap_or("");
        if let Ok(size) = last.parse::<f32>() {
            parts.pop();
            let family = parts.join(" ");
            Self {
                family: if family.is_empty() {
                    last.to_owned()
                } else {
                    family
                },
                size: Some(size),
            }
        } else {
            Self {
                family: trimmed.to_owned(),
                size: None,
            }
        }
    }

    /// Family only, as the desktop configured it.
    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }
}

/// Extract the family from a Pango description.
///
/// Convenience for the one-line portal → `AppFlags` path.
#[must_use]
pub fn family_from_description(description: &str) -> String {
    FontDescription::parse(description).family
}

/// The ordered font stack the launcher tries.
///
/// The system's family first, then the historic fallbacks that keep the
/// launcher readable on a minimal image missing that family. Duplicates are
/// removed while preserving order.
#[must_use]
pub fn font_stack(primary: &str) -> Vec<String> {
    let mut stack = Vec::new();
    let primary = primary.trim();
    if !primary.is_empty() {
        stack.push(primary.to_owned());
    }
    for fallback in FONT_STACK {
        if !stack.iter().any(|existing| existing == fallback) {
            stack.push((*fallback).to_owned());
        }
    }
    stack
}

/// The iced font for a family, falling back to the default weight/style.
///
/// Leaks the family string to obtain a `'static` reference, which is
/// acceptable: one family is leaked for the lifetime of the process and the
/// launcher holds exactly one.
#[must_use]
pub fn iced_font(family: &str) -> iced::Font {
    let family = family.trim();
    if family.is_empty() {
        return iced::Font::DEFAULT;
    }
    let leaked: &'static str = Box::leak(family.to_owned().into_boxed_str());
    iced::Font {
        family: iced::font::Family::Name(leaked),
        ..iced::Font::DEFAULT
    }
}

/// The window's end of the typography channel, mirroring `AppearanceLink`.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The window's end of the font-family channel.
#[derive(Debug, Clone)]
pub struct TypographyLink {
    id: u64,
    updates: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
}

/// The feeder's end.
pub type TypographySender = mpsc::UnboundedSender<String>;

impl TypographyLink {
    /// A new link and the sender that drives it.
    #[must_use]
    pub fn new() -> (Self, TypographySender) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                updates: Arc::new(Mutex::new(Some(receiver))),
            },
            sender,
        )
    }

    fn take_updates(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        self.updates.lock().ok()?.take()
    }

    /// A subscription yielding each new family name.
    pub fn subscription(&self) -> iced::Subscription<String> {
        iced::Subscription::run_with(self.clone(), |link| {
            let updates = link.take_updates();
            iced::futures::stream::unfold(updates, |updates| async move {
                let mut updates = updates?;
                let next = updates.recv().await?;
                Some((next, Some(updates)))
            })
        })
    }
}

impl std::hash::Hash for TypographyLink {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cantarell_11() {
        let desc = FontDescription::parse("Cantarell 11");
        assert_eq!(desc.family, "Cantarell");
        assert_eq!(desc.size, Some(11.0));
    }

    #[test]
    fn parse_adwaita_sans_11() {
        let desc = FontDescription::parse("Adwaita Sans 11");
        assert_eq!(desc.family, "Adwaita Sans");
        assert_eq!(desc.size, Some(11.0));
    }

    #[test]
    fn parse_with_style_before_size() {
        let desc = FontDescription::parse("Cantarell Bold 11");
        assert_eq!(desc.family, "Cantarell Bold");
        assert_eq!(desc.size, Some(11.0));
    }

    #[test]
    fn parse_quoted_gsettings_output() {
        let desc = FontDescription::parse("'Cantarell 11'");
        assert_eq!(desc.family, "Cantarell");
        assert_eq!(desc.size, Some(11.0));
    }

    #[test]
    fn parse_double_quoted() {
        let desc = FontDescription::parse("\"Inter 10\"");
        assert_eq!(desc.family, "Inter");
        assert_eq!(desc.size, Some(10.0));
    }

    #[test]
    fn parse_without_size() {
        let desc = FontDescription::parse("Cantarell");
        assert_eq!(desc.family, "Cantarell");
        assert_eq!(desc.size, None);
    }

    #[test]
    fn font_stack_puts_system_first_and_deduplicates() {
        let stack = font_stack("Cantarell");
        assert_eq!(stack.first().map(String::as_str), Some("Cantarell"));
        // Cantarell appears only once even though FONT_STACK starts with it.
        assert_eq!(
            stack.iter().filter(|family| *family == "Cantarell").count(),
            1
        );
        // Fallbacks still present.
        assert!(stack.contains(&"Inter".to_owned()));
    }

    #[test]
    fn font_stack_with_empty_primary_is_just_fallbacks() {
        let stack = font_stack("");
        assert_eq!(
            stack,
            FONT_STACK
                .iter()
                .map(|family| (*family).to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn font_stack_with_custom_family_prepends_it() {
        let stack = font_stack("Ubuntu");
        assert_eq!(stack.first().map(String::as_str), Some("Ubuntu"));
        assert!(stack.contains(&"Cantarell".to_owned()));
    }

    #[test]
    fn iced_font_for_empty_is_default() {
        assert_eq!(iced_font(""), iced::Font::DEFAULT);
        assert_eq!(iced_font("   "), iced::Font::DEFAULT);
    }
}
