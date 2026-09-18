//! The `Wallpaper` half of the extension API.
//!
//! Ports `ExtWallpaperService`
//! (`src/server/src/extension/api/wallpaper-service.hpp`): one method, whose
//! only real decision is what to do when the extension does not say how the
//! image should fit.

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &["Wallpaper/set"];

/// How the image is laid out on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Fill the screen, cropping what does not fit. The default.
    #[default]
    Cover,
    /// Fit the whole image, leaving bars.
    Contain,
    /// Fill the screen, distorting the image.
    Stretch,
    /// Centre it at its own size.
    Center,
    /// Repeat it.
    Tile,
}

impl Fit {
    /// The C++ `mapFit`, which is a switch over the IDL's `WallpaperFit`.
    ///
    /// `None` for a name the IDL does not declare; the caller then keeps
    /// [`Fit::Cover`], which is what `value_or(WallpaperFit::Cover)` does for
    /// an absent one.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        Some(match name {
            "Cover" => Self::Cover,
            "Contain" => Self::Contain,
            "Stretch" => Self::Stretch,
            "Center" => Self::Center,
            "Tile" => Self::Tile,
            _ => return None,
        })
    }
}

/// `WallpaperRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    /// The image to set.
    pub path: String,
    /// The screen to set it on; all of them when absent.
    pub screen: Option<String>,
    /// How to lay it out.
    pub fit: Fit,
}

/// The wallpaper backend an extension can reach.
pub trait Wallpaper {
    /// `WallpaperManager::setWallpaper`. The error is what the extension's
    /// promise rejects with — the C++ returns the manager's own future, so the
    /// manager, not the service, decides what failure reads like.
    fn set(&self, request: &Request) -> Result<(), String>;
}

/// Serves `Wallpaper` from one backend.
#[derive(Debug)]
pub struct WallpaperService<W> {
    wallpaper: W,
}

impl<W: Wallpaper> WallpaperService<W> {
    /// Serves `wallpaper`.
    pub const fn new(wallpaper: W) -> Self {
        Self { wallpaper }
    }

    /// The backend this serves.
    pub const fn wallpaper(&self) -> &W {
        &self.wallpaper
    }

    /// Answers `call`, or `None` if it is not a `Wallpaper` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let options = call.params.get("options");
        let option = |name: &str| {
            options
                .and_then(|options| options.get(name))
                .filter(|value| !value.is_null())
                .and_then(serde_json::Value::as_str)
        };

        let request = Request {
            path: call
                .params
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            screen: option("screen").map(str::to_owned),
            // `options.fit.transform(mapFit).value_or(WallpaperFit::Cover)`
            fit: option("fit").and_then(Fit::from_wire).unwrap_or_default(),
        };

        Some(match self.wallpaper.set(&request) {
            Ok(()) => tsapi::reply(id, serde_json::Value::Null),
            Err(error) => tsapi::reply_error(id, &error),
        })
    }
}

impl<W: Wallpaper> tsapi::Service for WallpaperService<W> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}
