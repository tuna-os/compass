//! Finding script commands on disk, and what a found one runs and shows.
//!
//! Ports `src/server/src/script/script-scanner.cpp`,
//! `script-command-file.cpp` and `script-metadata-store.cpp`. The parsing of a
//! script's header block already lives in [`crate::script_command`]; this
//! module is the layer around it — which files are even offered to the parser,
//! what argument vector a script is run with, which icon it shows, and the
//! record of an inline script's last line of output.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::image_url::{ColorLike, ImageUrl};
use crate::script_command::{OutputMode, ScriptCommand};

/// How deep the scan walks below a script directory.
///
/// The C++ compares `depth + 1 < MAX_DEPTH`, so a directory sitting at depth 4
/// is listed but not descended into.
pub const MAX_DEPTH: u8 = 5;

/// Extensions that are never a script, however executable the file looks.
pub const FORBIDDEN_EXTENSIONS: &[&str] = &["md", "svg", "txt"];

/// How much of a file is sniffed for a NUL byte before it is called binary.
pub const SNIFF_BYTES: usize = 8192;

/// A directory waiting to be listed, and the id prefix its entries inherit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedDirectory {
    /// Dotted id of the directory itself; empty for a root script directory.
    pub id: String,
    /// Where it is.
    pub path: PathBuf,
    /// How many directories below a root it sits.
    pub depth: u8,
}

/// The last component of a path, the way the C++ helper computes it.
///
/// `std::filesystem::path` has no filename when it ends in a separator, and
/// then the helper reaches for the parent — so `a/b/` is `b`, not the empty
/// string a naive `file_name()` would give.
pub fn last_path_component(path: &Path) -> String {
    if let Some(name) = path.file_name() {
        return name.to_string_lossy().into_owned();
    }
    let parent = path.parent().unwrap_or(path);
    match parent.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// Whether a directory entry is skipped outright, before it is even given an id.
///
/// A dotfile is hidden; `.template` is a Raycast convention for a script that
/// is meant to be copied rather than run, and the C++ looks for it *anywhere*
/// in the name rather than only as a suffix.
pub fn is_skipped_name(filename: &str) -> bool {
    filename.starts_with('.') || filename.contains(".template")
}

/// Whether a file's extension puts it out of the running.
pub fn has_forbidden_extension(path: &Path) -> bool {
    match path.extension() {
        Some(ext) => FORBIDDEN_EXTENSIONS.contains(&&*ext.to_string_lossy()),
        None => false,
    }
}

/// Whether the first [`SNIFF_BYTES`] of a file are free of NUL bytes.
///
/// This is the whole of the C++ test for "is this text?". An empty file passes,
/// and so does a binary whose first NUL happens to sit past the sniff window.
pub fn head_looks_like_text(head: &[u8]) -> bool {
    !head[..head.len().min(SNIFF_BYTES)].contains(&0)
}

/// What the scan decided about one directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryOutcome {
    /// Skipped by name, before an id was formed.
    Hidden,
    /// A directory to descend into, with the id its children inherit.
    Descend(ScannedDirectory),
    /// A directory too deep to descend into. It yields nothing.
    TooDeep,
    /// An id already claimed by an earlier file.
    Duplicate,
    /// A forbidden extension.
    Forbidden,
    /// Binary, by the NUL sniff.
    Binary,
    /// A candidate script, with the id it claims.
    Candidate(String),
}

/// One directory entry, as the scan sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    /// The entry's own name.
    pub name: String,
    /// Its full path.
    pub path: PathBuf,
    /// Whether it is a directory.
    pub is_dir: bool,
}

/// The id an entry claims: the directory's id, a dot, and the file name.
pub fn entry_id(dir_id: &str, filename: &str) -> String {
    if dir_id.is_empty() {
        filename.to_owned()
    } else {
        format!("{dir_id}.{filename}")
    }
}

/// Classify one entry against the scan's rules.
///
/// The order is the C++ order and it matters twice. A directory is descended
/// into *before* the duplicate check, so a directory never consumes an id and a
/// name shared with an already-seen file still contributes its children. And
/// the duplicate check comes *before* the extension and binary checks, so a
/// `notes.md` next to a script of the same id is rejected for being a
/// duplicate — but a first-seen `notes.md` is rejected for its extension
/// without claiming the id, leaving it free for a later file.
pub fn classify(
    dir: &ScannedDirectory,
    entry: &DirEntryInfo,
    ids_seen: &HashSet<String>,
    is_text: impl FnOnce(&Path) -> bool,
) -> EntryOutcome {
    if is_skipped_name(&entry.name) {
        return EntryOutcome::Hidden;
    }

    let id = entry_id(&dir.id, &entry.name);

    if entry.is_dir {
        if dir.depth + 1 < MAX_DEPTH {
            return EntryOutcome::Descend(ScannedDirectory {
                id,
                path: entry.path.clone(),
                depth: dir.depth + 1,
            });
        }
        return EntryOutcome::TooDeep;
    }

    if ids_seen.contains(&id) {
        return EntryOutcome::Duplicate;
    }
    if has_forbidden_extension(&entry.path) {
        return EntryOutcome::Forbidden;
    }
    if !is_text(&entry.path) {
        return EntryOutcome::Binary;
    }

    EntryOutcome::Candidate(id)
}

/// A script the scan found and parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCommandFile {
    /// Dotted id, derived from the path below its script directory.
    pub id: String,
    /// Where the file is.
    pub path: PathBuf,
    /// Its parsed header block.
    pub data: ScriptCommand,
}

/// The icon a script with no usable icon of its own shows.
pub fn default_icon() -> ImageUrl {
    ImageUrl::builtin("code").with_background_tint(ColorLike::Semantic("Accent".into()))
}

impl ScriptCommandFile {
    /// Parse a file that the scan has already accepted.
    pub fn parse(
        path: impl Into<PathBuf>,
        id: impl Into<String>,
        text: &str,
    ) -> Result<Self, String> {
        let path = path.into();
        let data = ScriptCommand::parse(text)?;
        Ok(Self {
            id: id.into(),
            path,
            data,
        })
    }

    /// The grouping name shown beside the command.
    ///
    /// An inline script shows its last line of output instead of a package
    /// name, and `"No data"` until it has run once. Everything else falls back
    /// to the name of the directory the script sits in.
    pub fn package_name(&self, last_run: Option<&str>) -> String {
        if self.data.mode == OutputMode::Inline {
            return match last_run {
                Some(line) => line.to_owned(),
                None => "No data".to_owned(),
            };
        }
        if let Some(name) = &self.data.package_name {
            return name.clone();
        }
        self.path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The argument vector the script is run with.
    ///
    /// `@vicinae.exec` wins outright; otherwise the interpreter is resolved
    /// from the shebang or the extension. The supplied values are zipped
    /// against the declared arguments, and a zip stops at the shorter of the
    /// two — so extra values are dropped rather than appended, and a short
    /// call passes fewer arguments rather than empty ones.
    pub fn command_line(&self, interpreter: &[String], args: &[String]) -> Vec<String> {
        let mut cmdline: Vec<String> = if self.data.exec.is_empty() {
            interpreter.to_vec()
        } else {
            self.data.exec.clone()
        };

        cmdline.push(self.path.to_string_lossy().into_owned());

        for (arg, value) in self.data.arguments.iter().zip(args.iter()) {
            if arg.percent_encoded {
                cmdline.push(percent_encode(value));
            } else {
                cmdline.push(value.clone());
            }
        }

        cmdline
    }

    /// The icon the script shows.
    ///
    /// The chain is emoji, then a path that exists as given, then the same path
    /// read relative to the script's own directory, then an `https` URL, then
    /// the default. A `http` URL is *not* accepted: the C++ tests the scheme
    /// for `https` exactly.
    pub fn icon(
        &self,
        is_emoji: impl Fn(&str) -> bool,
        is_file: impl Fn(&Path) -> bool,
    ) -> ImageUrl {
        let Some(icon) = &self.data.icon else {
            return default_icon();
        };

        if is_emoji(icon) {
            return ImageUrl::new(crate::image_url::ImageUrlType::Emoji, icon.clone());
        }

        let direct = Path::new(icon);
        if is_file(direct) {
            return ImageUrl::local(icon.clone());
        }

        if let Some(parent) = self.path.parent() {
            let relative = parent.join(icon);
            if is_file(&relative) {
                return ImageUrl::local(relative.to_string_lossy().into_owned());
            }
        }

        if url_scheme(icon).as_deref() == Some("https") {
            return ImageUrl::http(icon.clone());
        }

        default_icon()
    }
}

/// The scheme of a URL, lowercased, or `None` when the text does not open with
/// one.
///
/// `QUrl` lowercases the scheme it parses, so `HTTPS://example.com` is an
/// `https` URL and reaches the remote-image branch. A scheme must start with a
/// letter and may then hold letters, digits, `+`, `-` and `.`.
pub fn url_scheme(text: &str) -> Option<String> {
    let (scheme, _) = text.split_once(':')?;
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// Percent-encode one argument value the way `QUrl::toPercentEncoding` does:
/// everything but the unreserved set, and no exceptions passed in.
pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let b = *byte;
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One script's record of its last inline run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMetadata {
    /// Seconds since the epoch.
    pub last_run_at: i64,
    /// The output line, base64-encoded.
    ///
    /// The C++ comment says why: the output is arbitrary bytes from someone
    /// else's script, and encoding it keeps the metadata file valid JSON
    /// whatever the script printed.
    pub output: String,
}

/// The on-disk shape of the metadata file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataData {
    /// Keyed by script id.
    #[serde(default)]
    pub inline_runs: HashMap<String, RunMetadata>,
}

/// The record of what inline scripts last printed.
#[derive(Debug, Clone, Default)]
pub struct ScriptMetadataStore {
    data: MetadataData,
}

impl ScriptMetadataStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a store from the file's text.
    ///
    /// A file that will not parse is not an error the caller has to handle: the
    /// C++ logs and carries on with an empty store, because a corrupt cache of
    /// last-run output is not a reason to refuse to list anyone's scripts.
    pub fn from_json(text: &str) -> Self {
        Self {
            data: serde_json::from_str(text).unwrap_or_default(),
        }
    }

    /// What the store would write to disk.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.data).expect("metadata is always serializable")
    }

    /// Record a script's newest output line.
    pub fn save_run(&mut self, script_id: &str, line: &str, now: i64) {
        self.data.inline_runs.insert(
            script_id.to_owned(),
            RunMetadata {
                last_run_at: now,
                output: base64::engine::general_purpose::STANDARD.encode(line),
            },
        );
    }

    /// The last line a script printed, decoded.
    pub fn last_run_data(&self, id: &str) -> Option<String> {
        let entry = self.data.inline_runs.get(id)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&entry.output)
            .ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// When a script last ran.
    pub fn last_run_at(&self, id: &str) -> Option<i64> {
        self.data.inline_runs.get(id).map(|e| e.last_run_at)
    }
}

/// The directories a scan walks: the caller's custom paths first, then the
/// defaults. Order decides which of two same-id scripts wins, and a custom
/// directory is meant to shadow a packaged one.
pub fn scan_directories(custom: &[PathBuf], defaults: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = custom.to_vec();
    out.extend_from_slice(defaults);
    out
}
