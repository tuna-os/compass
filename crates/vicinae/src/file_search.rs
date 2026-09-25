//! Search Files, the engine side.
//!
//! Ports what `SearchFilesViewHost` asks of the files service: recently
//! accessed files from `recently-used.xbel` for the empty query, the path
//! itself when the query names one that exists, and the file index otherwise.
//! The index is the `vicinae-file-indexer` helper, supervised by
//! [`IndexerClient`] and configured from the file extension's preferences,
//! the way `FileExtension::initialized` hands them to the files service.
//!
//! The view's debounce and stale-result checks live with the launcher, which
//! owns the timing; what is decided here is only what a query answers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_core::file_category::{FileCategory, file_category};
use compass_core::file_search::{
    DIRECT_PATH_HEADING, INDEXED_QUERY_LIMIT, IndexingSettings, QueryPlan, RECENT_FILES_LIMIT,
    RECENT_HEADING, category_key, is_explicit_path_query, plan_query, recent_files_usable,
    results_heading,
};
use compass_db::query_engine::IndexedFileCategory;
use compass_ipc::FileHit;

use crate::indexer_client::IndexerClient;
use crate::indexer_service::WireCategory;

/// The sentence an index search answers while the indexer is not running.
pub const INDEXER_UNAVAILABLE: &str = "file search is unavailable: the file indexer is not \
                                       running (it is turned off, or not installed)";

/// A heading and the files under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResults {
    /// What the list is.
    pub heading: &'static str,
    /// The files, in presentation order.
    pub files: Vec<FileHit>,
}

/// The engine's file search: the indexer it supervises, if it runs one.
#[derive(Debug, Default)]
pub struct FileSearch {
    indexer: Option<Arc<IndexerClient>>,
}

impl FileSearch {
    /// No indexer: direct paths and recent files still answer.
    #[must_use]
    pub fn without_indexer() -> Self {
        Self::default()
    }

    /// Starts the indexer helper configured from `settings`, unless indexing
    /// is turned off — then nothing runs, as `stopProcess` leaves it.
    #[must_use]
    pub fn start(settings: IndexingSettings) -> Self {
        if !settings.enabled {
            tracing::info!("file indexing is turned off; file search answers no index queries");
            return Self::without_indexer();
        }
        let client = IndexerClient::new();
        client.configure(settings.paths, settings.excluded_paths);
        client.start();
        Self {
            indexer: Some(client),
        }
    }

    /// Answers a Search Files query. Blocking: it reads the recent-files list
    /// and waits for the indexer.
    ///
    /// # Errors
    ///
    /// [`INDEXER_UNAVAILABLE`] when the query needs the index and the
    /// indexer is not running.
    pub fn search(
        &self,
        query: &str,
        category: Option<FileCategory>,
    ) -> Result<FileResults, &'static str> {
        let trimmed = query.trim();
        let home = compass_core::xdg_dirs::home_dir()
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_default();
        let expanded = compass_core::create_extension::expand_path(trimmed, &home);
        let is_path = is_explicit_path_query(trimmed);
        let target = Path::new(&expanded);
        let exists = is_path && target.exists();
        let category_of_path = exists.then(|| file_category(target, target.is_dir()));
        match plan_query(
            query,
            &expanded,
            exists,
            is_path,
            category_of_path,
            category,
        ) {
            QueryPlan::EmptyQuery => {
                let recent = recent_files(category);
                if recent_files_usable(recent.len()) {
                    return Ok(FileResults {
                        heading: RECENT_HEADING,
                        files: recent.iter().map(|path| hit(path, None)).collect(),
                    });
                }
                self.indexed("", category)
            }
            QueryPlan::DirectPath(path) => Ok(FileResults {
                heading: DIRECT_PATH_HEADING,
                files: vec![hit(Path::new(&path), category_of_path)],
            }),
            QueryPlan::DirectPathFiltered => Ok(FileResults {
                heading: DIRECT_PATH_HEADING,
                files: Vec::new(),
            }),
            QueryPlan::Search => self.indexed(query, category),
        }
    }

    /// Whether the index answers queries: the indexer is running.
    #[must_use]
    pub fn index_available(&self) -> bool {
        self.indexer
            .as_ref()
            .is_some_and(|indexer| indexer.is_running())
    }

    /// The index's own rows for `query`, as `FileSearch/search` hands them
    /// to an extension: no recent files, no direct path, no heading. Empty
    /// while the indexer is not running, as the C++ `queryAsync` settles.
    #[must_use]
    pub fn index_query(
        &self,
        query: &str,
        limit: i32,
        category: Option<FileCategory>,
    ) -> Vec<(PathBuf, f64, FileCategory, Option<String>)> {
        let Some(indexer) = self.indexer.as_ref().filter(|indexer| indexer.is_running()) else {
            return Vec::new();
        };
        indexer
            .query(query, limit, category.map(wire_category))
            .into_iter()
            .map(|result| {
                (
                    result.path,
                    result.rank,
                    from_indexed(result.category),
                    result.mime_type,
                )
            })
            .collect()
    }

    /// Asks the index, under the heading its query earns.
    fn indexed(
        &self,
        query: &str,
        category: Option<FileCategory>,
    ) -> Result<FileResults, &'static str> {
        let indexer = self
            .indexer
            .as_ref()
            .filter(|indexer| indexer.is_running())
            .ok_or(INDEXER_UNAVAILABLE)?;
        let files = indexer
            .query(query, INDEXED_QUERY_LIMIT, category.map(wire_category))
            .into_iter()
            .map(|result| hit(&result.path, Some(from_indexed(result.category))))
            .collect();
        Ok(FileResults {
            heading: results_heading(query),
            files,
        })
    }
}

/// The recently used files, newest first, as `XbelRecentFilesProvider`
/// lists them: private bookmarks and missing files skipped, the category
/// filter applied, at most [`RECENT_FILES_LIMIT`].
#[must_use]
pub fn recent_files(category: Option<FileCategory>) -> Vec<PathBuf> {
    let Some(xbel) = compass_xdg::bookmarks::recently_used_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&xbel) else {
        return Vec::new();
    };
    let bookmarks = match compass_xdg::bookmarks::parse(&text) {
        Ok(bookmarks) => bookmarks,
        Err(error) => {
            tracing::warn!(%error, path = %xbel.display(), "unreadable recent files list");
            return Vec::new();
        }
    };
    compass_xdg::bookmarks::recent_paths(
        &bookmarks,
        RECENT_FILES_LIMIT,
        &|path: &Path| path.exists(),
        &|path: &Path| category.is_none_or(|wanted| file_category(path, path.is_dir()) == wanted),
    )
}

/// One row: the path, its last component, and its category — the one the
/// index recorded when it has one, else read from the path.
pub fn hit(path: &Path, category: Option<FileCategory>) -> FileHit {
    let category = category.unwrap_or_else(|| file_category(path, path.is_dir()));
    FileHit {
        path: path.to_string_lossy().into_owned(),
        name: path.file_name().map_or_else(
            || path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
        category: category_key(category).to_owned(),
    }
}

fn wire_category(category: FileCategory) -> WireCategory {
    match category {
        FileCategory::Other => WireCategory::Other,
        FileCategory::Directory => WireCategory::Directory,
        FileCategory::Image => WireCategory::Image,
        FileCategory::Video => WireCategory::Video,
        FileCategory::Audio => WireCategory::Audio,
        FileCategory::Document => WireCategory::Document,
        FileCategory::Archive => WireCategory::Archive,
        FileCategory::Application => WireCategory::Application,
    }
}

fn from_indexed(category: IndexedFileCategory) -> FileCategory {
    match category {
        IndexedFileCategory::Other => FileCategory::Other,
        IndexedFileCategory::Directory => FileCategory::Directory,
        IndexedFileCategory::Image => FileCategory::Image,
        IndexedFileCategory::Video => FileCategory::Video,
        IndexedFileCategory::Audio => FileCategory::Audio,
        IndexedFileCategory::Document => FileCategory::Document,
        IndexedFileCategory::Archive => FileCategory::Archive,
        IndexedFileCategory::Application => FileCategory::Application,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_path_answers_itself_and_the_filter_can_empty_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "x").expect("write");
        let search = FileSearch::without_indexer();
        let query = file.to_string_lossy().into_owned();

        let found = search.search(&query, None).expect("a direct path");
        assert_eq!(found.heading, DIRECT_PATH_HEADING);
        assert_eq!(found.files.len(), 1);
        assert_eq!(found.files[0].name, "notes.md");
        assert_eq!(found.files[0].category, "Documents");

        let filtered = search
            .search(&query, Some(FileCategory::Image))
            .expect("a filtered direct path");
        assert_eq!(filtered.heading, DIRECT_PATH_HEADING);
        assert!(filtered.files.is_empty());
    }

    #[test]
    fn an_index_query_without_an_indexer_says_so() {
        let search = FileSearch::without_indexer();
        assert_eq!(search.search("report", None), Err(INDEXER_UNAVAILABLE));
        assert_eq!(
            search.search("/does/not/exist/anywhere", None),
            Err(INDEXER_UNAVAILABLE),
            "a path that does not exist is searched for instead"
        );
    }

    #[test]
    fn the_root_is_searched_for_rather_than_listed() {
        let search = FileSearch::without_indexer();
        assert_eq!(search.search("/", None), Err(INDEXER_UNAVAILABLE));
    }
}
