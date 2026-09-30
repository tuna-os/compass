//! The file index an extension reaches through `FileSearch/search`.
//!
//! The same `compass-file-indexer` helper Search Files asks, supervised by
//! the engine ([`crate::file_search::FileSearch`]). An extension gets the
//! index's rows only — the C++ `ExtFileSearchService` calls the indexer's
//! `queryAsync` and nothing else, so no recent files and no direct path.
//! With indexing off, or the helper not running, the answer is an empty
//! list, and `environment.canAccess(FileSearch)` is `false`.

use std::sync::Arc;

use compass_worker_host::file_search_service::{FileIndexer, IndexerFileResult, QueryParams};

use crate::file_search::FileSearch;

/// [`FileIndexer`] over the engine's file search.
#[derive(Debug, Clone)]
pub struct EngineFiles {
    search: Arc<FileSearch>,
}

impl EngineFiles {
    /// Serves `search`'s index.
    #[must_use]
    pub const fn new(search: Arc<FileSearch>) -> Self {
        Self { search }
    }
}

impl FileIndexer for EngineFiles {
    fn query(&self, query: &str, params: &QueryParams) -> Vec<IndexerFileResult> {
        // The wire's `limit` is a `uint`; the helper's is an `i32`.
        let limit = i32::try_from(params.limit.max(0)).unwrap_or(i32::MAX);
        self.search
            .index_query(query, limit, params.category)
            .into_iter()
            .map(|(path, rank, category, mime_type)| IndexerFileResult {
                path,
                rank,
                category,
                mime_type,
            })
            .collect()
    }

    fn is_available(&self) -> bool {
        self.search.index_available()
    }
}
