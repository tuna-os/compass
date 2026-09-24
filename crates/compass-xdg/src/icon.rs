//! Icon theme lookup per the freedesktop [icon theme specification][spec].
//!
//! This module finds icon files given an icon name, searching the standard
//! XDG icon directories in precedence order, respecting theme inheritance and
//! the icon theme's `index.theme` file.
//!
//! [spec]: https://specifications.freedesktop.org/icon-theme-spec/latest/

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::xdg_dirs::icon_dirs;

/// An icon theme directory entry.
#[derive(Debug, Clone)]
pub struct IconThemeDir {
    /// The directory path containing icons.
    pub path: PathBuf,
    /// The size this directory is for, or None for scalable.
    pub size: Option<u32>,
    /// The icon type (fixed, scalable, threshold).
    pub icon_type: IconType,
    /// The context this directory is for, or None for all.
    pub context: Option<String>,
}

/// The type of icons in a directory, from the icon theme specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconType {
    /// Fixed size icons (e.g., 16x16, 24x24).
    Fixed,
    /// Scalable icons (SVG).
    Scalable,
    /// Threshold icons - used if no fixed size matches.
    Threshold,
}

impl IconType {
    /// Parse from the string in index.theme.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fixed" => Some(Self::Fixed),
            "scalable" => Some(Self::Scalable),
            "threshold" => Some(Self::Threshold),
            _ => None,
        }
    }
}

impl std::str::FromStr for IconType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str(s).ok_or(())
    }
}

/// A parsed icon theme index file.
#[derive(Debug, Clone, Default)]
pub struct IconTheme {
    /// The theme name.
    pub name: String,
    /// The theme comment/description.
    pub comment: Option<String>,
    /// Parent themes to inherit from.
    pub inherits: Vec<String>,
    /// Directories in this theme.
    pub directories: Vec<IconThemeDir>,
    /// The theme's example icon.
    pub example: Option<String>,
}

impl IconTheme {
    /// Parse an index.theme file.
    pub fn parse(data: &str) -> Option<Self> {
        let mut theme = Self::default();
        let mut current_section = String::new();

        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                current_section = section.to_owned();
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            match current_section.as_str() {
                "Icon Theme" => match key {
                    "Name" => theme.name = value.to_owned(),
                    "Comment" => theme.comment = Some(value.to_owned()),
                    "Inherits" => {
                        theme.inherits = value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToOwned::to_owned)
                            .collect();
                    }
                    "Example" => theme.example = Some(value.to_owned()),
                    _ => {}
                },
                _ => {
                    // Directory section
                    let _dir = IconThemeDir {
                        path: PathBuf::new(),
                        size: None,
                        icon_type: IconType::Fixed,
                        context: None,
                    };
                    // We'll parse directory sections differently - need to know the section name
                }
            }
        }

        // Second pass for directory sections
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                current_section = section.to_owned();
                continue;
            }

            if current_section != "Icon Theme" {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };

                // Find or create the directory entry for this section
                let mut found = false;
                for dir in &mut theme.directories {
                    if dir.path.as_os_str() == current_section.as_str() {
                        match key {
                            "Size" => dir.size = value.parse().ok(),
                            "Type" => {
                                dir.icon_type = IconType::from_str(value).unwrap_or(IconType::Fixed)
                            }
                            "Context" => dir.context = Some(value.to_owned()),
                            _ => {}
                        }
                        found = true;
                        break;
                    }
                }

                if !found {
                    let size = if key == "Size" {
                        value.parse().ok()
                    } else {
                        None
                    };
                    let icon_type = if key == "Type" {
                        IconType::from_str(value).unwrap_or(IconType::Fixed)
                    } else {
                        IconType::Fixed
                    };
                    let context = if key == "Context" {
                        Some(value.to_owned())
                    } else {
                        None
                    };

                    theme.directories.push(IconThemeDir {
                        path: PathBuf::from(current_section.clone()),
                        size,
                        icon_type,
                        context,
                    });
                }
            }
        }

        Some(theme)
    }

    /// Read and parse the index.theme file at the given path.
    pub fn from_file(path: &Path) -> Option<Self> {
        let data = std::fs::read_to_string(path).ok()?;
        Self::parse(&data)
    }
}

/// Find an icon file by name, searching through XDG icon directories.
///
/// Returns the absolute path to the icon file if found, preferring the
/// highest-resolution match according to the icon theme specification.
///
/// # Arguments
///
/// * `icon_name` - The icon name to search for (without extension).
/// * `theme_name` - The icon theme to search in. If None, uses the default
///   theme from `$XDG_CURRENT_DESKTOP` or falls back to "hicolor".
/// * `size` - Desired icon size in pixels. If None, returns the scalable version
///   if available, otherwise the largest fixed size.
/// * `scale` - Desired scale factor (e.g., 2.0 for @2x).
///
/// # Search order
///
/// 1. `$XDG_DATA_HOME/icons/<theme>/...`
/// 2. Each `$XDG_DATA_DIRS/icons/<theme>/...` in order
/// 3. If theme not found, try parent themes (from `Inherits`)
/// 4. Fall back to "hicolor" theme
/// 5. If still not found, try other themes in the same directories
pub fn find_icon(
    icon_name: &str,
    theme_name: Option<&str>,
    size: Option<u32>,
    scale: f32,
) -> Option<PathBuf> {
    let theme = theme_name.unwrap_or("hicolor");
    let search_dirs = icon_search_dirs(theme);
    let target_size = size.map(|s| (s as f32 * scale).round() as u32);
    find_icon_in_dirs(icon_name, theme, target_size, &search_dirs)
}

fn find_icon_in_dirs(
    icon_name: &str,
    theme: &str,
    target_size: Option<u32>,
    search_dirs: &[PathBuf],
) -> Option<PathBuf> {
    // Search in theme and its parents
    let mut visited = HashSet::new();
    let mut themes_to_search = vec!["hicolor".to_owned(), theme.to_owned()];

    while let Some(current_theme) = themes_to_search.pop() {
        if !visited.insert(current_theme.clone()) {
            continue;
        }

        let Some(metadata) = load_theme(search_dirs, &current_theme) else {
            continue;
        };
        if let Some(found) = find_icon_in_theme(
            search_dirs,
            &current_theme,
            &metadata,
            icon_name,
            target_size,
        ) {
            return Some(found);
        }

        // Add parent themes
        themes_to_search.extend(metadata.inherits.into_iter().rev());
    }

    // Fallback: search all themes in the search directories
    for base_dir in search_dirs {
        if let Ok(read_dir) = std::fs::read_dir(base_dir) {
            for entry in read_dir.flatten() {
                let theme_name = entry.file_name();
                let theme_name_str = theme_name.to_string_lossy();
                if !visited.insert(theme_name_str.to_string()) {
                    continue;
                }
                let Some(metadata) = load_theme(search_dirs, &theme_name_str) else {
                    continue;
                };
                if let Some(found) = find_icon_in_theme(
                    search_dirs,
                    &theme_name_str,
                    &metadata,
                    icon_name,
                    target_size,
                ) {
                    return Some(found);
                }
            }
        }
    }

    None
}

/// Get the icon search directories in precedence order.
fn icon_search_dirs(_theme: &str) -> Vec<PathBuf> {
    icon_dirs()
}

/// The first index supplies metadata for this theme across every base directory.
fn load_theme(search_dirs: &[PathBuf], theme: &str) -> Option<IconTheme> {
    search_dirs
        .iter()
        .find_map(|base| IconTheme::from_file(&base.join(theme).join("index.theme")))
}

fn find_icon_in_theme(
    search_dirs: &[PathBuf],
    theme_name: &str,
    theme: &IconTheme,
    icon_name: &str,
    target_size: Option<u32>,
) -> Option<PathBuf> {
    search_dirs.iter().find_map(|base| {
        find_icon_in_theme_dir(&base.join(theme_name), theme, icon_name, target_size)
    })
}

/// Search for an icon file within a theme directory.
fn find_icon_in_theme_dir(
    theme_dir: &Path,
    theme: &IconTheme,
    icon_name: &str,
    target_size: Option<u32>,
) -> Option<PathBuf> {
    // Find matching directories
    let mut candidates = Vec::new();

    for dir in &theme.directories {
        let dir_path = theme_dir.join(&dir.path);
        if !dir_path.exists() {
            continue;
        }

        // Check context match (we ignore context for now, could be improved)
        // Check if this directory can provide the target size
        let can_provide = match dir.icon_type {
            IconType::Fixed => {
                if let Some(dir_size) = dir.size {
                    target_size.is_none_or(|ts| ts <= dir_size)
                } else {
                    true
                }
            }
            IconType::Scalable => true,
            IconType::Threshold => {
                if let Some(dir_size) = dir.size {
                    target_size.is_none_or(|ts| ts >= dir_size)
                } else {
                    true
                }
            }
        };

        if !can_provide {
            continue;
        }

        // Search for the icon file in this directory
        let extensions = ["svg", "png", "xpm", "jpg", "jpeg"];
        for ext in &extensions {
            let icon_path = dir_path.join(format!("{icon_name}.{ext}"));
            if icon_path.exists() {
                candidates.push((icon_path, dir.icon_type, dir.size));
            }
        }
    }

    // Sort candidates by preference:
    // 1. Scalable (SVG) first
    // 2. Then fixed size >= target, smallest first
    // 3. Then threshold
    candidates.sort_by(|a, b| {
        use IconType::*;
        let (_, type_a, size_a) = a;
        let (_, type_b, size_b) = b;

        // Scalable wins
        match (type_a, type_b) {
            (Scalable, Scalable) => {}
            (Scalable, _) => return std::cmp::Ordering::Less,
            (_, Scalable) => return std::cmp::Ordering::Greater,
            _ => {}
        }

        // For fixed sizes, prefer exact match or smallest >= target
        if matches!((type_a, type_b), (Fixed, Fixed)) {
            match (size_a, size_b, target_size) {
                (Some(sa), Some(sb), Some(ts)) => {
                    let diff_a = sa.abs_diff(ts);
                    let diff_b = sb.abs_diff(ts);
                    diff_a.cmp(&diff_b).then(sa.cmp(sb))
                }
                (Some(_), None, _) => std::cmp::Ordering::Less,
                (None, Some(_), _) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        } else {
            std::cmp::Ordering::Equal
        }
    });

    candidates.first().map(|(p, _, _)| p.clone())
}

/// Get the default icon theme name from the environment.
pub fn default_theme() -> String {
    // Check $XDG_CURRENT_DESKTOP for known desktop-specific themes
    if let Ok(desktops) = std::env::var("XDG_CURRENT_DESKTOP") {
        for desktop in desktops.split(':') {
            match desktop.to_ascii_lowercase().as_str() {
                "gnome" | "gnome-classic" => return "Adwaita".to_owned(),
                "kde" => return "breeze".to_owned(),
                "xfce" => return "elementary-xfce".to_owned(),
                _ => {}
            }
        }
    }

    // Check $GTK_THEME
    if let Ok(gtk_theme) = std::env::var("GTK_THEME")
        && let Some(theme) = gtk_theme.split(':').next()
    {
        return theme.to_owned();
    }

    "hicolor".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_theme(base: &Path, name: &str, inherits: &str, directory: &str) {
        let root = base.join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("index.theme"),
            format!(
                "[Icon Theme]\nName={name}\nInherits={inherits}\nDirectories={directory}\n\
                 [{directory}]\nSize=512\nType=Fixed\nContext=Applications\n"
            ),
        )
        .unwrap();
    }

    fn write_icon(base: &Path, theme: &str, directory: &str, name: &str) -> PathBuf {
        let root = base.join(theme).join(directory);
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("{name}.png"));
        fs::write(&path, b"png").unwrap();
        path
    }

    #[test]
    fn local_overlay_uses_system_metadata_and_overrides_system_icon() {
        let root = tempdir().unwrap();
        let local = root.path().join("local");
        let system = root.path().join("system");
        write_theme(&system, "hicolor", "", "512x512/apps");
        let expected = write_icon(&local, "hicolor", "512x512/apps", "antigravity-ide");
        write_icon(&system, "hicolor", "512x512/apps", "antigravity-ide");
        assert!(!local.join("hicolor/index.theme").exists());
        assert_eq!(
            find_icon_in_dirs("antigravity-ide", "hicolor", Some(32), &[local, system]),
            Some(expected)
        );
    }

    #[test]
    fn first_index_controls_directories_and_inheritance_across_roots() {
        let root = tempdir().unwrap();
        let local = root.path().join("local");
        let system = root.path().join("system");
        write_theme(&local, "custom", "preferred", "512x512/apps");
        write_theme(&system, "custom", "other", "obsolete/apps");
        let expected = write_icon(&system, "custom", "512x512/apps", "app");
        let dirs = [local, system];
        assert_eq!(
            find_icon_in_dirs("app", "custom", Some(32), &dirs),
            Some(expected)
        );
        let theme = load_theme(&dirs, "custom").unwrap();
        assert_eq!(theme.inherits, ["preferred"]);
        assert_eq!(theme.directories.len(), 1);
        assert_eq!(theme.directories[0].path, PathBuf::from("512x512/apps"));
    }

    #[test]
    fn inherited_and_fallback_themes_find_metadata_free_overlays() {
        let root = tempdir().unwrap();
        let local = root.path().join("local");
        let system = root.path().join("system");
        write_theme(&system, "custom", "parent", "512x512/apps");
        write_theme(&system, "parent", "custom", "512x512/apps");
        write_theme(&system, "hicolor", "", "512x512/apps");
        let inherited = write_icon(&local, "parent", "512x512/apps", "inherited");
        let fallback = write_icon(&local, "hicolor", "512x512/apps", "fallback");
        let dirs = [local, system];
        assert_eq!(
            find_icon_in_dirs("inherited", "custom", Some(32), &dirs),
            Some(inherited)
        );
        assert_eq!(
            find_icon_in_dirs("fallback", "custom", Some(32), &dirs),
            Some(fallback)
        );
        assert_eq!(
            find_icon_in_dirs("missing", "custom", Some(32), &dirs),
            None
        );
    }

    #[test]
    fn nested_directory_metadata_belongs_to_one_entry() {
        let theme = IconTheme::parse(
            "[Icon Theme]\nName=Test\nDirectories=32x32/apps,scalable/apps\n\
             [32x32/apps]\nSize=32\nType=Fixed\nContext=Applications\n\
             [scalable/apps]\nSize=48\nType=Scalable\nContext=Applications\n",
        )
        .unwrap();
        assert_eq!(theme.directories.len(), 2);
        assert_eq!(theme.directories[0].path, PathBuf::from("32x32/apps"));
        assert_eq!(theme.directories[0].size, Some(32));
        assert_eq!(theme.directories[0].icon_type, IconType::Fixed);
        assert_eq!(
            theme.directories[0].context.as_deref(),
            Some("Applications")
        );
        assert_eq!(theme.directories[1].size, Some(48));
        assert_eq!(theme.directories[1].icon_type, IconType::Scalable);
        assert_eq!(
            theme.directories[1].context.as_deref(),
            Some("Applications")
        );
    }

    #[test]
    fn nested_fixed_directories_choose_the_closest_sufficient_size() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("index.theme"),
            "[Icon Theme]\nName=Test\nDirectories=256x256/apps,48x48/apps\n\
             [256x256/apps]\nSize=256\nType=Fixed\nContext=Applications\n\
             [48x48/apps]\nSize=48\nType=Fixed\nContext=Applications\n",
        )
        .unwrap();
        for size in [256, 48] {
            let icons = dir.path().join(format!("{size}x{size}/apps"));
            fs::create_dir_all(&icons).unwrap();
            fs::write(icons.join("app.png"), b"png").unwrap();
        }
        assert_eq!(
            find_icon_in_theme_dir(
                dir.path(),
                &IconTheme::from_file(&dir.path().join("index.theme")).unwrap(),
                "app",
                Some(32),
            ),
            Some(dir.path().join("48x48/apps/app.png"))
        );
    }

    #[test]
    fn icon_theme_parse() {
        let data = r#"
[Icon Theme]
Name=Test Theme
Comment=A test theme
Inherits=hicolor
Example=folder

[scalable]
Size=256
Type=Scalable
Context=places

[16x16]
Size=16
Type=Fixed
Context=places
"#;

        let theme = IconTheme::parse(data).unwrap();
        assert_eq!(theme.name, "Test Theme");
        assert_eq!(theme.comment, Some("A test theme".to_owned()));
        assert_eq!(theme.inherits, vec!["hicolor"]);
        assert_eq!(theme.example, Some("folder".to_owned()));
        assert_eq!(theme.directories.len(), 2);

        let scalable = theme
            .directories
            .iter()
            .find(|d| d.path.as_os_str() == "scalable")
            .unwrap();
        assert_eq!(scalable.icon_type, IconType::Scalable);
        assert_eq!(scalable.size, Some(256));
        assert_eq!(scalable.context, Some("places".to_owned()));

        let fixed = theme
            .directories
            .iter()
            .find(|d| d.path.as_os_str() == "16x16")
            .unwrap();
        assert_eq!(fixed.icon_type, IconType::Fixed);
        assert_eq!(fixed.size, Some(16));
        assert_eq!(fixed.context, Some("places".to_owned()));
    }

    #[test]
    fn find_icon_in_test_theme() {
        let dir = tempdir().unwrap();
        let theme_dir = dir.path().join("test-theme");
        fs::create_dir_all(theme_dir.join("scalable")).unwrap();
        fs::create_dir_all(theme_dir.join("16x16")).unwrap();

        fs::write(
            theme_dir.join("index.theme"),
            r#"
[Icon Theme]
Name=Test
Inherits=hicolor

[scalable]
Size=256
Type=Scalable

[16x16]
Size=16
Type=Fixed
"#,
        )
        .unwrap();

        fs::write(
            theme_dir.join("scalable").join("test-icon.svg"),
            "<svg></svg>",
        )
        .unwrap();
        fs::write(theme_dir.join("16x16").join("test-icon.png"), b"png").unwrap();

        let found = find_icon_in_theme_dir(
            &theme_dir,
            &IconTheme::from_file(&theme_dir.join("index.theme")).unwrap(),
            "test-icon",
            Some(16),
        );
        assert!(found.is_some());
        assert!(found.unwrap().to_string_lossy().contains("test-icon"));
    }
}
