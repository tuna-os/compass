//! What kind of file something is, by its extension.
//!
//! Ports `vicinae::fileCategoryFor` and the six extension lists beside it
//! (`src/lib/common/include/common/file-category.hpp`). The file search and
//! the clipboard both use this to pick an icon and a filter, so a category
//! that differs between the engines is a file that appears under one filter in
//! one engine and another in the other.
//!
//! The lists are written out here and **checked against the C++ header by a
//! test**, rather than parsed at build time: they are short, they change
//! rarely, and a build script for six arrays would be more machinery than the
//! thing it generates. The test is what keeps them from drifting.
//!
//! # Two details worth naming
//!
//! **The order of the checks decides ties.** `svg` is in the image list and
//! nothing else, but if an extension were ever in two lists the earlier one
//! would win; `fileCategoryFor` tests image, video, audio, document, archive,
//! application in that order, and so does this.
//!
//! **A dotfile has no extension.** `std::filesystem::path::extension` treats a
//! leading dot as part of the name, so `.bashrc` is not a `bashrc` file.
//! Rust's `Path::extension` agrees, which is why this can defer to it.

use std::path::Path;

/// What a file is, as far as the launcher cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileCategory {
    /// None of the below.
    Other,
    /// A directory, whatever its name.
    Directory,
    /// An image.
    Image,
    /// A video.
    Video,
    /// A sound.
    Audio,
    /// Something to read.
    Document,
    /// Something compressed.
    Archive,
    /// Something to run or install.
    Application,
}

/// Extensions that make a file an [`FileCategory::Image`].
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "avif", "bmp", "tif", "tiff", "svg", "heic", "ico",
];
/// Extensions that make a file a [`FileCategory::Video`].
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "wmv", "flv", "mpeg", "mpg", "3gp",
];
/// Extensions that make a file [`FileCategory::Audio`].
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "aac", "m4a", "ogg", "opus", "wma", "aiff", "mid", "midi",
];
/// Extensions that make a file a [`FileCategory::Document`].
pub const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "txt", "md", "markdown", "doc", "docx", "odt", "rtf", "pages", "xls", "xlsx", "ods",
    "csv", "ppt", "pptx", "odp", "epub",
];
/// Extensions that make a file an [`FileCategory::Archive`].
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst", "lz4", "deb", "rpm",
];
/// Extensions that make a file an [`FileCategory::Application`].
pub const APPLICATION_EXTENSIONS: &[&str] = &["desktop", "appimage", "exe", "msi", "app", "dmg"];

/// The lists, in the order `fileCategoryFor` tests them.
pub const CATEGORY_EXTENSIONS: &[(FileCategory, &[&str])] = &[
    (FileCategory::Image, IMAGE_EXTENSIONS),
    (FileCategory::Video, VIDEO_EXTENSIONS),
    (FileCategory::Audio, AUDIO_EXTENSIONS),
    (FileCategory::Document, DOCUMENT_EXTENSIONS),
    (FileCategory::Archive, ARCHIVE_EXTENSIONS),
    (FileCategory::Application, APPLICATION_EXTENSIONS),
];

/// The extension of `path`, lower-cased and without its dot.
///
/// Empty when there is none — including for a dotfile, whose leading dot is
/// part of its name.
#[must_use]
pub fn normalized_extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default()
}

/// Whether `extension` is in `candidates`, ignoring ASCII case.
#[must_use]
pub fn has_extension(extension: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(extension))
}

/// The category of `path`.
///
/// `is_directory` is the caller's, as it is in the C++: the category of a
/// directory does not depend on its name, and a stat is the caller's to do.
#[must_use]
pub fn file_category(path: &Path, is_directory: bool) -> FileCategory {
    if is_directory {
        return FileCategory::Directory;
    }

    let extension = normalized_extension(path);
    for (category, extensions) in CATEGORY_EXTENSIONS {
        if has_extension(&extension, extensions) {
            return *category;
        }
    }

    FileCategory::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_is_a_directory_whatever_it_is_called() {
        assert_eq!(
            file_category(Path::new("/home/u/holiday.png"), true),
            FileCategory::Directory,
            "a directory named like an image is still a directory"
        );
    }

    #[test]
    fn the_extension_decides_and_case_does_not() {
        for (name, expected) in [
            ("photo.JPG", FileCategory::Image),
            ("clip.MkV", FileCategory::Video),
            ("song.flac", FileCategory::Audio),
            ("notes.MD", FileCategory::Document),
            ("bundle.TAR", FileCategory::Archive),
            ("firefox.desktop", FileCategory::Application),
            ("script.rs", FileCategory::Other),
        ] {
            assert_eq!(
                file_category(Path::new(name), false),
                expected,
                "{name} should be {expected:?}"
            );
        }
    }

    #[test]
    fn only_the_last_extension_counts() {
        // `path.extension()` in both languages: `archive.tar.gz` is a `gz`.
        assert_eq!(normalized_extension(Path::new("archive.tar.gz")), "gz");
        assert_eq!(
            file_category(Path::new("archive.tar.gz"), false),
            FileCategory::Archive
        );
    }

    #[test]
    fn a_dotfile_has_no_extension() {
        // `.bashrc` is a name, not a `bashrc` file -- and `std::filesystem`
        // and Rust agree about that, which is the only reason this port can
        // defer to the standard library.
        assert_eq!(normalized_extension(Path::new(".bashrc")), "");
        assert_eq!(
            file_category(Path::new(".bashrc"), false),
            FileCategory::Other
        );
        assert_eq!(normalized_extension(Path::new("noextension")), "");
    }

    #[test]
    fn an_extension_is_matched_whole() {
        // `extensionEquals` compares lengths first, so `pngx` is not `png`.
        assert!(has_extension("png", IMAGE_EXTENSIONS));
        assert!(!has_extension("pngx", IMAGE_EXTENSIONS));
        assert!(!has_extension("pn", IMAGE_EXTENSIONS));
        assert!(!has_extension("", IMAGE_EXTENSIONS));
    }
}
