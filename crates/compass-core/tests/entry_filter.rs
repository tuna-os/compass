//! What the file indexer refuses to walk into.
//!
//! Read off `EntryFilter` (`src/file-indexer/src/entry-filter.cpp`).

use std::path::{Path, PathBuf};

use compass_core::entry_filter::{
    CACHEDIR_TAG_SIGNATURE, EXCLUDED_ABSOLUTE_PATHS, EXCLUDED_FILENAMES, Entry, EntryFilter,
    GitIgnoreReader, NOINDEX_SUFFIX, basename, excluded_paths, glob_match,
    has_cachedir_tag_signature, is_cachedir_tag, is_hidden_path, is_machine_trash_directory,
    last_path_component,
};

/// No ignore files anywhere.
fn no_ignore_files(_path: &Path) -> Option<String> {
    None
}

/// An ordinary file entry at `path`.
fn file(path: &str) -> Entry<'_> {
    Entry {
        path: Path::new(path),
        is_symlink: false,
        is_directory: false,
    }
}

/// A directory entry at `path`.
fn directory(path: &str) -> Entry<'_> {
    Entry {
        path: Path::new(path),
        is_symlink: false,
        is_directory: true,
    }
}

/// A filter with the built-ins resolved under `/home/ada`.
fn filter() -> EntryFilter {
    EntryFilter::new(Some(Path::new("/home/ada")))
}

#[test]
fn an_ordinary_file_is_visited() {
    assert!(filter().should_visit(file("/home/ada/notes.txt"), no_ignore_files));
}

#[test]
fn a_symlink_is_never_followed() {
    // The first check, and the one that keeps a walk from looping for ever
    // through a link back up the tree.
    let entry = Entry {
        path: Path::new("/home/ada/link"),
        is_symlink: true,
        is_directory: true,
    };
    assert!(!filter().should_visit(entry, no_ignore_files));
}

#[test]
fn the_kernel_and_runtime_filesystems_are_never_walked() {
    // An indexer that walks /proc never finishes.
    for path in EXCLUDED_ABSOLUTE_PATHS {
        assert!(
            !filter().should_visit(directory(path), no_ignore_files),
            "{path} should be excluded"
        );
    }
    assert!(EXCLUDED_ABSOLUTE_PATHS.contains(&"/proc"));
    assert!(EXCLUDED_ABSOLUTE_PATHS.contains(&"/sys"));
}

#[test]
fn the_home_relative_exclusions_resolve_under_the_home_directory() {
    let paths = excluded_paths(Some(Path::new("/home/ada")));
    assert!(paths.contains(&PathBuf::from("/home/ada/.cargo/registry")));
    assert!(
        !paths.contains(&PathBuf::from("/home/ada/node_modules")),
        "node_modules is a filename rule, not a path one"
    );
    assert!(paths.contains(&PathBuf::from("/proc")));
}

#[test]
fn a_package_cache_is_excluded_by_its_whole_path_not_its_name() {
    // ~/.cargo/registry is excluded; a directory called "registry" elsewhere
    // is not, because the exclusion is a path and not a filename.
    assert!(!filter().should_visit(directory("/home/ada/.cargo/registry"), no_ignore_files));

    let mut open = filter();
    open.set_ignore_hidden_paths(false);
    assert!(open.should_visit(directory("/home/ada/projects/registry"), no_ignore_files));
}

#[test]
fn the_home_relative_list_is_the_cpp_list() {
    // Each of these is a directory with an enormous file count and nothing a
    // person searches for by name; one quietly dropped is a search that stops
    // being useful, not a bug anyone reports.
    for expected in [
        ".cargo/registry",
        ".rustup",
        ".npm",
        ".local/share/Trash",
        ".local/share/flatpak/runtime",
        "snap",
    ] {
        assert!(
            compass_core::entry_filter::EXCLUDED_HOME_RELATIVE_PATHS.contains(&expected),
            "{expected} should be excluded"
        );
    }
}

#[test]
fn without_a_home_directory_only_the_absolute_exclusions_apply() {
    let paths = excluded_paths(None);
    assert_eq!(paths.len(), EXCLUDED_ABSOLUTE_PATHS.len());
    assert!(paths.contains(&PathBuf::from("/proc")));
}

#[test]
fn the_excluded_filenames_are_skipped_wherever_they_appear() {
    for name in ["node_modules", ".git", "__pycache__", ".venv", "lost+found"] {
        assert!(
            EXCLUDED_FILENAMES.contains(&name),
            "{name} should be on the list"
        );
        let path = format!("/home/ada/project/{name}");
        assert!(
            !filter().should_visit(directory(&path), no_ignore_files),
            "{path} should be excluded"
        );
    }
}

#[test]
fn a_noindex_suffix_excludes_anything() {
    // The Spotlight convention, honoured by many apps.
    assert_eq!(NOINDEX_SUFFIX, ".noindex");
    assert!(!filter().should_visit(directory("/home/ada/Big.noindex"), no_ignore_files));
    assert!(!filter().should_visit(file("/home/ada/data.noindex"), no_ignore_files));
}

#[test]
fn configured_filenames_are_excluded_as_well_as_the_built_in_ones() {
    let mut filter = filter();
    filter.set_excluded_filenames(vec!["target".to_owned()]);
    assert!(!filter.should_visit(directory("/home/ada/project/target"), no_ignore_files));
    assert!(filter.should_visit(directory("/home/ada/project/src"), no_ignore_files));
}

#[test]
fn configured_paths_are_excluded() {
    let mut filter = filter();
    filter.set_excluded_paths(vec![PathBuf::from("/home/ada/Videos")]);
    assert!(!filter.should_visit(directory("/home/ada/Videos"), no_ignore_files));
    assert!(filter.should_visit(directory("/home/ada/Videos/holiday"), no_ignore_files));
}

#[test]
fn hidden_paths_are_skipped_only_when_asked_for() {
    let mut filter = filter();
    assert!(
        filter.should_visit(file("/home/ada/.bashrc"), no_ignore_files),
        "off by default"
    );

    filter.set_ignore_hidden_paths(true);
    assert!(!filter.should_visit(file("/home/ada/.bashrc"), no_ignore_files));
}

#[test]
fn a_hidden_ancestor_hides_everything_below_it() {
    // isHiddenPath tests every component, which is what makes this skip whole
    // trees rather than individual files.
    let mut filter = filter();
    filter.set_ignore_hidden_paths(true);
    assert!(!filter.should_visit(file("/home/ada/.config/app/settings.json"), no_ignore_files));
    assert!(is_hidden_path(Path::new(
        "/home/ada/.config/app/settings.json"
    )));
    assert!(!is_hidden_path(Path::new("/home/ada/config/app.json")));
}

#[test]
fn a_machine_cache_directory_is_skipped_but_a_file_of_that_name_is_not() {
    // The check is guarded by is_directory in the C++, so a *file* called
    // GPUCache is indexed. That asymmetry is the port's to keep.
    assert!(!filter().should_visit(directory("/home/ada/app/GrShaderCache"), no_ignore_files));
    assert!(filter().should_visit(file("/home/ada/app/GrShaderCache"), no_ignore_files));
}

#[test]
fn cache_data_is_only_trash_inside_a_cache() {
    // "Cache_Data" is specific enough to be a machine's, but only where the
    // surrounding path says so.
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/app/Cache/Cache_Data"
    )));
    assert!(!is_machine_trash_directory(Path::new(
        "/home/ada/notes/Cache_Data"
    )));
}

#[test]
fn a_directory_called_cache_is_only_trash_in_the_right_company() {
    // "cache" alone is far too common to exclude everywhere.
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/app/Shared Dictionary/cache"
    )));
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/.var/app/org.x/cache"
    )));
    assert!(!is_machine_trash_directory(Path::new(
        "/home/ada/project/cache"
    )));
}

#[test]
fn a_var_cache_is_only_trash_when_cache_comes_after_var() {
    // pathContainsAfter, not pathContains: the order is the rule.
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/.var/x/cache"
    )));
    assert!(!is_machine_trash_directory(Path::new(
        "/home/ada/cache/x/.var"
    )));
}

#[test]
fn the_named_machine_caches_are_trash_anywhere() {
    for name in [
        "GPUCache",
        "Code Cache",
        "CacheStorage",
        "Service Worker",
        "blob_storage",
        "DawnGraphiteCache",
        "DawnWebGPUCache",
        "mesa_shader_cache",
        "shader_cache",
        "ShaderCache",
        "GrShaderCache",
        "Crashpad",
        "crashpad",
    ] {
        let path = format!("/anywhere/at/all/{name}");
        assert!(
            is_machine_trash_directory(Path::new(&path)),
            "{name} should be machine trash"
        );
    }
}

#[test]
fn a_cachedir_tag_needs_the_signature_and_not_just_the_name() {
    // A file merely named CACHEDIR.TAG is not one, and treating it as one
    // would drop a directory somebody wanted indexed.
    let path = Path::new("/home/ada/build/CACHEDIR.TAG");
    assert!(is_cachedir_tag(path, CACHEDIR_TAG_SIGNATURE.as_bytes()));
    assert!(!is_cachedir_tag(path, b"something else entirely"));
    assert!(!is_cachedir_tag(
        Path::new("/home/ada/CACHEDIR.TAG.bak"),
        CACHEDIR_TAG_SIGNATURE.as_bytes()
    ));
}

#[test]
fn the_cachedir_signature_is_the_one_from_the_spec() {
    assert_eq!(
        CACHEDIR_TAG_SIGNATURE,
        "Signature: 8a477f597d28d172789f06886806bc55"
    );
    assert!(has_cachedir_tag_signature(
        format!("{CACHEDIR_TAG_SIGNATURE}\n# more text").as_bytes()
    ));
    assert!(!has_cachedir_tag_signature(b"Signature: "));
}

#[test]
fn no_ignore_files_configured_means_no_ignore_files_read() {
    // The only rule that touches the disk, so it must not run for nothing.
    let filter = filter();
    assert!(!filter.is_ignored(Path::new("/home/ada/p/x.o"), |_| {
        panic!("no ignore file should be read")
    }));
}

#[test]
fn a_gitignore_beside_the_file_hides_it() {
    let mut filter = filter();
    filter.set_ignore_files(vec![".gitignore".to_owned()]);

    let hidden = filter.should_visit(file("/home/ada/p/notes.log"), |path| {
        (path == Path::new("/home/ada/p/.gitignore")).then(|| "*.log".to_owned())
    });
    assert!(!hidden);
}

#[test]
fn a_gitignore_at_the_top_of_a_tree_applies_far_below_it() {
    // The walk goes all the way up, so a repository's .gitignore covers a file
    // nested many directories under it.
    let mut filter = filter();
    filter.set_ignore_files(vec![".gitignore".to_owned()]);

    let ignored = filter.is_ignored(Path::new("/home/ada/repo/a/b/c/out.o"), |path| {
        (path == Path::new("/home/ada/repo/.gitignore")).then(|| "*.o".to_owned())
    });
    assert!(ignored);
}

#[test]
fn a_gitignore_that_matches_nothing_leaves_the_file_alone() {
    let mut filter = filter();
    filter.set_ignore_files(vec![".gitignore".to_owned()]);

    assert!(filter.should_visit(file("/home/ada/p/notes.txt"), |path| {
        (path == Path::new("/home/ada/p/.gitignore")).then(|| "*.log".to_owned())
    }));
}

#[test]
fn several_ignore_file_names_can_be_honoured() {
    let mut filter = filter();
    filter.set_ignore_files(vec![".gitignore".to_owned(), ".ignore".to_owned()]);
    assert_eq!(filter.ignore_files().len(), 2);

    assert!(filter.is_ignored(Path::new("/home/ada/p/x.tmp"), |path| {
        (path == Path::new("/home/ada/p/.ignore")).then(|| "*.tmp".to_owned())
    }));
}

#[test]
fn only_the_filename_is_matched_against_a_pattern() {
    // The C++ calls this a hack in a comment: directory separators are not
    // handled, so build/*.o never matches anything. Reproduced, because
    // narrowing it would start indexing directories people's .gitignore files
    // currently keep out.
    let reader = GitIgnoreReader::from_text("build/*.o");
    assert!(!reader.matches(Path::new("/home/ada/repo/build/main.o")));

    let by_name = GitIgnoreReader::from_text("*.o");
    assert!(by_name.matches(Path::new("/home/ada/repo/build/main.o")));
}

#[test]
fn a_leading_slash_is_stripped_rather_than_anchoring_the_pattern() {
    // So /target matches a target anywhere in the tree, not just beside the
    // ignore file.
    let reader = GitIgnoreReader::from_text("/target");
    assert!(reader.matches(Path::new("/home/ada/repo/a/b/target")));
}

#[test]
fn every_line_including_comments_becomes_a_pattern() {
    // The C++ reads the file straight into a vector with no filtering. A
    // comment never matches anything real, which is why nobody has noticed.
    let reader = GitIgnoreReader::from_text("# a comment\n\n*.log\n");
    assert_eq!(reader.patterns().len(), 3);
    assert!(reader.matches(Path::new("/x/y.log")));
    assert!(!reader.matches(Path::new("/x/# a comment.txt")));
}

#[test]
fn an_empty_pattern_matches_only_an_empty_name() {
    let reader = GitIgnoreReader::from_text("\n");
    assert!(!reader.matches(Path::new("/home/ada/notes.txt")));
}

#[test]
fn globs_match_the_way_fnmatch_does() {
    assert!(glob_match("*.log", "server.log"));
    assert!(!glob_match("*.log", "server.txt"));
    assert!(glob_match("?.log", "a.log"));
    assert!(!glob_match("?.log", "ab.log"));
    assert!(glob_match("build", "build"));
    assert!(glob_match("*", "anything"));
}

#[test]
fn a_star_does_not_cross_a_separator() {
    // FNM_PATHNAME. The names matched here never contain one, but the rule is
    // what keeps the pattern from reaching across directories if they ever do.
    assert!(!glob_match("*.o", "build/main.o"));
    assert!(glob_match("*.o", "main.o"));
}

#[test]
fn a_character_class_matches_a_set_and_a_range() {
    assert!(glob_match("[abc].txt", "b.txt"));
    assert!(!glob_match("[abc].txt", "d.txt"));
    assert!(glob_match("[a-z].txt", "q.txt"));
    assert!(!glob_match("[a-z].txt", "Q.txt"));
    assert!(glob_match("[!a-z].txt", "Q.txt"), "a negated class");
}

#[test]
fn the_basename_is_taken_after_the_last_separator() {
    assert_eq!(basename("/home/ada/notes.txt"), "notes.txt");
    assert_eq!(basename("notes.txt"), "notes.txt");
    assert_eq!(
        basename("/home/ada/"),
        "",
        "a string operation, so a trailing slash gives an empty name"
    );
}

#[test]
fn the_last_path_component_falls_back_to_the_parents_name() {
    // getLastPathComponent, which is what a pattern is matched against — so a
    // directory handed in with a trailing slash still matches by its own name.
    assert_eq!(
        last_path_component(Path::new("/home/ada/notes.txt")),
        "notes.txt"
    );
    assert_eq!(last_path_component(Path::new("/home/ada/repo/")), "repo");
}

#[test]
fn the_rules_run_cheapest_first() {
    // A symlink is refused without any string work, and the disk-touching
    // ignore-file walk never runs for an entry excluded by name.
    let mut filter = filter();
    filter.set_ignore_files(vec![".gitignore".to_owned()]);

    let entry = Entry {
        path: Path::new("/home/ada/project/node_modules"),
        is_symlink: false,
        is_directory: true,
    };
    assert!(!filter.should_visit(entry, |_| panic!(
        "an entry excluded by name must not cost a disk read"
    )));
}
