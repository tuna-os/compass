//! The icon a desktop notification carries, as a file the notification
//! server can read.
//!
//! `UIService::sendDesktopNotification` renders the extension's icon —
//! whatever kind of image it is — to a 128×128 PNG in the temporary
//! directory (`vicinae-notif-XXXXXX.png`) and passes that path, since the
//! freedesktop protocol takes a path or a theme icon name and nothing else.
//! Ported here for what the launcher can draw:
//!
//! - a builtin icon, rasterised from its SVG with `resvg`, tinted when the
//!   image asks for a tint (the icons are monochrome);
//! - a remote image, fetched through the launcher's image cache
//!   ([`compass_ui::remote_image`]) and then treated as a file;
//! - a file (an absolute path, `file://`, or one of the extension's assets):
//!   an SVG is rasterised, a PNG or JPEG decoded and fitted into the square
//!   (passed as it is when it does not decode, for the server to try);
//! - a file icon: the file-type icon of the path
//!   ([`compass_ui::icons::file_glyph`]), from the icon theme or the builtin
//!   `folder` / `blank-document`, drawn as a file or a builtin;
//! - a `data:` URL, decoded by the `data-url` crate and drawn as an SVG or a
//!   PNG or JPEG would be.

use std::path::{Path, PathBuf};

use compass_extension_api::view::ImageSource;

/// The side of the square the C++ renders into.
pub const SIZE: u32 = 128;

/// Where an image's parts are found.
pub struct Sources<'a> {
    /// The extension's `assets` directory.
    pub assets: Option<&'a Path>,
    /// The builtin icon set's directory.
    pub builtin_dir: Option<PathBuf>,
    /// Fetches a remote image into the cache, answering the cached file.
    pub fetch: &'a dyn Fn(&str) -> Result<PathBuf, String>,
    /// Where rendered PNGs are written.
    pub out_dir: PathBuf,
    /// Finds an icon theme's file for a name, for a file icon.
    pub find_icon: &'a dyn Fn(&str) -> Option<PathBuf>,
}

impl std::fmt::Debug for Sources<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sources")
            .field("assets", &self.assets)
            .field("builtin_dir", &self.builtin_dir)
            .field("out_dir", &self.out_dir)
            .finish_non_exhaustive()
    }
}

impl Sources<'_> {
    /// The real ones: the builtin icons where they are installed, the image
    /// cache's fetch, and the temporary directory.
    #[must_use]
    pub fn from_environment(assets: Option<&Path>) -> Sources<'_> {
        Sources {
            assets,
            builtin_dir: compass_core::builtin_icon::directory(),
            fetch: &compass_ui::remote_image::fetch,
            out_dir: std::env::temp_dir(),
            find_icon: &find_theme_icon,
        }
    }
}

/// The icon theme's file for `name`, at the size the notification is drawn.
fn find_theme_icon(name: &str) -> Option<PathBuf> {
    compass_xdg::find_icon(name, Some(&compass_xdg::default_theme()), Some(SIZE), 1.0)
}

/// The file to pass for `icon`, the JSON the extension sent: the source,
/// else its fallback. `None` when neither can be drawn.
#[must_use]
pub fn icon_path(icon: &serde_json::Value, sources: &Sources<'_>) -> Option<PathBuf> {
    let image = compass_worker_host::view_model::image_from_json(icon)?;
    let tint = image
        .tint
        .as_ref()
        .and_then(compass_ui::extension_page::color_of)
        .map(|color| color.into_rgba8());
    std::iter::once(&image.source)
        .chain(image.fallback.as_ref())
        .find_map(|source| render(source, tint, sources))
}

/// The light side of a themed source, as a notification has no theme.
fn untheme(mut source: &ImageSource) -> &ImageSource {
    while let ImageSource::Themed { light, .. } = source {
        source = light;
    }
    source
}

fn render(source: &ImageSource, tint: Option<[u8; 4]>, sources: &Sources<'_>) -> Option<PathBuf> {
    match untheme(source) {
        ImageSource::Builtin(name) => builtin(name, tint, sources),
        ImageSource::Url(url) if url.starts_with("data:") => data_url(url, &sources.out_dir),
        ImageSource::FileIcon(path) => {
            match compass_ui::icons::file_glyph(Path::new(path), sources.find_icon) {
                compass_ui::icons::Glyph::Art(art) => file(art.path(), &sources.out_dir),
                compass_ui::icons::Glyph::Builtin { name, .. } => builtin(&name, tint, sources),
            }
        }
        ImageSource::Asset(relative) => file(&sources.assets?.join(relative), &sources.out_dir),
        ImageSource::Url(url) if compass_ui::remote_image::is_remote(url) => {
            let cached = (sources.fetch)(url)
                .map_err(|reason| tracing::info!(%url, %reason, "notification icon not fetched"))
                .ok()?;
            file(&cached, &sources.out_dir)
        }
        ImageSource::Url(url) => {
            let path = Path::new(url.strip_prefix("file://")?);
            file(path, &sources.out_dir)
        }
        ImageSource::Themed { .. } => None,
    }
}

/// A builtin icon drawn from its SVG.
fn builtin(name: &str, tint: Option<[u8; 4]>, sources: &Sources<'_>) -> Option<PathBuf> {
    let file = sources
        .builtin_dir
        .as_ref()?
        .join(compass_core::builtin_icon::file_name(name)?);
    rasterize(&std::fs::read(file).ok()?, tint, &sources.out_dir)
}

/// A `data:` URL's image: an SVG rasterised, anything else decoded as a PNG
/// or JPEG.
fn data_url(url: &str, out_dir: &Path) -> Option<PathBuf> {
    let parsed = data_url::DataUrl::process(url)
        .map_err(|error| tracing::info!(?error, "notification icon is not a data URL"))
        .ok()?;
    let svg = parsed.mime_type().subtype == "svg+xml";
    let (bytes, _) = parsed
        .decode_to_vec()
        .map_err(|error| tracing::info!(?error, "notification icon's data URL does not decode"))
        .ok()?;
    if svg {
        rasterize(&bytes, None, out_dir)
    } else {
        raster(&bytes, out_dir)
    }
}

/// A file on disk as the notification takes it: an SVG drawn to a PNG, any
/// other image as it is.
fn file(path: &Path, out_dir: &Path) -> Option<PathBuf> {
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        return rasterize(&std::fs::read(path).ok()?, None, out_dir);
    }
    raster(&std::fs::read(path).ok()?, out_dir).or_else(|| Some(path.to_path_buf()))
}

/// Decodes a PNG or JPEG and fits it, aspect kept and centred, into a
/// [`SIZE`] square PNG in `out_dir`, as `decodeImageData` scales an image
/// down to the size asked for (a smaller one is centred, not enlarged).
fn raster(bytes: &[u8], out_dir: &Path) -> Option<PathBuf> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|error| tracing::info!(%error, "notification icon is not a PNG or JPEG"))
        .ok()?;
    let fitted = if decoded.width() > SIZE || decoded.height() > SIZE {
        decoded.resize(SIZE, SIZE, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    let mut canvas = image::RgbaImage::new(SIZE, SIZE);
    image::imageops::overlay(
        &mut canvas,
        &fitted.to_rgba8(),
        i64::from((SIZE - fitted.width()) / 2),
        i64::from((SIZE - fitted.height()) / 2),
    );
    let mut png = Vec::new();
    canvas
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    write_png(&png, out_dir)
}

/// Draws `svg` into a [`SIZE`] square, aspect kept and centred, tinting
/// every drawn pixel when asked, and writes it as a new PNG in `out_dir`.
fn rasterize(svg: &[u8], tint: Option<[u8; 4]>, out_dir: &Path) -> Option<PathBuf> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default())
        .map_err(|error| tracing::info!(%error, "notification icon is not an SVG"))
        .ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let size = tree.size();
    #[allow(clippy::cast_precision_loss)]
    let side = SIZE as f32;
    let scale = (side / size.width()).min(side / size.height());
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (side - size.width() * scale) / 2.0,
        (side - size.height() * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    if let Some([r, g, b, _]) = tint {
        for pixel in pixmap.pixels_mut() {
            let alpha = pixel.alpha();
            // Premultiplied: each channel is at most the alpha.
            let channel = |c: u8| u8::try_from(u16::from(c) * u16::from(alpha) / 255).unwrap_or(c);
            if let Some(tinted) = tiny_skia::PremultipliedColorU8::from_rgba(
                channel(r),
                channel(g),
                channel(b),
                alpha,
            ) {
                *pixel = tinted;
            }
        }
    }
    let png = pixmap.encode_png().ok()?;
    write_png(&png, out_dir)
}

/// Writes `png` as a new `vicinae-notif-*.png` in `out_dir`, kept.
fn write_png(png: &[u8], out_dir: &Path) -> Option<PathBuf> {
    let mut out = tempfile::Builder::new()
        .prefix("vicinae-notif-")
        .suffix(".png")
        .tempfile_in(out_dir)
        .ok()?;
    std::io::Write::write_all(&mut out, png).ok()?;
    let (_, path) = out.keep().ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12" viewBox="0 0 24 12"><rect width="24" height="12" fill="black"/></svg>"#;

    fn decoded(path: &Path) -> (u32, u32, Vec<u8>) {
        let bytes = std::fs::read(path).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&bytes).unwrap();
        (pixmap.width(), pixmap.height(), pixmap.data().to_vec())
    }

    fn pixel(data: &[u8], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * SIZE + x) * 4) as usize;
        [data[at], data[at + 1], data[at + 2], data[at + 3]]
    }

    #[test]
    fn a_builtin_icon_is_drawn_into_a_tinted_square_png() {
        let icons = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(icons.path().join("bell.svg"), SQUARE).unwrap();
        let no_fetch = |_: &str| -> Result<PathBuf, String> { panic!("nothing remote here") };
        let sources = Sources {
            assets: None,
            builtin_dir: Some(icons.path().to_path_buf()),
            fetch: &no_fetch,
            out_dir: out.path().to_path_buf(),
            find_icon: &|_| None,
        };

        let plain = icon_path(&serde_json::json!("bell"), &sources).expect("drawn");
        assert!(plain.starts_with(out.path()));
        let name = plain.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("vicinae-notif-") && name.ends_with(".png"),
            "{name}"
        );
        let (width, height, data) = decoded(&plain);
        assert_eq!((width, height), (SIZE, SIZE));
        assert_eq!(pixel(&data, 64, 64), [0, 0, 0, 255], "the middle is drawn");
        assert_eq!(
            pixel(&data, 64, 10)[3],
            0,
            "a wide icon is centred, not stretched"
        );

        let tinted = icon_path(
            &serde_json::json!({"source": {"raw": "bell"}, "tintColor": {"raw": "#ff0000"}}),
            &sources,
        )
        .expect("drawn");
        let (_, _, data) = decoded(&tinted);
        assert_eq!(pixel(&data, 64, 64), [255, 0, 0, 255]);

        assert!(
            icon_path(&serde_json::json!("no-such-icon"), &sources).is_none(),
            "not one of ours"
        );
        let with_fallback = icon_path(
            &serde_json::json!({"source": {"raw": "/nonexistent/a.png"}, "fallback": {"raw": "bell"}}),
            &sources,
        );
        assert!(with_fallback.is_some(), "the fallback is drawn instead");
    }

    #[test]
    fn a_remote_image_is_fetched_and_a_file_is_passed_or_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let png = dir.path().join("cached.png");
        std::fs::write(&png, b"\x89PNG").unwrap();
        let svg = dir.path().join("logo.svg");
        std::fs::write(&svg, SQUARE).unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("icon.png"), b"\x89PNG").unwrap();
        let fetched = std::sync::Mutex::new(Vec::new());
        let fetch = |url: &str| -> Result<PathBuf, String> {
            fetched.lock().unwrap().push(url.to_owned());
            if url.ends_with("svg") {
                Ok(svg.clone())
            } else if url.contains("broken") {
                Err("404".into())
            } else {
                Ok(png.clone())
            }
        };
        let sources = Sources {
            assets: Some(&assets),
            builtin_dir: None,
            fetch: &fetch,
            out_dir: out.path().to_path_buf(),
            find_icon: &|_| None,
        };
        let icon = |value: serde_json::Value| icon_path(&value, &sources);

        assert_eq!(
            icon(serde_json::json!("https://x.test/a.png")),
            Some(png.clone())
        );
        let drawn = icon(serde_json::json!({"source": {"raw": "https://x.test/l.svg"}})).unwrap();
        assert_eq!(decoded(&drawn).0, SIZE, "a remote SVG is drawn");
        assert_eq!(icon(serde_json::json!("https://x.test/broken.png")), None);
        assert_eq!(
            *fetched.lock().unwrap(),
            [
                "https://x.test/a.png",
                "https://x.test/l.svg",
                "https://x.test/broken.png"
            ]
        );

        assert_eq!(
            icon(serde_json::json!("icon.png")),
            Some(assets.join("icon.png"))
        );
        assert_eq!(
            icon(serde_json::json!({"source": {"themed": {"light": "icon.png", "dark": "x.png"}}})),
            Some(assets.join("icon.png")),
            "the light side"
        );
        let absolute = assets.join("icon.png").to_string_lossy().into_owned();
        assert_eq!(
            icon(serde_json::json!(format!("file://{absolute}"))),
            Some(assets.join("icon.png")),
            "a file is passed as it is"
        );
        assert_eq!(icon(serde_json::json!("/nonexistent/bell.png")), None);
        assert_eq!(icon(serde_json::json!("data:image/png;base64,AAAA")), None);
    }

    /// A 2×1 PNG: red then transparent.
    fn tiny_png() -> Vec<u8> {
        let mut canvas = image::RgbaImage::new(2, 1);
        canvas.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let mut png = Vec::new();
        canvas
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    #[test]
    fn a_data_url_is_decoded_and_drawn_into_the_square() {
        use base64::Engine as _;
        let out = tempfile::tempdir().unwrap();
        let no_fetch = |_: &str| -> Result<PathBuf, String> { panic!("nothing remote here") };
        let sources = Sources {
            assets: None,
            builtin_dir: None,
            fetch: &no_fetch,
            out_dir: out.path().to_path_buf(),
            find_icon: &|_| None,
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        let drawn = icon_path(
            &serde_json::json!(format!("data:image/png;base64,{encoded}")),
            &sources,
        )
        .expect("a PNG data URL is drawn");
        assert!(drawn.starts_with(out.path()));
        let (width, height, data) = decoded(&drawn);
        assert_eq!((width, height), (SIZE, SIZE));
        assert_eq!(
            pixel(&data, 63, 63),
            [255, 0, 0, 255],
            "centred, not enlarged"
        );
        assert_eq!(pixel(&data, 0, 0)[3], 0);

        let svg = icon_path(
            &serde_json::json!(format!(
                "data:image/svg+xml;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(SQUARE)
            )),
            &sources,
        )
        .expect("an SVG data URL is drawn");
        assert_eq!(pixel(&decoded(&svg).2, 64, 64), [0, 0, 0, 255]);
        let percent = icon_path(
            &serde_json::json!(format!(
                "data:image/svg+xml,{}",
                SQUARE.replace('#', "%23").replace(' ', "%20")
            )),
            &sources,
        );
        assert!(percent.is_some(), "a data URL need not be base64");
    }

    #[test]
    fn a_file_icon_is_the_themes_mime_icon_or_the_builtin_document() {
        let dir = tempfile::tempdir().unwrap();
        let icons = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(icons.path().join("blank-document.svg"), SQUARE).unwrap();
        std::fs::write(icons.path().join("folder.svg"), SQUARE).unwrap();
        let theme_png = dir.path().join("image-png.png");
        std::fs::write(&theme_png, tiny_png()).unwrap();
        let photo = dir.path().join("photo.png");
        std::fs::write(&photo, b"x").unwrap();
        let notes = dir.path().join("notes.qqqq");
        std::fs::write(&notes, b"x").unwrap();
        let asked = std::sync::Mutex::new(Vec::new());
        let find = |name: &str| -> Option<PathBuf> {
            asked.lock().unwrap().push(name.to_owned());
            (name == "image-png").then(|| theme_png.clone())
        };
        let no_fetch = |_: &str| -> Result<PathBuf, String> { panic!("nothing remote here") };
        let sources = Sources {
            assets: None,
            builtin_dir: Some(icons.path().to_path_buf()),
            fetch: &no_fetch,
            out_dir: out.path().to_path_buf(),
            find_icon: &find,
        };
        let icon = |path: &Path| {
            icon_path(
                &serde_json::json!({"fileIcon": path.to_string_lossy()}),
                &sources,
            )
        };

        let themed = icon(&photo).expect("the theme has image-png");
        assert!(themed.starts_with(out.path()));
        assert_eq!(pixel(&decoded(&themed).2, 63, 63), [255, 0, 0, 255]);
        assert_eq!(
            asked.lock().unwrap().first().map(String::as_str),
            Some("image-png")
        );

        let document = icon(&notes).expect("the builtin document");
        assert_eq!(pixel(&decoded(&document).2, 64, 64), [0, 0, 0, 255]);
        assert!(icon(dir.path()).is_some(), "a directory draws the folder");
        assert!(
            asked.lock().unwrap().contains(&"folder".to_owned()),
            "a directory asks the theme for its generic icon"
        );
    }
}
