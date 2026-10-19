//! Search Files, the engine side.
//!
//! Ports what `SearchFilesViewHost` asks of the files service: recently
//! accessed files from `recently-used.xbel` for the empty query, the path
//! itself when the query names one that exists, and the file index otherwise.
//! The index is the `compass-file-indexer` helper, supervised by
//! [`IndexerClient`] and configured from the file extension's preferences,
//! the way `FileExtension::initialized` hands them to the files service.
//!
//! The view's debounce and stale-result checks live with the launcher, which
//! owns the timing; what is decided here is only what a query answers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

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

/// The engine's file search, sharing one helper across live settings changes.
#[derive(Debug)]
pub struct FileSearch {
    indexer: Arc<IndexerClient>,
    settings: Mutex<Option<IndexingSettings>>,
}

impl Default for FileSearch {
    fn default() -> Self {
        Self {
            indexer: IndexerClient::new(),
            settings: Mutex::new(None),
        }
    }
}

impl FileSearch {
    /// No running indexer: direct paths and recent files still answer.
    #[must_use]
    pub fn without_indexer() -> Self {
        Self::default()
    }

    /// Starts the indexer helper configured from `settings`, unless indexing
    /// is turned off. The same client can be enabled later.
    #[must_use]
    pub fn start(settings: IndexingSettings) -> Self {
        let files = Self::default();
        files.apply_settings(settings);
        files
    }

    /// Applies a settings snapshot to the existing helper, without rebuilding
    /// the index. Identical snapshots do nothing; disabled indexing keeps the
    /// configuration for the next start.
    ///
    /// Blocking: stopping joins the helper's supervisor. Call from a blocking
    /// task, without holding the engine's async lock. The local lock serializes
    /// lifecycle changes while queries continue to share the same client.
    pub fn apply_settings(&self, settings: IndexingSettings) {
        let mut applied = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        self.apply_settings_locked(&mut applied, settings);
    }

    /// Loads a fresh snapshot while serialized with other updates. Callbacks
    /// signal a reload rather than applying a snapshot that may already be stale.
    pub(crate) fn reload_settings(
        &self,
        load: impl FnOnce() -> Result<IndexingSettings, String>,
    ) -> Result<(), String> {
        let mut applied = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let settings = load()?;
        self.apply_settings_locked(&mut applied, settings);
        Ok(())
    }

    fn apply_settings_locked(&self, applied: &mut Option<IndexingSettings>, settings: IndexingSettings) {
        let rejected = self.indexer.configuration_rejected();
        if applied.as_ref() == Some(&settings)
            && (!settings.enabled || (self.indexer.is_running() && !rejected))
        {
            return;
        }
        if !settings.enabled {
            self.indexer.stop();
        }
        let paths_changed = applied.as_ref().is_none_or(|previous| {
            previous.paths != settings.paths || previous.excluded_paths != settings.excluded_paths
        });
        if paths_changed || rejected {
            self.indexer
                .configure(settings.paths.clone(), settings.excluded_paths.clone());
        }
        if settings.enabled {
            self.indexer.start();
        }
        *applied = Some(settings);
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
        self.indexer.is_running() && self.indexer.configuration_ready()
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
        if !self.index_available() {
            return Vec::new();
        }
        self.current_results(self.indexer.query(query, limit, category.map(wire_category)))
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

    /// A query may finish after a newer configuration has taken effect. The
    /// helper reconciles asynchronously, so never expose rows outside the
    /// latest accepted scope, including responses already in flight at stop.
    fn current_results(
        &self,
        mut results: Vec<compass_db::query_engine::IndexerFileResult>,
    ) -> Vec<compass_db::query_engine::IndexerFileResult> {
        let applied = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        if self.indexer.configuration_rejected() {
            return Vec::new();
        }
        let Some(settings) = applied.as_ref().filter(|settings| settings.enabled) else {
            return Vec::new();
        };
        results.retain(|result| {
            settings.paths.iter().any(|root| result.path.starts_with(root))
                && !settings.excluded_paths.iter().any(|excluded| result.path.starts_with(excluded))
        });
        results
    }

    /// Asks the index, under the heading its query earns.
    fn indexed(
        &self,
        query: &str,
        category: Option<FileCategory>,
    ) -> Result<FileResults, &'static str> {
        if !self.index_available() {
            return Err(INDEXER_UNAVAILABLE);
        }
        let files = self
            .current_results(self.indexer.query(query, INDEXED_QUERY_LIMIT, category.map(wire_category)))
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

    #[test]
    fn disabled_settings_are_staged_without_starting_a_helper() {
        let settings = IndexingSettings {
            enabled: false,
            paths: Vec::new(),
            excluded_paths: Vec::new(),
        };
        let search = FileSearch::start(settings.clone());
        assert!(!search.index_available());
        search.apply_settings(settings.clone());
        assert_eq!(*search.settings.lock().unwrap(), Some(settings));
        assert!(!search.index_available());
    }

    #[test]
    fn a_delayed_reload_reads_the_latest_file_instead_of_its_old_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let old = dir.path().join("old").to_string_lossy().into_owned();
        let latest = dir.path().join("latest").to_string_lossy().into_owned();
        let write = |root: &str| {
            std::fs::write(&path, serde_json::json!({
                "autoIndexing": false, "indexingPaths": [root]
            }).to_string()).unwrap();
        };
        let load = |path: &Path| -> Result<IndexingSettings, String> {
            let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            Ok(IndexingSettings::from_preferences(value.as_object(), None))
        };
        write(&old);
        let files = Arc::new(FileSearch::start(load(&path).unwrap()));
        let (release, delayed) = std::sync::mpsc::channel();
        let older_callback = {
            let files = Arc::clone(&files);
            let path = path.clone();
            std::thread::spawn(move || {
                delayed.recv().unwrap();
                files.reload_settings(|| load(&path)).unwrap();
            })
        };
        // A newer callback completes before the older blocking task is scheduled.
        write(&latest);
        files.reload_settings(|| load(&path)).unwrap();
        release.send(()).unwrap();
        older_callback.join().unwrap();
        assert_eq!(files.settings.lock().unwrap().as_ref().unwrap().paths, [latest]);
        std::fs::write(&path, "{invalid").unwrap();
        let previous = files.settings.lock().unwrap().clone();
        assert!(files.reload_settings(|| load(&path)).is_err());
        assert_eq!(*files.settings.lock().unwrap(), previous);
        assert!(!files.index_available());
    }

    #[test]
    fn shared_searches_filter_in_flight_rows_against_the_latest_scope() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kept");
        let files = Arc::new(FileSearch::without_indexer());
        let extension_search = Arc::clone(&files);
        let row = |path: PathBuf| compass_db::query_engine::IndexerFileResult {
            path, rank: 1.0, category: IndexedFileCategory::Document, mime_type: None,
        };
        // Set only the accepted snapshot: this pure result test never starts a helper.
        *files.settings.lock().unwrap() = Some(IndexingSettings {
            enabled: true,
            paths: vec![root.to_string_lossy().into_owned()],
            excluded_paths: vec![root.join("private").to_string_lossy().into_owned()],
        });
        let rows = extension_search.current_results(vec![
            row(root.join("public.txt")),
            row(root.join("private/secret.txt")),
            row(dir.path().join("kept-sibling/outside.txt")),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, root.join("public.txt"));
        files.settings.lock().unwrap().as_mut().unwrap().enabled = false;
        assert!(extension_search.current_results(rows).is_empty());
    }
}
