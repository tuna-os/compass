//! `FileSearch/search`, read against
//! `src/server/src/extension/api/file-search-service.hpp` and `figura/tsapi.fig`.
//!
//! The indexer here is a recording stub: it answers whatever the test gave it
//! and keeps the [`QueryParams`] it was asked with, so the tests can pin both
//! halves of the adapter — what reaches the indexer, and what reaches the wire.

use std::cell::RefCell;
use std::path::PathBuf;

use compass_core::file_category::FileCategory;
use compass_worker_host::file_search_service::{
    FileIndexer, FileSearchService, IndexerFileResult, QueryParams,
};
use compass_worker_host::tsapi::Call;

#[derive(Default)]
struct Stub {
    rows: Vec<IndexerFileResult>,
    seen: RefCell<Vec<(String, QueryParams)>>,
}

impl FileIndexer for Stub {
    fn query(&self, query: &str, params: &QueryParams) -> Vec<IndexerFileResult> {
        self.seen
            .borrow_mut()
            .push((query.to_owned(), params.clone()));
        self.rows.clone()
    }
}

fn row(path: &str, category: FileCategory, mime: Option<&str>) -> IndexerFileResult {
    IndexerFileResult {
        path: PathBuf::from(path),
        rank: 1.0,
        category,
        mime_type: mime.map(str::to_owned),
    }
}

fn call(params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "FileSearch/search",
        "params": params,
    }))
    .expect("a well-formed call")
}

/// The `result` of the reply, parsed.
fn result(answer: &str) -> serde_json::Value {
    let reply: serde_json::Value = serde_json::from_str(answer).expect("a JSON reply");
    assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
    assert_eq!(reply["id"], 7, "{reply}");
    reply["result"].clone()
}

#[test]
fn a_search_reaches_the_indexer_with_the_query_the_extension_typed() {
    let service = FileSearchService::new(Stub::default());
    let answer = service
        .handle(&call(serde_json::json!({
            "q": "quarterly report",
            "opts": {"filters": {}, "limit": 25},
        })))
        .expect("a FileSearch call is answered");

    assert_eq!(result(&answer), serde_json::json!([]));
    let seen = service.indexer().seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "quarterly report");
    assert_eq!(seen[0].1.limit, 25);
    assert_eq!(seen[0].1.category, None);
}

#[test]
fn a_row_becomes_the_file_info_the_idl_declares() {
    // `struct FileInfo { path: string; category: FileSearchCategory; mimeType?: string; }`
    let service = FileSearchService::new(Stub {
        rows: vec![row("/home/u/a.png", FileCategory::Image, Some("image/png"))],
        ..Stub::default()
    });

    let answer = service
        .handle(&call(serde_json::json!({"q": "a", "opts": {"limit": 5}})))
        .expect("answered");
    assert_eq!(
        result(&answer),
        serde_json::json!([{
            "path": "/home/u/a.png",
            "category": "Image",
            "mimeType": "image/png",
        }])
    );
}

#[test]
fn an_unknown_mime_type_is_an_absent_key_not_a_null() {
    // `mimeType?: string` in the generated TypeScript -- `undefined`, not
    // `null`, is what an extension checks for.
    let service = FileSearchService::new(Stub {
        rows: vec![row("/home/u/notes", FileCategory::Directory, None)],
        ..Stub::default()
    });

    let answer = service
        .handle(&call(
            serde_json::json!({"q": "notes", "opts": {"limit": 5}}),
        ))
        .expect("answered");
    let files = result(&answer);
    assert!(
        files[0].get("mimeType").is_none(),
        "the key should be absent: {files}"
    );
}

#[test]
fn every_category_survives_the_round_trip() {
    // Both C++ `mapCategory` switches, which the compiler checks for
    // exhaustiveness there and this checks for here.
    let all = [
        (FileCategory::Other, "Other"),
        (FileCategory::Directory, "Directory"),
        (FileCategory::Image, "Image"),
        (FileCategory::Video, "Video"),
        (FileCategory::Audio, "Audio"),
        (FileCategory::Document, "Document"),
        (FileCategory::Archive, "Archive"),
        (FileCategory::Application, "Application"),
    ];

    for (category, name) in all {
        let service = FileSearchService::new(Stub {
            rows: vec![row("/f", category, None)],
            ..Stub::default()
        });
        let answer = service
            .handle(&call(serde_json::json!({
                "q": "f",
                "opts": {"filters": {"category": name}, "limit": 1},
            })))
            .expect("answered");

        assert_eq!(result(&answer)[0]["category"], name, "outbound {name}");
        assert_eq!(
            service.indexer().seen.borrow()[0].1.category,
            Some(category),
            "inbound {name}"
        );
    }
}

#[test]
fn a_category_the_idl_does_not_declare_is_an_error_not_a_guess() {
    let service = FileSearchService::new(Stub::default());
    let answer = service
        .handle(&call(serde_json::json!({
            "q": "f",
            "opts": {"filters": {"category": "Spreadsheet"}, "limit": 1},
        })))
        .expect("answered");

    let reply: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert!(
        reply["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Spreadsheet"),
        "the error should name the category: {reply}"
    );
    assert!(
        service.indexer().seen.borrow().is_empty(),
        "and the indexer should not be asked at all"
    );
}

#[test]
fn an_omitted_limit_asks_for_nothing_the_way_the_cpp_does() {
    // `IndexerQueryParams::limit` defaults to 100, but `ExtFileSearchService`
    // always assigns `opts.limit`, and glaze value-initialises a `uint` the
    // JSON omits. So an extension that leaves `limit` out asks for zero rows.
    // Reproduced deliberately: guessing 100 here would make the Rust host
    // return results the C++ host does not.
    let service = FileSearchService::new(Stub::default());
    service
        .handle(&call(
            serde_json::json!({"q": "f", "opts": {"filters": {}}}),
        ))
        .expect("answered");

    assert_eq!(service.indexer().seen.borrow()[0].1.limit, 0);
    // The named default is still the C++ one, for a caller building params by hand.
    assert_eq!(QueryParams::default().limit, 100);
}

#[test]
fn the_adapter_does_not_re_filter_what_the_indexer_returned() {
    // The C++ returns `results | transform(toFileInfo)` -- no truncation, no
    // re-filtering. An indexer that overshoots its limit is a bug to see, not
    // to paper over.
    let service = FileSearchService::new(Stub {
        rows: vec![
            row("/a", FileCategory::Image, None),
            row("/b", FileCategory::Video, None),
            row("/c", FileCategory::Audio, None),
        ],
        ..Stub::default()
    });

    let answer = service
        .handle(&call(serde_json::json!({
            "q": "f",
            "opts": {"filters": {"category": "Image"}, "limit": 1},
        })))
        .expect("answered");
    assert_eq!(result(&answer).as_array().map(Vec::len), Some(3));
}

#[test]
fn another_services_call_is_not_answered_here() {
    let service = FileSearchService::new(Stub::default());
    let mut other = call(serde_json::json!({"q": "f"}));
    other.method = "Storage/get".to_owned();

    assert!(service.handle(&other).is_none());
    assert!(service.indexer().seen.borrow().is_empty());
}

#[test]
fn an_event_is_not_answered_either() {
    // No id: `FileSearch` declares no events, but a malformed message must not
    // produce a reply the client has nothing to resolve.
    let service = FileSearchService::new(Stub::default());
    let event: Call = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "FileSearch/search",
        "params": {"q": "f"},
    }))
    .unwrap();

    assert!(service.handle(&event).is_none());
}
