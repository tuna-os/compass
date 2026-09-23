//! What a scan is and what it reports.
//!
//! Ports the `Scan`, `FullScan`, `IncrementalScan`, `ScanEvent`, and
//! `ScanMode` types from `scan.hpp`: the dispatcher and the scanners pass
//! these around, and the status callback reports these back.

use std::path::PathBuf;

use crate::db_writer::{ScanStatus, ScanType};

/// How thoroughly an incremental scan reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    /// Read everything under the entrypoint.
    Exhaustive,
    /// Skip what the database says is unchanged.
    Pruned,
}

/// A full scan and what it skips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullScan {
    /// Subtrees to leave out.
    pub excluded_paths: Vec<PathBuf>,
}

/// An incremental scan and how far it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncrementalScan {
    /// How thoroughly to read.
    pub mode: ScanMode,
    /// How deep to descend, when bounded.
    pub max_depth: Option<usize>,
    /// Basenames to leave out.
    pub excluded_filenames: Vec<String>,
    /// Subtrees to leave out.
    pub excluded_paths: Vec<PathBuf>,
}

/// One scan request: where, what shape, and whether anyone listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Where to scan.
    pub path: PathBuf,
    /// What shape the scan is.
    pub data: ScanData,
    /// Whether progress events go out.
    pub notify: bool,
}

/// The shape of a [`Scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanData {
    /// Read everything.
    Full(FullScan),
    /// Read what changed.
    Incremental(IncrementalScan),
}

impl Scan {
    /// Which shape this scan is.
    #[must_use]
    pub fn scan_type(&self) -> ScanType {
        match self.data {
            ScanData::Full(_) => ScanType::Full,
            ScanData::Incremental(_) => ScanType::Incremental,
        }
    }
}

/// What the dispatcher hears about a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEvent {
    /// The dispatcher's id for the scan.
    pub scan_id: i32,
    /// Which shape the scan was.
    pub scan_type: ScanType,
    /// Where the scan stands.
    pub status: ScanStatus,
    /// What was scanned.
    pub entrypoint: PathBuf,
    /// How many files the scan processed.
    pub processed_file_count: usize,
}
