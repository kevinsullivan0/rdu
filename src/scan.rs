use crate::native::{self, Meta};
pub use crate::progress::{ScanPhase, ScanProgress, ScanSnapshot};
use std::{
    collections::{HashSet, VecDeque},
    ffi::{CString, OsString},
    io,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
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
    pinned_parent: Option<Arc<std::os::fd::OwnedFd>>,
}
struct Work {
    queue: VecDeque<Job>,
    pending: usize,
}
struct Batch {
    job: Job,
    entries: Vec<(Entry, Option<(u64, u64)>)>,
    own: Meta,
    errors: u64,
}

// Consume the current directory capability before entering shared completion.
// A leaf drops its final Arc here; one local child keeps a verified parent.
fn prepare_local(
    children: &mut Vec<Job>,
    scanned_fd: Option<Arc<std::os::fd::OwnedFd>>,
) -> (usize, Option<Job>) {
    let child_count = children.len();
    let local = children.pop().map(|mut child| {
        child.pinned_parent = scanned_fd;
        child
    });
    (child_count, local)
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
        pinned_parent: None,
    };
    let work = Mutex::new(Work {
        queue: VecDeque::from([initial]),
        pending: 1,
    });
    let ready = Condvar::new();
    let batches = Mutex::new(Vec::new());
    // Pin a bounded number of previously verified directories. Cached handles
    // refer to the originally observed directory even if it is later renamed.
    // Queued jobs own no descriptors, so width cannot exhaust the fd limit.
    let cache = Mutex::new(VecDeque::<(usize, Arc<std::os::fd::OwnedFd>)>::new());
    let next_id = AtomicUsize::new(1);
    let count = AtomicUsize::new(1);
    std::thread::scope(|scope| {
        for _ in 0..options.threads {
            let root = &root;
            let absolute = &absolute;
            let work = &work;
            let ready = &ready;
            let batches = &batches;
            let next_id = &next_id;
            let count = &count;
            let cache = &cache;
            scope.spawn(move || {
                #[cfg(feature = "profile")]
                let _worker = crate::profile::Worker::start();
                let mut local: Option<Job> = None;
                loop {
                    let mut job = if let Some(job) = local.take() {
                        job
                    } else {
                        let mut w = {
                            #[cfg(feature = "profile")]
                            let _timer =
                                crate::profile::Timer::new(crate::profile::Event::WorkLock);
                            work.lock().unwrap()
                        };
                        loop {
                            if let Some(j) = w.queue.pop_back() {
                                break j;
                            }
                            if w.pending == 0 {
                                return;
                            }
                            w = {
                                #[cfg(feature = "profile")]
                                let _timer =
                                    crate::profile::Timer::new(crate::profile::Event::Wait);
                                ready.wait(w).unwrap()
                            };
                        }
                    };
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
                    let parent_hint = job.pinned_parent.take();
                    let opened = (|| {
                        if options.is_cancelled() {
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "scan cancelled",
                            ));
                        }
                        // Every component is opened independently with O_NOFOLLOW; queued path replacements cannot traverse a symlink.
                        let parent_fd = parent_hint.or_else(|| {
                            job.parent.and_then(|id| {
                                #[cfg(feature = "profile")]
                                let _timer =
                                    crate::profile::Timer::new(crate::profile::Event::CacheLock);
                                let mut c = cache.lock().unwrap();
                                let index = c.iter().position(|(key, _)| *key == id)?;
                                let item = c.remove(index)?;
                                let fd = item.1.clone();
                                c.push_back(item);
                                Some(fd)
                            })
                        });
                        let fd = if let Some(parent) = parent_fd {
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
                        Ok(Arc::new(fd))
                    })();
                    match opened {
                        Err(_) => errors += 1,
                        Ok(fd) => {
                            scanned_fd = Some(fd.clone());
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
                                        children.push(Job {
                                            id,
                                            parent: Some(job.id),
                                            name: entry.name.clone(),
                                            path: job.path.join(&entry.name),
                                            expected: raw.meta,
                                            depth: job.depth + 1,
                                            pinned_parent: None,
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
                    // Only parents of discovered child jobs can be reused.
                    // Delay cache admission until enumeration identifies them;
                    // leaf directories must not evict useful parent handles.
                    if !children.is_empty()
                        && let Some(fd) = &scanned_fd
                    {
                        #[cfg(feature = "profile")]
                        let _timer = crate::profile::Timer::new(crate::profile::Event::CacheLock);
                        let mut c = cache.lock().unwrap();
                        c.push_back((job.id, fd.clone()));
                        while c.len() > options.threads * 2 {
                            c.pop_front();
                        }
                    }
                    {
                        #[cfg(feature = "profile")]
                        let _timer = crate::profile::Timer::new(crate::profile::Event::ResultLock);
                        batches.lock().unwrap().push(Batch {
                            job,
                            entries,
                            own,
                            errors,
                        });
                    }
                    let (child_count, continuation) = prepare_local(&mut children, scanned_fd);
                    local = continuation;
                    let mut w = {
                        #[cfg(feature = "profile")]
                        let _timer = crate::profile::Timer::new(crate::profile::Event::WorkLock);
                        work.lock().unwrap()
                    };
                    w.pending = w.pending - 1 + child_count;
                    // Shared queued jobs remain descriptor-free.
                    w.queue.extend(children);
                    ready.notify_all();
                }
            });
        }
    });
    if let Some(progress) = &options.progress {
        progress.accounting();
    }
    #[cfg(feature = "profile")]
    let _aggregate_timer = crate::profile::Timer::new(crate::profile::Event::Aggregation);
    let mut batches = batches.into_inner().unwrap();
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
    use std::os::fd::RawFd;

    #[test]
    fn continuation_preparation_releases_leaf_last_owner_and_preserves_child_count() {
        const CHILD: &str = "RDU_LEAF_CLOSE_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "scan::completion_tests::continuation_preparation_releases_leaf_last_owner_and_preserves_child_count", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        fn opened(path: &Path) -> (Arc<std::os::fd::OwnedFd>, RawFd) {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            let fd = Arc::new(native::open_at(libc::AT_FDCWD, &name).unwrap());
            let raw = fd.as_raw_fd();
            (fd, raw)
        }
        let fixture = tempfile::tempdir().unwrap();
        let (fd, raw) = opened(fixture.path());
        assert_eq!(Arc::strong_count(&fd), 1);
        let (count, local) = prepare_local(&mut Vec::new(), Some(fd));
        assert_eq!(count, 0);
        assert!(local.is_none());
        assert_eq!(
            native::stat_fd(raw).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );

        let (fd, raw) = opened(fixture.path());
        let expected = native::stat_fd(raw).unwrap();
        let child = |id| Job {
            id,
            parent: Some(0),
            name: id.to_string().into(),
            path: PathBuf::from(id.to_string()),
            expected,
            depth: 1,
            pinned_parent: None,
        };
        let mut children = vec![child(1), child(2)];
        let (count, local) = prepare_local(&mut children, Some(fd));
        assert_eq!(count, 2);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, 1);
        assert!(children[0].pinned_parent.is_none());
        let local = local.unwrap();
        assert_eq!(local.id, 2);
        assert_eq!(Arc::strong_count(local.pinned_parent.as_ref().unwrap()), 1);
        assert!(native::stat_fd(raw).is_ok());
        drop(local);
        assert_eq!(
            native::stat_fd(raw).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );
    }
}
