//! What the file indexer refuses to look at.
//!
//! A port of `EntryFilter` and `GitIgnoreReader`
//! (`src/file-indexer/src/entry-filter.cpp`).
//!
//! # Every rule here exists because something once made the index useless
//!
//! An indexer that walks `/proc` never finishes. One that walks
//! `~/.cargo/registry` fills the index with thousands of vendored source files
//! nobody searches for, and the one file they *did* want is ranked below them.
//! So this is a long list of specific names rather than a general rule, and it
//! is worth porting exactly: a name dropped from the list is not a bug anyone
//! sees as a bug, it is a search that quietly stops being useful.
//!
//! The rules run in the C++'s order, and the order is load-bearing — the
//! cheap string checks come before the ones that stat, and `isIgnored`, which
//! walks up the tree reading `.gitignore` files, comes last.

use std::path::{Component, Path, PathBuf};

/// The absolute paths never indexed, from `excludedPaths()`.
///
/// These are the ones that are not under a home directory: kernel and runtime
/// filesystems where walking is either endless or pointless.
pub const EXCLUDED_ABSOLUTE_PATHS: &[&str] =
    &["/sys", "/run", "/proc", "/tmp", "/var/tmp", "/efi", "/dev"];

/// The paths never indexed, relative to the home directory.
///
/// Package caches, language toolchains, container and VM images: places with
/// enormous file counts and nothing a person searches for by name.
pub const EXCLUDED_HOME_RELATIVE_PATHS: &[&str] = &[
    ".local/share/Trash",
    ".conda",
    "anaconda3",
    "miniconda3",
    "miniforge3",
    ".cargo/registry",
    ".cargo/git",
    ".rustup",
    "go/pkg",
    ".go/pkg",
    ".bun/install",
    ".npm",
    ".nvm",
    ".pnpm-store",
    ".yarn",
    ".m2/repository",
    ".ivy2",
    ".wine",
    ".docker",
    ".android",
    ".local/share/flatpak/app",
    ".local/share/flatpak/repo/objects",
    ".local/share/flatpak/runtime",
    ".local/share/pnpm/store",
    ".local/share/Steam/config/htmlcache",
    ".local/share/vicinae/clipboard-data",
    "snap",
];

/// Directory and file names never indexed, wherever they appear.
pub const EXCLUDED_FILENAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".cache",
    ".clangd",
    ".ccache",
    ".gradle",
    ".terraform",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "node_modules",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".angular",
    ".svelte-kit",
    "GPUCache",
    "Code Cache",
    "CacheStorage",
    "Service Worker",
    "blob_storage",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "__MACOSX",
    ".Trash",
    "lost+found",
    ".snapshots",
];

/// The suffix that marks a directory as a cache, by convention.
pub const CACHEDIR_TAG_SUFFIX: &str = "/CACHEDIR.TAG";

/// The first line such a file must start with, from the CACHEDIR.TAG spec.
///
/// Checked byte for byte: a file merely *named* `CACHEDIR.TAG` is not one, and
/// treating it as one would drop a directory somebody wanted indexed.
pub const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// The suffix Spotlight uses to mean "do not index", honoured by many apps.
pub const NOINDEX_SUFFIX: &str = ".noindex";

/// The absolute excluded paths, with the home-relative ones resolved under
/// `home`.
#[must_use]
pub fn excluded_paths(home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = EXCLUDED_ABSOLUTE_PATHS.iter().map(PathBuf::from).collect();
    if let Some(home) = home {
        paths.extend(
            EXCLUDED_HOME_RELATIVE_PATHS
                .iter()
                .map(|relative| home.join(relative)),
        );
    }
    paths
}

/// Whether any component of `path` starts with a dot.
///
/// Not just the last one: a file inside `~/.config` is hidden even though its
/// own name is not, which is what makes "ignore hidden paths" skip whole trees
/// rather than individual files.
#[must_use]
pub fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    })
}

/// The last component of `path`, as `basenameView` computes it.
///
/// A string operation, not a path one: it takes everything after the last `/`,
/// so a trailing slash yields an empty name rather than the directory's.
#[must_use]
pub fn basename(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// Whether `path` has `component` as one of its components.
fn path_contains(path: &Path, component: &str) -> bool {
    path.components()
        .any(|part| part.as_os_str() == std::ffi::OsStr::new(component))
}

/// Whether `second` appears as a component somewhere after `first`.
fn path_contains_after(path: &Path, first: &str, second: &str) -> bool {
    let mut seen_first = false;
    for part in path.components() {
        let name = part.as_os_str();
        if seen_first && name == std::ffi::OsStr::new(second) {
            return true;
        }
        if name == std::ffi::OsStr::new(first) {
            seen_first = true;
        }
    }
    false
}

/// Whether `path` is a directory generated by a machine and read by one.
///
/// Shader caches, crash dumps, browser storage. Two of these are conditional
/// rather than by name — `Cache_Data` only inside a `Cache`, and `cache` only
/// inside a `Shared Dictionary` — because those names are ordinary enough that
/// excluding them everywhere would drop real directories.
#[must_use]
pub fn is_machine_trash_directory(path: &Path) -> bool {
    let filename = basename(&path.to_string_lossy()).to_owned();

    match filename.as_str() {
        "Cache_Data" => path_contains(path, "Cache"),
        "GPUCache" | "Code Cache" | "CacheStorage" | "Service Worker" | "blob_storage"
        | "DawnGraphiteCache" | "DawnWebGPUCache" | "mesa_shader_cache" | "shader_cache"
        | "ShaderCache" | "GrShaderCache" | "Crashpad" | "crashpad" => true,
        "cache" => {
            path_contains(path, "Shared Dictionary") || path_contains_after(path, ".var", "cache")
        }
        _ => path_contains_after(path, ".var", "cache"),
    }
}

/// Whether `contents` begins with the CACHEDIR.TAG signature.
#[must_use]
pub fn has_cachedir_tag_signature(contents: &[u8]) -> bool {
    contents.starts_with(CACHEDIR_TAG_SIGNATURE.as_bytes())
}

/// Whether `path` names a CACHEDIR.TAG whose contents say so.
#[must_use]
pub fn is_cachedir_tag(path: &Path, contents: &[u8]) -> bool {
    path.to_string_lossy().ends_with(CACHEDIR_TAG_SUFFIX) && has_cachedir_tag_signature(contents)
}

/// The patterns of one ignore file.
#[derive(Debug, Clone, Default)]
pub struct GitIgnoreReader {
    /// Every line, verbatim — including blanks and comments.
    ///
    /// The C++ reads the file straight into a vector with no filtering, so a
    /// `#comment` line is matched as a glob. It never matches anything real,
    /// which is why nobody has noticed; reproduced so the two engines ignore
    /// the same files.
    patterns: Vec<String>,
}

impl GitIgnoreReader {
    /// Read patterns from an ignore file's text.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            patterns: text.lines().map(str::to_owned).collect(),
        }
    }

    /// The patterns, in file order.
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Whether any pattern matches `path`'s last component.
    ///
    /// # This is not gitignore
    ///
    /// Only the *filename* is matched, and a leading `/` is stripped rather
    /// than anchoring anything — the C++ calls this "a hack" in a comment and
    /// says directory separators are not handled properly. So `build/*.o`
    /// never matches, and `/target` matches a `target` anywhere in the tree,
    /// not just beside the ignore file. Reproduced, because narrowing it would
    /// start indexing directories people's `.gitignore` files currently keep
    /// out, and widening it would drop files they expect to find.
    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        let filename = last_path_component(path);
        self.patterns.iter().any(|pattern| {
            let processed = pattern.strip_prefix('/').unwrap_or(pattern);
            glob_match(processed, &filename)
        })
    }
}

/// The last component, as `getLastPathComponent` computes it.
///
/// A path ending in a separator has no filename, so its parent's name is used
/// — which is how a directory handed in with a trailing slash still matches by
/// its own name.
#[must_use]
pub fn last_path_component(path: &Path) -> String {
    path.file_name().map_or_else(
        || {
            path.parent().and_then(Path::file_name).map_or_else(
                || path.to_string_lossy().into_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
        },
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `fnmatch` with `FNM_PATHNAME`: `*` and `?` do not cross a `/`.
///
/// Only the subset the ignore patterns can reach is implemented — `*`, `?` and
/// `[...]` — because the name being matched is a single path component and so
/// never contains a separator anyway.
#[must_use]
pub fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), name.as_bytes())
}

/// The recursive half of [`glob_match`].
fn glob_match_bytes(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some(b'*') => {
            // `*` never matches a separator under FNM_PATHNAME.
            (0..=name.len())
                .take_while(|taken| name[..*taken].iter().all(|byte| *byte != b'/'))
                .any(|taken| glob_match_bytes(&pattern[1..], &name[taken..]))
        }
        Some(b'?') => {
            !name.is_empty() && name[0] != b'/' && glob_match_bytes(&pattern[1..], &name[1..])
        }
        Some(b'[') => match_bracket(pattern, name),
        Some(b'\\') if pattern.len() > 1 => {
            !name.is_empty() && name[0] == pattern[1] && glob_match_bytes(&pattern[2..], &name[1..])
        }
        Some(literal) => {
            !name.is_empty() && name[0] == *literal && glob_match_bytes(&pattern[1..], &name[1..])
        }
    }
}

/// Match a `[...]` class at the head of `pattern`.
fn match_bracket(pattern: &[u8], name: &[u8]) -> bool {
    let Some(close) = pattern
        .iter()
        .position(|byte| *byte == b']')
        .filter(|pos| *pos > 1)
    else {
        // An unterminated `[` is a literal `[`, as fnmatch treats it.
        return !name.is_empty() && name[0] == b'[' && glob_match_bytes(&pattern[1..], &name[1..]);
    };
    if name.is_empty() {
        return false;
    }

    let mut class = &pattern[1..close];
    let negated = matches!(class.first(), Some(b'!' | b'^'));
    if negated {
        class = &class[1..];
    }

    let mut matched = false;
    let mut index = 0;
    while index < class.len() {
        if index + 2 < class.len() && class[index + 1] == b'-' {
            if (class[index]..=class[index + 2]).contains(&name[0]) {
                matched = true;
            }
            index += 3;
        } else {
            if class[index] == name[0] {
                matched = true;
            }
            index += 1;
        }
    }

    (matched != negated) && glob_match_bytes(&pattern[close + 1..], &name[1..])
}

/// What the filter needs to know about an entry without walking the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Where it is.
    pub path: &'a Path,
    /// Whether it is a symlink.
    pub is_symlink: bool,
    /// Whether it is a directory.
    pub is_directory: bool,
}

/// Decides which directory entries the indexer walks into.
#[derive(Debug, Clone, Default)]
pub struct EntryFilter {
    /// Extra paths to exclude, from the configuration.
    excluded_paths: Vec<PathBuf>,
    /// Extra filenames to exclude.
    excluded_filenames: Vec<String>,
    /// The ignore files to honour, such as `.gitignore`.
    ignore_files: Vec<String>,
    /// Whether dotted paths are skipped.
    ignore_hidden_paths: bool,
    /// The built-in excluded paths, resolved against a home directory.
    builtin_excluded: Vec<PathBuf>,
}

impl EntryFilter {
    /// A filter whose built-in exclusions resolve under `home`.
    #[must_use]
    pub fn new(home: Option<&Path>) -> Self {
        Self {
            builtin_excluded: excluded_paths(home),
            ..Self::default()
        }
    }

    /// Set the configured excluded paths.
    pub fn set_excluded_paths(&mut self, paths: Vec<PathBuf>) {
        self.excluded_paths = paths;
    }

    /// Set the configured excluded filenames.
    pub fn set_excluded_filenames(&mut self, filenames: Vec<String>) {
        self.excluded_filenames = filenames;
    }

    /// Set which ignore files to read.
    pub fn set_ignore_files(&mut self, files: Vec<String>) {
        self.ignore_files = files;
    }

    /// Set whether dotted paths are skipped.
    pub fn set_ignore_hidden_paths(&mut self, value: bool) {
        self.ignore_hidden_paths = value;
    }

    /// The ignore files being honoured.
    #[must_use]
    pub fn ignore_files(&self) -> &[String] {
        &self.ignore_files
    }

    /// Whether `path` is one of the configured excluded paths.
    #[must_use]
    pub fn is_excluded_path(&self, path: &Path) -> bool {
        self.excluded_paths.iter().any(|excluded| excluded == path)
    }

    /// Whether the indexer should walk into `entry`.
    ///
    /// `read_ignore_file` is asked for the contents of an ignore file at a
    /// given path, or `None` if there is none there. It is consulted last and
    /// only when ignore files are configured, because it is the only rule that
    /// touches the disk.
    pub fn should_visit(
        &self,
        entry: Entry<'_>,
        read_ignore_file: impl FnMut(&Path) -> Option<String>,
    ) -> bool {
        if entry.is_symlink {
            return false;
        }

        let path = entry.path;

        if self.ignore_hidden_paths && is_hidden_path(path) {
            return false;
        }
        if self
            .builtin_excluded
            .iter()
            .any(|excluded| excluded == path)
        {
            return false;
        }

        let lossy = path.to_string_lossy();
        let filename = basename(&lossy);

        if EXCLUDED_FILENAMES.contains(&filename) || filename.ends_with(NOINDEX_SUFFIX) {
            return false;
        }
        if self
            .excluded_filenames
            .iter()
            .any(|excluded| excluded == filename)
        {
            return false;
        }
        if entry.is_directory && is_machine_trash_directory(path) {
            return false;
        }
        if self.is_ignored(path, read_ignore_file) {
            return false;
        }
        if self.is_excluded_path(path) {
            return false;
        }

        true
    }

    /// Whether an ignore file anywhere above `path` matches it.
    ///
    /// Walks upward from the containing directory to the root. Every level is
    /// consulted, so a `.gitignore` at the top of a repository applies to a
    /// file nested many directories below it.
    pub fn is_ignored(
        &self,
        path: &Path,
        mut read_ignore_file: impl FnMut(&Path) -> Option<String>,
    ) -> bool {
        // No early return for an empty `ignore_files`: the C++ has one, but
        // the loop below already does nothing without any, and a guard whose
        // removal changes no behaviour is a guard no test can hold.
        let mut directory = path.parent();
        while let Some(current) = directory {
            for name in &self.ignore_files {
                let ignore_path = current.join(name);
                let Some(text) = read_ignore_file(&ignore_path) else {
                    continue;
                };
                if GitIgnoreReader::from_text(&text).matches(path) {
                    return true;
                }
            }
            if current == Path::new("/") || current.as_os_str().is_empty() {
                break;
            }
            directory = current.parent();
        }

        false
    }
}
