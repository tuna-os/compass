//! Installing an extension bundle onto disk.
//!
//! Ports `ExtensionRegistry::installFromZip`. The archive arrives from the
//! network, so the whole of this is about one property: **a bad download must
//! not destroy the installation it was going to replace.**
//!
//! Nothing here touches a filesystem. The steps are returned in order and the
//! caller performs them, which is what lets the ordering — the part that
//! carries the safety — be tested without unpacking an archive.

use std::path::{Path, PathBuf};

/// The prefix an extension installed from the Raycast store is filed under.
///
/// It keeps store extensions in their own namespace, so one cannot collide
/// with a locally developed extension of the same name.
pub const STORE_ID_PREFIX: &str = "store.raycast.";

/// The id a store extension is installed as.
#[must_use]
pub fn store_extension_id(name: &str) -> String {
    format!("{STORE_ID_PREFIX}{name}")
}

/// The file every bundle must contain to be an extension at all.
pub const MANIFEST_NAME: &str = "package.json";

/// How many leading path components the archive's entries are stripped of.
///
/// A published bundle wraps everything in one top-level directory named after
/// the extension; without stripping it, every extension would install one
/// level too deep and its manifest would never be found.
pub const STRIP_COMPONENTS: usize = 1;

/// Where a bundle is unpacked before it is trusted.
///
/// Beside the target rather than in a temporary directory, because the last
/// step is a rename and a rename across filesystems is a copy that can fail
/// halfway. The leading dot keeps it out of the registry's own listing.
#[must_use]
pub fn staging_dir(extensions_dir: &Path, id: &str) -> PathBuf {
    extensions_dir.join(format!(".staging-{id}"))
}

/// Where an extension lives once it is installed.
#[must_use]
pub fn install_dir(extensions_dir: &Path, id: &str) -> PathBuf {
    extensions_dir.join(id)
}

/// One step of an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Remove a directory and everything under it, ignoring failure.
    RemoveAll(PathBuf),
    /// Unpack the archive into a directory.
    Extract {
        /// Where the entries go.
        into: PathBuf,
        /// How many leading components to drop.
        strip_components: usize,
    },
    /// Check the manifest is there. A failure abandons the install.
    RequireManifest(PathBuf),
    /// Move the staged directory into place.
    Rename {
        /// The staging directory.
        from: PathBuf,
        /// The install directory.
        to: PathBuf,
    },
}

/// The steps that install a bundle, in order.
///
/// The order is the safety property and every part of it earns its place:
///
/// 1. **Clear the staging directory first.** A previous install that died
///    part-way leaves one behind, and unpacking over it would mix two
///    extensions into one.
/// 2. **Unpack into staging**, never into the target.
/// 3. **Check the manifest** — still in staging, so a truncated or wrong
///    archive is discarded with the installed version untouched. This is the
///    step the whole shape exists for.
/// 4. **Remove the target**, and only now, once the replacement is known good.
/// 5. **Rename staging into place**, which is atomic within a filesystem, so
///    there is no moment where the extension is half-written.
#[must_use]
pub fn install_steps(extensions_dir: &Path, id: &str) -> Vec<Step> {
    let staging = staging_dir(extensions_dir, id);
    let target = install_dir(extensions_dir, id);

    vec![
        Step::RemoveAll(staging.clone()),
        Step::Extract {
            into: staging.clone(),
            strip_components: STRIP_COMPONENTS,
        },
        Step::RequireManifest(staging.join(MANIFEST_NAME)),
        Step::RemoveAll(target.clone()),
        Step::Rename {
            from: staging,
            to: target,
        },
    ]
}

/// Why an install did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallFailure {
    /// The archive could not be opened at all.
    UnreadableArchive,
    /// It unpacked, but holds no manifest, so it is not an extension.
    NoManifest,
    /// The staged directory could not be moved into place.
    RenameFailed,
}

/// What to clean up after a failure.
///
/// Every failure removes the staging directory and **only** the staging
/// directory. The installed version is either still there — because the
/// failure happened before it was removed — or already replaced. Reaching for
/// the target here is how a cleanup path deletes a working extension because
/// its replacement was broken.
#[must_use]
pub fn cleanup_after(failure: &InstallFailure, extensions_dir: &Path, id: &str) -> Vec<Step> {
    match failure {
        // Nothing was unpacked, so there is nothing to clear — and the C++
        // returns before touching the disk at all.
        InstallFailure::UnreadableArchive => Vec::new(),
        InstallFailure::NoManifest | InstallFailure::RenameFailed => {
            vec![Step::RemoveAll(staging_dir(extensions_dir, id))]
        }
    }
}

/// Whether a failure could have left the previous version gone.
///
/// Only a failed rename can: it is the one failure that happens *after* the
/// target is removed. Saying so is worth a function because it is the
/// difference between "the install did not happen" and "the extension is now
/// missing", and a caller reporting the first when it is the second sends
/// someone looking in the wrong place.
#[must_use]
pub const fn may_have_removed_previous(failure: &InstallFailure) -> bool {
    matches!(failure, InstallFailure::RenameFailed)
}
