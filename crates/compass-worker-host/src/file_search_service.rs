//! The `FileSearch` half of the extension API.
//!
//! Ports `ExtFileSearchService`
//! (`src/server/src/extension/api/file-search-service.hpp`): one method, which
//! is a thin adapter between the wire types the IDL declares and the file
//! indexer's own. The indexer is behind [`FileIndexer`] here for the same
//! reason it is behind `AbstractFileIndexer` there — the service does not care
//! which backend answers, and a test can answer without one.
//!
//! # The adapter does not second-guess the indexer
//!
//! The C++ hands `limit` and `category` to `queryAsync` and returns whatever
//! comes back, mapped. It does not truncate, re-sort or re-filter the results.
//! Neither does this: if an indexer returns more rows than `limit`, that is the
//! indexer's bug, and hiding it here would make it harder to find.

use std::path::PathBuf;

use compass_core::file_category::FileCategory;

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &["FileSearch/search"];

/// The C++ `IndexerQueryParams` default, for a caller that builds one by hand.
pub const DEFAULT_LIMIT: i64 = 100;

/// One row from the index.
///
/// Mirrors `IndexerFileResult`. `rank` is carried even though `FileInfo` has no
/// field for it: it is what the indexer ordered by, and dropping it at the
/// boundary would make an ordering bug unattributable.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexerFileResult {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// The indexer's own relevance score.
    pub rank: f64,
    /// What kind of file it is.
    pub category: FileCategory,
    /// Its MIME type, when the indexer knows one.
    pub mime_type: Option<String>,
}

/// What the service asks the indexer for. Mirrors `IndexerQueryParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryParams {
    /// At most this many rows.
    pub limit: i64,
    /// Only rows of this category, when set.
    pub category: Option<FileCategory>,
}

impl Default for QueryParams {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            category: None,
        }
    }
}

/// A file index the extension API can query.
///
/// The Rust counterpart of `AbstractFileIndexer`, narrowed to what the
/// extension API uses: scanning, rebuilding and the preference plumbing belong
/// to the indexer's own owner, not to an extension.
pub trait FileIndexer {
    /// Rows matching `query`, subject to `params`.
    fn query(&self, query: &str, params: &QueryParams) -> Vec<IndexerFileResult>;

    /// Whether the index is usable at all.
    ///
    /// The C++ reads this once per command launch, to set
    /// `capabilities.fileSearch`; an extension that asks anyway still gets an
    /// answer, which is why this does not gate [`FileSearchService::handle`].
    fn is_available(&self) -> bool {
        true
    }
}

/// Serves `FileSearch` from one index.
#[derive(Debug)]
pub struct FileSearchService<I> {
    indexer: I,
}

impl<I: FileIndexer> FileSearchService<I> {
    /// Serves `indexer`.
    pub const fn new(indexer: I) -> Self {
        Self { indexer }
    }

    /// The index this serves.
    pub const fn indexer(&self) -> &I {
        &self.indexer
    }

    /// Answers `call`, or `None` if it is not a `FileSearch` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let query = call
            .params
            .get("q")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let opts = call.params.get("opts");

        // `IndexerQueryParams::limit` defaults to 100, but the C++ never reaches
        // that default: it always assigns `opts.limit`, and glaze
        // value-initialises a `uint` field the JSON omits. A caller that leaves
        // `limit` out therefore asks for zero rows, not for a hundred.
        let limit = opts
            .and_then(|opts| opts.get("limit"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let category = opts
            .and_then(|opts| opts.get("filters"))
            .and_then(|filters| filters.get("category"))
            .filter(|category| !category.is_null());
        let category = match category {
            None => None,
            Some(value) => match value.as_str().and_then(category_from_wire) {
                Some(category) => Some(category),
                None => {
                    return Some(tsapi::reply_error(
                        id,
                        &format!("no such category: {value}"),
                    ));
                }
            },
        };

        let results = self.indexer.query(query, &QueryParams { limit, category });

        let files: Vec<serde_json::Value> = results.iter().map(file_info).collect();
        Some(tsapi::reply(id, serde_json::Value::Array(files)))
    }
}

/// One `FileInfo`, as the IDL declares it.
///
/// `mimeType` is optional there, so an absent one is an absent key rather than
/// a null — the same shape the generated TypeScript's `mimeType?: string`
/// expects.
fn file_info(result: &IndexerFileResult) -> serde_json::Value {
    let mut info = serde_json::Map::new();
    info.insert(
        "path".to_owned(),
        serde_json::Value::String(result.path.to_string_lossy().into_owned()),
    );
    info.insert(
        "category".to_owned(),
        serde_json::Value::String(category_to_wire(result.category).to_owned()),
    );
    if let Some(mime) = &result.mime_type {
        info.insert(
            "mimeType".to_owned(),
            serde_json::Value::String(mime.clone()),
        );
    }
    serde_json::Value::Object(info)
}

/// `tsapi::FileSearchCategory` -> `vicinae::FileCategory`.
///
/// The two enums list the same eight names in the same order, and the C++ maps
/// them with a pair of switches that the compiler checks for exhaustiveness.
/// Matching on the name keeps that property here: a name the IDL grows and this
/// does not becomes an error reply, not a silently wrong category.
fn category_from_wire(name: &str) -> Option<FileCategory> {
    Some(match name {
        "Other" => FileCategory::Other,
        "Directory" => FileCategory::Directory,
        "Image" => FileCategory::Image,
        "Video" => FileCategory::Video,
        "Audio" => FileCategory::Audio,
        "Document" => FileCategory::Document,
        "Archive" => FileCategory::Archive,
        "Application" => FileCategory::Application,
        _ => return None,
    })
}

/// `vicinae::FileCategory` -> `tsapi::FileSearchCategory`.
const fn category_to_wire(category: FileCategory) -> &'static str {
    match category {
        FileCategory::Other => "Other",
        FileCategory::Directory => "Directory",
        FileCategory::Image => "Image",
        FileCategory::Video => "Video",
        FileCategory::Audio => "Audio",
        FileCategory::Document => "Document",
        FileCategory::Archive => "Archive",
        FileCategory::Application => "Application",
    }
}

impl<I: FileIndexer> tsapi::Service for FileSearchService<I> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}
