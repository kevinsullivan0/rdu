//! Explicit, freshly reviewed deletion capabilities. Staging never writes.
use crate::scan::{Options, Tree};
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[cfg(target_os = "macos")]
mod macos;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Target {
    pub directory: usize,
    pub entry: usize,
}
#[derive(Debug)]
pub struct PlanSummary {
    pub paths: Vec<PathBuf>,
    pub target_count: usize,
    pub object_count: usize,
    pub allocated_bytes: u64,
    pub confirmation_phrase: String,
    pub notes: Vec<String>,
}
#[derive(Debug)]
pub struct DeletionSession {
    #[cfg(target_os = "macos")]
    inner: macos::Session,
}
#[derive(Debug)]
pub struct PreparedPlan {
    summary: PlanSummary,
    #[cfg(target_os = "macos")]
    inner: macos::Plan,
}
#[derive(Debug)]
pub struct ConfirmedPlan(PreparedPlan);
#[derive(Default, Debug)]
pub struct PrepareControl {
    cancelled: AtomicBool,
}
impl PrepareControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "deletion preparation cancelled",
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Default, Debug)]
pub struct DeleteControl {
    cancelled: AtomicBool,
    completed: AtomicUsize,
    total: AtomicUsize,
    purged: AtomicUsize,
    preserved: AtomicUsize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DeleteProgress {
    pub completed: usize,
    pub total: usize,
    pub purged: usize,
    pub preserved: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeState {
    Purged,
    Restored,
    Preserved,
    NotAttempted,
    Uncertain,
}
#[derive(Debug)]
pub struct ObjectOutcome {
    pub path: PathBuf,
    pub state: OutcomeState,
    pub detail: String,
}
#[derive(Debug, Default)]
pub struct DeleteReport {
    pub total: usize,
    pub purged: usize,
    /// Includes both restored and retained objects.
    pub preserved: usize,
    pub not_attempted: usize,
    pub uncertain: usize,
    /// Details are capped at 4096 records / 8 MiB. The journal is retained when capped.
    pub details_truncated: bool,
    pub outcomes: Vec<ObjectOutcome>,
    pub recovery: Option<PathBuf>,
    pub cancelled: bool,
    pub error: Option<String>,
}
impl DeleteControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn snapshot(&self) -> DeleteProgress {
        DeleteProgress {
            completed: self.completed.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            purged: self.purged.load(Ordering::Relaxed),
            preserved: self.preserved.load(Ordering::Relaxed),
        }
    }
}
impl DeletionSession {
    pub fn scan(
        path: &Path,
        options: &Options,
        read_only: bool,
    ) -> io::Result<(Tree, Option<Self>)> {
        #[cfg(target_os = "macos")]
        {
            macos::scan(path, options, read_only)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let tree = crate::scan::scan(path, options)?;
            let _ = read_only;
            Ok((tree, None))
        }
    }
    pub fn prepare(&self, targets: &[Target]) -> io::Result<PreparedPlan> {
        self.prepare_with_control(targets, &PrepareControl::default())
    }
    pub fn prepare_with_control(
        &self,
        targets: &[Target],
        control: &PrepareControl,
    ) -> io::Result<PreparedPlan> {
        control.check()?;
        #[cfg(target_os = "macos")]
        {
            self.inner.prepare(targets, control)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = targets;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "permanent deletion requires tested local macOS APFS",
            ))
        }
    }
}
impl PreparedPlan {
    pub fn summary(&self) -> &PlanSummary {
        &self.summary
    }
    /// Exact fresh manifest path for inspection without copying every full path.
    pub fn manifest_path(&self, index: usize) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            self.inner.manifest_path(index)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = index;
            None
        }
    }
    pub fn confirm(self, phrase: &str) -> io::Result<ConfirmedPlan> {
        if phrase != self.summary.confirmation_phrase {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "confirmation phrase must match exactly",
            ));
        }
        #[cfg(target_os = "macos")]
        self.inner.check_age()?;
        Ok(ConfirmedPlan(self))
    }
}
pub fn execute(plan: ConfirmedPlan, control: &DeleteControl) -> DeleteReport {
    #[cfg(target_os = "macos")]
    {
        macos::execute(plan.0, control)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (plan, control);
        DeleteReport {
            error: Some("permanent deletion requires tested local macOS APFS".into()),
            ..Default::default()
        }
    }
}
