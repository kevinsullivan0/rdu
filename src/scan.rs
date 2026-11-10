use crate::native::{self, Meta};
mod owner;
pub use crate::progress::{ScanPhase, ScanProgress, ScanSnapshot};
use std::{
    collections::{HashSet, VecDeque},
    ffi::{CString, OsString},
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Symlink,
    Other,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Identity {
    pub dev: u64,
    pub ino: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeOrigin {
    Live,
    Imported,
}
#[derive(Debug)]
pub struct Entry {
    pub identity: Identity,
    pub name: OsString,
    pub kind: Kind,
    pub allocated: u64,
    pub apparent: u64,
    pub child: Option<usize>,
}
#[derive(Debug)]
pub struct Directory {
    pub identity: Identity,
    pub name: OsString,
    pub parent: Option<usize>,
    pub entries: Vec<Entry>,
    pub allocated: u64,
    pub apparent: u64,
    pub errors: u64,
}
#[derive(Debug)]
pub struct Tree {
    pub origin: TreeOrigin,
    pub complete: bool,
    pub root: PathBuf,
    pub dirs: Vec<Directory>,
    pub files: u64,
    pub errors: u64,
    pub elapsed: Duration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Auto,
    Portable,
    Bulk,
}
#[derive(Clone)]
pub struct Options {
    pub threads: usize,
    pub backend: Backend,
    pub one_file_system: bool,
    pub max_entries: usize,
    pub max_depth: usize,
    pub cancelled: Option<fn() -> bool>,
    pub progress: Option<Arc<ScanProgress>>,
    pub filters: Option<Arc<crate::filter::Filters>>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            threads: 8,
            backend: Backend::Auto,
            one_file_system: false,
            max_entries: 10_000_000,
            max_depth: 1024,
            cancelled: None,
            progress: None,
            filters: None,
        }
    }
}
impl Options {
    fn is_cancelled(&self) -> bool {
        self.cancelled.is_some_and(|f| f())
            || self.progress.as_ref().is_some_and(|p| p.is_cancelled())
    }
}
#[derive(Clone)]
struct Job {
    id: usize,
    parent: Option<usize>,
    name: OsString,
    path: PathBuf,
    expected: Meta,
    depth: usize,
}
#[derive(Default)]
struct CapabilityCache {
    slots: VecDeque<(usize, OwnedFd)>,
}
impl CapabilityCache {
    fn parent(&mut self, id: usize) -> Option<&OwnedFd> {
        let at = self.slots.iter().position(|(key, _)| *key == id)?;
        let item = self.slots.remove(at).unwrap();
        self.slots.push_back(item);
        Some(&self.slots.back().unwrap().1)
    }
    fn retain(&mut self, id: usize, fd: OwnedFd) {
        if self.slots.len() == 2 {
            self.slots.pop_front();
        }
        self.slots.push_back((id, fd));
    }
}
struct Batch {
    job: Job,
    entries: Vec<(Entry, Option<(u64, u64)>)>,
    own: Meta,
    errors: u64,
}

pub fn scan(path: &Path, options: &Options) -> io::Result<Tree> {
    let start = Instant::now();
    if options.threads == 0 || options.threads > 64 || options.max_entries == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "threads must be 1..64 and max-entries positive",
        ));
    }
    #[cfg(not(target_os = "macos"))]
    if options.backend == Backend::Bulk {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bulk backend requires macOS",
        ));
    }
    let absolute: PathBuf = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    }
    .components()
    .collect();
    let name = CString::new(absolute.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
    let root = native::open_at(libc::AT_FDCWD, &name)?;
    let root_meta = native::stat_fd(root.as_raw_fd())?;
    if let Some(progress) = &options.progress {
        progress.discovered.store(1, Ordering::Relaxed);
    }
    let initial = Job {
        id: 0,
        parent: None,
        name: absolute.as_os_str().to_owned(),
        path: PathBuf::new(),
        expected: root_meta,
        depth: 0,
    };
    let scheduler = owner::Scheduler::new(options.threads, initial);
    let next_id = AtomicUsize::new(1);
    let count = AtomicUsize::new(1);
    let mut batches = std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(options.threads);
        for worker in 0..options.threads {
            let root = &root;
            let absolute = &absolute;
            let scheduler = &scheduler;
            let next_id = &next_id;
            let count = &count;
            workers.push(scope.spawn(move || {
                #[cfg(feature = "profile")]
                let _worker = crate::profile::Worker::start();
                let mut local = VecDeque::new();
                let mut cache = CapabilityCache::default();
                let mut batches = Vec::new();
                while let Some(job) = scheduler.acquire(worker, &mut local) {
                    let mut children = Vec::new();
                    let progress = options.progress.as_deref();
                    if let Some(progress) = progress {
                        progress.active.fetch_add(1, Ordering::Relaxed);
                    }
                    let mut observed_entries = 0u64;
                    let mut observed_files = 0u64;
                    let mut entries = Vec::new();
                    let mut errors = 0;
                    let mut own = job.expected;
                    let mut scanned_fd = None;
                    let opened = (|| {
                        if options.is_cancelled() {
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "scan cancelled",
                            ));
                        }
                        // Every component is opened independently with O_NOFOLLOW; queued path replacements cannot traverse a symlink.
                        let parent = job.parent.and_then(|id| {
                            if id == 0 {
                                Some(root)
                            } else {
                                cache.parent(id)
                            }
                        });
                        let fd = if let Some(parent) = parent {
                            #[cfg(feature = "profile")]
                            crate::profile::count(crate::profile::Event::CacheHit);
                            let name = CString::new(job.name.as_bytes()).map_err(|_| {
                                io::Error::new(io::ErrorKind::InvalidData, "invalid name")
                            })?;
                            native::open_at(parent.as_raw_fd(), &name)?
                        } else {
                            #[cfg(feature = "profile")]
                            {
                                crate::profile::count(crate::profile::Event::CacheMiss);
                                crate::profile::count(crate::profile::Event::RootReopen);
                            }
                            let mut fd = native::open_at(root.as_raw_fd(), c".")?;
                            for component in job.path.components() {
                                let Component::Normal(part) = component else {
                                    return Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        "non-child path",
                                    ));
                                };
                                let part = CString::new(part.as_bytes()).map_err(|_| {
                                    io::Error::new(io::ErrorKind::InvalidData, "invalid name")
                                })?;
                                #[cfg(feature = "profile")]
                                crate::profile::count(crate::profile::Event::FallbackOpen);
                                fd = native::open_at(fd.as_raw_fd(), &part)?;
                            }
                            fd
                        };
                        let m = native::stat_fd(fd.as_raw_fd())?;
                        if m.ino != job.expected.ino || m.dev != job.expected.dev {
                            return Err(io::Error::new(
                                io::ErrorKind::NotFound,
                                "directory changed during scan",
                            ));
                        }
                        own = m;
                        Ok(fd)
                    })();
                    match opened {
                        Err(_) => errors += 1,
                        Ok(fd) => {
                            let bulk = options.backend != Backend::Portable;
                            let delivered = std::cell::Cell::new(0usize);
                            let mut visit = |raw: native::Raw| {
                                if options.is_cancelled() {
                                    errors += 1;
                                    return false;
                                }
                                if options.filters.as_ref().is_some_and(|filters| {
                                    !filters.accepts(absolute, &job.path, &raw.name, raw.meta.kind)
                                }) {
                                    return true;
                                }
                                if count
                                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                                        if n < options.max_entries {
                                            Some(n + 1)
                                        } else {
                                            None
                                        }
                                    })
                                    .is_err()
                                {
                                    errors += 1;
                                    return false;
                                }
                                if let Some(progress) = progress {
                                    observed_entries += 1;
                                    observed_files += u64::from(raw.meta.kind != Kind::Directory);
                                    if observed_entries >= 256 {
                                        progress.observe(observed_entries, observed_files);
                                        observed_entries = 0;
                                        observed_files = 0;
                                    }
                                }
                                let mut entry = Entry {
                                    identity: Identity {
                                        dev: raw.meta.dev,
                                        ino: raw.meta.ino,
                                    },
                                    name: raw.name,
                                    kind: raw.meta.kind,
                                    allocated: raw.meta.allocated,
                                    apparent: raw.meta.apparent,
                                    child: None,
                                };
                                if raw.meta.kind == Kind::Directory {
                                    if (options.one_file_system && raw.meta.dev != root_meta.dev)
                                        || job.depth >= options.max_depth
                                    {
                                        if job.depth >= options.max_depth {
                                            errors += 1
                                        }
                                    } else {
                                        let id = next_id.fetch_add(1, Ordering::Relaxed);
                                        entry.child = Some(id);
                                        scheduler.register(1);
                                        children.push(Job {
                                            id,
                                            parent: Some(job.id),
                                            name: entry.name.clone(),
                                            path: job.path.join(&entry.name),
                                            expected: raw.meta,
                                            depth: job.depth + 1,
                                        });
                                    }
                                }
                                let hardlink = (raw.meta.kind == Kind::File && raw.meta.links > 1)
                                    .then_some((raw.meta.dev, raw.meta.ino));
                                entries.push((entry, hardlink));
                                delivered.set(delivered.get() + 1);
                                true
                            };
                            // Retry unsupported bulk enumeration on a fresh directory description,
                            // only before delivery: no double counting after a partial native result.
                            let mut result = native::enumerate(fd.as_raw_fd(), bulk, &mut visit);
                            if result.is_err()
                                && options.backend == Backend::Auto
                                && delivered.get() == 0
                            {
                                result = native::portable(fd.as_raw_fd(), &mut visit);
                            }
                            match result {
                                Ok(n) => errors += n,
                                Err(_) => errors += 1,
                            }
                            scanned_fd = Some(fd);
                        }
                    }
                    if let Some(progress) = progress {
                        progress.observe(observed_entries, observed_files);
                        progress
                            .discovered
                            .fetch_add(children.len() as u64, Ordering::Relaxed);
                        progress.errors.fetch_add(errors, Ordering::Relaxed);
                        progress.completed.fetch_add(1, Ordering::Relaxed);
                        progress.active.fetch_sub(1, Ordering::Relaxed);
                    }
                    // Parent zero already has the immutable pinned root capability. Other
                    // reused parents occupy at most two owner-only slots, never a queued job.
                    if !children.is_empty()
                        && job.id != 0
                        && let Some(fd) = scanned_fd.take()
                    {
                        cache.retain(job.id, fd);
                    }
                    drop(scanned_fd);
                    batches.push(Batch {
                        job,
                        entries,
                        own,
                        errors,
                    });
                    scheduler.finish(worker, &mut local, children);
                }
                batches
            }));
        }
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    if let Some(progress) = &options.progress {
        progress.accounting();
    }
    #[cfg(feature = "profile")]
    let _aggregate_timer = crate::profile::Timer::new(crate::profile::Event::Aggregation);
    batches.sort_unstable_by_key(|b| b.job.id);
    let mut dirs = Vec::with_capacity(batches.len());
    let mut seen = HashSet::new();
    let mut files = 0;
    let mut errors = 0;
    for batch in batches {
        let mut allocated = batch.own.allocated;
        let mut apparent = batch.own.apparent;
        errors += batch.errors;
        let entries = batch
            .entries
            .into_iter()
            .map(|(mut e, m)| {
                if e.kind != Kind::Directory {
                    files += 1
                }
                if m.is_some_and(|identity| !seen.insert(identity)) {
                    e.allocated = 0;
                    e.apparent = 0;
                }
                if e.child.is_none() {
                    allocated = allocated.saturating_add(e.allocated);
                    apparent = apparent.saturating_add(e.apparent);
                }
                e
            })
            .collect();
        dirs.push(Directory {
            identity: Identity {
                dev: batch.own.dev,
                ino: batch.own.ino,
            },
            name: batch.job.name,
            parent: batch.job.parent,
            entries,
            allocated,
            apparent,
            errors: batch.errors,
        });
    }
    // IDs are reserved after their parents, allowing a linear reverse accumulation without recursion.
    for id in (1..dirs.len()).rev() {
        let parent = dirs[id].parent.unwrap();
        dirs[parent].allocated = dirs[parent].allocated.saturating_add(dirs[id].allocated);
        dirs[parent].apparent = dirs[parent].apparent.saturating_add(dirs[id].apparent);
    }
    for id in 0..dirs.len() {
        let updates: Vec<_> = dirs[id]
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.child.map(|c| (i, dirs[c].allocated, dirs[c].apparent)))
            .collect();
        for (i, a, s) in updates {
            dirs[id].entries[i].allocated = a;
            dirs[id].entries[i].apparent = s;
        }
    }
    let cancelled = options.is_cancelled();
    if let Some(progress) = &options.progress {
        progress.finish(cancelled);
    }
    Ok(Tree {
        origin: TreeOrigin::Live,
        complete: errors == 0 && !cancelled,
        root: absolute,
        dirs,
        files,
        errors,
        elapsed: start.elapsed(),
    })
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    #[test]
    fn private_capability_cache_has_two_slots_and_recency() {
        let fixture = tempfile::tempdir().unwrap();
        let path = CString::new(fixture.path().as_os_str().as_bytes()).unwrap();
        let mut cache = CapabilityCache::default();
        for id in [1, 2] {
            cache.retain(id, native::open_at(libc::AT_FDCWD, &path).unwrap());
        }
        assert!(cache.parent(1).is_some());
        cache.retain(3, native::open_at(libc::AT_FDCWD, &path).unwrap());
        assert_eq!(cache.slots.len(), 2);
        assert!(cache.parent(2).is_none());
        assert!(cache.parent(1).is_some());
        assert!(cache.parent(3).is_some());
    }
}
