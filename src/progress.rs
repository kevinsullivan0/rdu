//! Optional per-scan work observations. Byte accounting remains in the final tree.
use std::{
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanPhase {
    Scanning,
    Accounting,
    Complete,
    Cancelled,
}

#[derive(Clone, Copy, Debug)]
pub struct ScanSnapshot {
    pub phase: ScanPhase,
    pub elapsed: Duration,
    pub entries_seen: u64,
    pub files_seen: u64,
    pub directories_discovered: u64,
    pub directories_completed: u64,
    pub active_workers: usize,
    pub errors: u64,
}

/// Create one observation object for each scan; snapshots are approximate live
/// observations across counters, not an atomic filesystem snapshot.
pub struct ScanProgress {
    started: Instant,
    phase: AtomicU8,
    cancelled: AtomicBool,
    pub(crate) entries: AtomicU64,
    pub(crate) files: AtomicU64,
    pub(crate) discovered: AtomicU64,
    pub(crate) completed: AtomicU64,
    pub(crate) active: AtomicUsize,
    pub(crate) errors: AtomicU64,
}

impl Default for ScanProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanProgress {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            phase: AtomicU8::new(0),
            cancelled: AtomicBool::new(false),
            entries: AtomicU64::new(0),
            files: AtomicU64::new(0),
            discovered: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            errors: AtomicU64::new(0),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> ScanSnapshot {
        let phase = match self.phase.load(Ordering::Acquire) {
            0 => ScanPhase::Scanning,
            1 => ScanPhase::Accounting,
            2 => ScanPhase::Complete,
            _ => ScanPhase::Cancelled,
        };
        ScanSnapshot {
            phase,
            elapsed: self.started.elapsed(),
            entries_seen: self.entries.load(Ordering::Relaxed),
            files_seen: self.files.load(Ordering::Relaxed),
            directories_discovered: self.discovered.load(Ordering::Relaxed),
            directories_completed: self.completed.load(Ordering::Relaxed),
            active_workers: self.active.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn accounting(&self) {
        self.phase.store(1, Ordering::Release);
    }

    pub(crate) fn finish(&self, cancelled: bool) {
        self.phase
            .store(if cancelled { 3 } else { 2 }, Ordering::Release);
    }

    pub(crate) fn observe(&self, entries: u64, files: u64) {
        self.entries.fetch_add(entries, Ordering::Relaxed);
        self.files.fetch_add(files, Ordering::Relaxed);
    }
}
