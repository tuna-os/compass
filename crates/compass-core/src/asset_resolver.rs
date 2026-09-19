//! Resolving an extension's relative assets.
//!
//! A complete port of `RelativeAssetResolver`
//! (`src/server/src/services/asset-resolver/asset-resolver.hpp`), which is a
//! list of base directories and a first-match lookup.
//!
//! A command pushes its asset directory when it loads and removes it when it
//! unloads, so several extensions' assets are searchable at once and the same
//! relative name can mean different files for different commands. The C++ is a
//! process-wide singleton; this is an ordinary value, because a singleton is
//! how that file is *reached*, not what it does.

use std::path::{Path, PathBuf};

/// The base directories, in the order they were added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetResolver {
    paths: Vec<PathBuf>,
}

impl AssetResolver {
    /// An empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The base directories, in search order.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// `addPath`: appends, without checking for duplicates.
    ///
    /// Adding the same directory twice leaves two entries, and [`Self::remove`]
    /// then takes one away — which is what makes two commands sharing an asset
    /// directory work: the second unload does not blind the first command.
    pub fn add(&mut self, path: impl Into<PathBuf>) {
        self.paths.push(path.into());
    }

    /// `removePath`: removes the **first** match, and does nothing when there
    /// is none.
    pub fn remove(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        if let Some(index) = self.paths.iter().position(|base| base == path) {
            self.paths.remove(index);
        }
    }

    /// `resolve`: the first base directory under which `relative` exists.
    ///
    /// `exists` answers `std::filesystem::exists`, which is true for a
    /// directory as well as a file, and follows symlinks. Order is the order
    /// the paths were added, so a command that loads later shadows nothing —
    /// it is searched last.
    #[must_use]
    pub fn resolve_with(
        &self,
        relative: impl AsRef<Path>,
        exists: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        let relative = relative.as_ref();
        self.paths
            .iter()
            .map(|base| base.join(relative))
            .find(|candidate| exists(candidate))
    }

    /// [`Self::resolve_with`], asking the filesystem.
    #[must_use]
    pub fn resolve(&self, relative: impl AsRef<Path>) -> Option<PathBuf> {
        self.resolve_with(relative, |path| path.exists())
    }
}
