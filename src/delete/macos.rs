//! macOS implementation. All unlink calls use the sealed, immediate slot parent.
use super::*;
use crate::scan::{Identity, Kind};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, OsString},
    fs::File,
    io::Write,
    mem::{MaybeUninit, size_of},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::Component,
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_OBJECTS: usize = 1_000_000;
const MAX_MEMORY: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 1024;
const MAX_AGE: Duration = Duration::from_secs(600);
const MAX_DETAILS: usize = 4096;
const MAX_DETAIL_MEMORY: usize = 8 * 1024 * 1024;
type Result<T> = io::Result<T>;
fn refuse(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, s.into())
}
fn cvt(n: libc::c_int) -> Result<()> {
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn cname(s: &std::ffi::OsStr) -> Result<CString> {
    let b = s.as_bytes();
    if !crate::native::component(b) {
        return Err(refuse("invalid single path component"));
    }
    CString::new(b).map_err(|_| refuse("invalid path component"))
}
fn stat(fd: RawFd) -> Result<libc::stat> {
    let mut out = MaybeUninit::uninit();
    // SAFETY: borrowed descriptor and writable stat storage; initialized on success.
    cvt(unsafe { libc::fstat(fd, out.as_mut_ptr()) })?;
    Ok(unsafe { out.assume_init() })
}
fn stat_at(fd: RawFd, name: &CStr) -> Result<libc::stat> {
    let mut out = MaybeUninit::uninit();
    // SAFETY: valid component/buffer. Final symlinks are inspected, never followed.
    cvt(unsafe {
        libc::fstatat(
            fd,
            name.as_ptr(),
            out.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    Ok(unsafe { out.assume_init() })
}
fn identity(s: &libc::stat) -> Identity {
    Identity {
        dev: s.st_dev as u64,
        ino: s.st_ino,
    }
}
fn kind(s: &libc::stat) -> Kind {
    match s.st_mode & libc::S_IFMT {
        libc::S_IFREG => Kind::File,
        libc::S_IFDIR => Kind::Directory,
        libc::S_IFLNK => Kind::Symlink,
        _ => Kind::Other,
    }
}
fn open(fd: RawFd, name: &CStr, directory: bool) -> Result<OwnedFd> {
    // O_NOFOLLOW rejects even O_SYMLINK on Darwin. O_SYMLINK itself pins the final link;
    // metadata opens accept only single components after their parent was checked.
    let access = if directory {
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW
    } else {
        libc::O_EVTONLY | libc::O_SYMLINK | libc::O_NONBLOCK | libc::O_NOCTTY
    };
    // SAFETY: nondirectory callers establish source namespace privacy and inspect type before this call.
    // O_EVTONLY alone does not remove read rights or prevent a device driver's open routine.
    // Production never reads nondirectory contents; NONBLOCK/NOCTTY are additional defenses.
    let out = unsafe { libc::openat(fd, name.as_ptr(), access | libc::O_CLOEXEC) };
    if out < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(out) })
    }
}
fn duplicate(fd: RawFd) -> Result<OwnedFd> {
    open(fd, c".", true)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Stamp {
    id: Identity,
    kind: Kind,
    size: i64,
    mtime: (i64, i64),
    ctime: (i64, i64),
    birth: (i64, i64),
    links: u16,
    allocated: u64,
}
impl Stamp {
    fn from(s: &libc::stat) -> Self {
        Self {
            id: identity(s),
            kind: kind(s),
            size: s.st_size,
            mtime: (s.st_mtime, s.st_mtime_nsec),
            ctime: (s.st_ctime, s.st_ctime_nsec),
            birth: (s.st_birthtime, s.st_birthtime_nsec),
            links: s.st_nlink,
            allocated: (s.st_blocks.max(0) as u64).saturating_mul(512),
        }
    }
    fn same_object(self, other: Self) -> bool {
        self.id == other.id && self.kind == other.kind && self.birth == other.birth
    }
    fn same_version(self, other: Self) -> bool {
        self.same_object(other) && (self.kind == Kind::Directory || self == other)
    }
}
#[derive(Clone, Debug)]
struct Part {
    name: OsString,
    id: Identity,
}
#[derive(Debug)]
struct Root {
    path: PathBuf,
    fd: OwnedFd,
    id: Identity,
    chain: Vec<Part>,
    volume: Volume,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Volume {
    fsid: [i32; 2],
    mount: Vec<u8>,
}
fn volume_allowed(name: &[u8], flags: u32) -> bool {
    name == b"apfs"
        && flags & libc::MNT_LOCAL as u32 != 0
        && flags & (libc::MNT_IGNORE_OWNERSHIP | libc::MNT_RDONLY) as u32 == 0
}
fn volume(fd: RawFd) -> Result<Volume> {
    let mut s = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: fstatfs initializes the writable structure on success.
    cvt(unsafe { libc::fstatfs(fd, s.as_mut_ptr()) })?;
    let s = unsafe { s.assume_init() };
    let name = unsafe { CStr::from_ptr(s.f_fstypename.as_ptr()) };
    if !volume_allowed(name.to_bytes(), s.f_flags) {
        return Err(refuse(
            "deletion requires local writable APFS with ownership enabled",
        ));
    }
    // Darwin fsid_t is two int32 values; libc keeps the field private. Transmute checks size at compile time.
    Ok(Volume {
        fsid: unsafe { std::mem::transmute::<libc::fsid_t, [i32; 2]>(s.f_fsid) },
        mount: unsafe { CStr::from_ptr(s.f_mntonname.as_ptr()) }
            .to_bytes()
            .to_vec(),
    })
}
// libc intentionally does not expose the Darwin ACL API. Types follow SDK sys/acl.h.
unsafe extern "C" {
    fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut libc::c_void;
    fn acl_init(count: libc::c_int) -> *mut libc::c_void;
    fn acl_set_fd(fd: libc::c_int, acl: *mut libc::c_void) -> libc::c_int;
    fn acl_get_entry(
        acl: *mut libc::c_void,
        index: libc::c_int,
        entry: *mut *mut libc::c_void,
    ) -> libc::c_int;
    fn acl_get_tag_type(entry: *mut libc::c_void, tag: *mut libc::c_int) -> libc::c_int;
    fn acl_valid(acl: *mut libc::c_void) -> libc::c_int;
    fn acl_free(obj: *mut libc::c_void) -> libc::c_int;
}
struct Acl(*mut libc::c_void);
impl Drop for Acl {
    fn drop(&mut self) {
        unsafe { acl_free(self.0) };
    }
}
fn acl_safe(fd: RawFd, empty: bool) -> Result<()> {
    let a = Acl(unsafe { acl_get_fd_np(fd, 0x100) });
    if a.0.is_null() {
        let e = io::Error::last_os_error();
        // Darwin returns ENOENT when the live fd has no FILESEC_ACL property.
        // Corroborate the descriptor; EACCES/ENOTSUP/other failures remain fatal.
        if e.raw_os_error() == Some(libc::ENOENT) {
            stat(fd)?;
            return Ok(());
        }
        return Err(e);
    }
    cvt(unsafe { acl_valid(a.0) })?;
    let mut entry = std::ptr::null_mut();
    let mut index = 0;
    loop {
        let n = unsafe { acl_get_entry(a.0, index, &mut entry) };
        // Darwin returns zero for an entry and -1/EINVAL at the end (unlike Linux).
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINVAL) {
                break;
            }
            return Err(e);
        }
        let mut tag = 0;
        cvt(unsafe { acl_get_tag_type(entry, &mut tag) })?;
        // Deny entries cannot grant namespace access. Any allow entry is conservatively refused.
        if empty || tag != 2 {
            return Err(refuse(
                "namespace ACL contains an allow or unsupported entry",
            ));
        }
        index = -1;
    }
    Ok(())
}
fn trusted(fd: RawFd, anchor: bool) -> Result<()> {
    let s = stat(fd)?;
    let uid = unsafe { libc::geteuid() };
    let sticky_root = s.st_uid == 0 && s.st_mode & libc::S_ISVTX != 0;
    if kind(&s) != Kind::Directory
        || (s.st_uid != uid && s.st_uid != 0)
        || (s.st_mode & 0o022 != 0 && !sticky_root)
        || (anchor && s.st_uid != uid && !sticky_root)
    {
        return Err(refuse(
            "quarantine anchor/ancestor is not a trusted owned namespace",
        ));
    }
    acl_safe(fd, false)
}
fn source_namespace_mode_safe(directory: &libc::stat, object: &libc::stat, uid: u32) -> bool {
    let private = directory.st_uid == uid && directory.st_mode & 0o022 == 0;
    let sticky = (directory.st_uid == 0 || directory.st_uid == uid)
        && directory.st_mode & libc::S_ISVTX != 0
        && object.st_uid == uid;
    private || sticky
}
fn nondirectory_source_safe(parent: RawFd, object: &libc::stat) -> Result<()> {
    if !matches!(kind(object), Kind::File | Kind::Symlink) {
        return Err(refuse(
            "unsupported current object type; no nondirectory open attempted",
        ));
    }
    let directory = stat(parent)?;
    let uid = unsafe { libc::geteuid() };
    // No O_REGULAR/type-conditional open exists in the supported SDK. A pre-stat is safe only
    // where excluded writers cannot substitute a device before open. Retained directory fds
    // do not bypass current mode/ACL checks. Same-UID/root policy changes are outside the boundary.
    if !source_namespace_mode_safe(&directory, object, uid) {
        return Err(refuse(
            "cannot safely pin nondirectory in a foreign-writable source namespace; preserved without opening",
        ));
    }
    acl_safe(parent, false)
}
impl Root {
    fn acquire(path: &Path, expected: Identity) -> Result<Self> {
        let mut fd = open(libc::AT_FDCWD, c"/", true)?;
        trusted(fd.as_raw_fd(), false)?;
        let mut chain = Vec::new();
        for c in path.components() {
            match c {
                Component::RootDir => continue,
                Component::Normal(n) => {
                    if chain.len() >= MAX_DEPTH {
                        return Err(refuse("root path exceeds depth budget"));
                    }
                    let next = open(fd.as_raw_fd(), &cname(n)?, true)?;
                    trusted(next.as_raw_fd(), false)?;
                    chain.push(Part {
                        name: n.to_owned(),
                        id: identity(&stat(next.as_raw_fd())?),
                    });
                    fd = next;
                }
                _ => return Err(refuse("noncanonical root path")),
            }
        }
        if identity(&stat(fd.as_raw_fd())?) != expected {
            return Err(refuse("root changed while acquiring deletion authority"));
        }
        trusted(fd.as_raw_fd(), true)?;
        let volume = volume(fd.as_raw_fd())?;
        Ok(Self {
            path: path.to_owned(),
            fd,
            id: expected,
            chain,
            volume,
        })
    }
    fn validate(&self) -> Result<()> {
        self.validate_with_control(None)
    }
    fn validate_with_control(&self, control: Option<&PrepareControl>) -> Result<()> {
        if let Some(c) = control {
            c.check()?;
        }
        let mut fd = open(libc::AT_FDCWD, c"/", true)?;
        trusted(fd.as_raw_fd(), false)?;
        for part in &self.chain {
            if let Some(c) = control {
                c.check()?;
            }
            let next = open(fd.as_raw_fd(), &cname(&part.name)?, true)?;
            if identity(&stat(next.as_raw_fd())?) != part.id {
                return Err(refuse("root ancestor identity changed"));
            }
            trusted(next.as_raw_fd(), false)?;
            fd = next;
        }
        if identity(&stat(fd.as_raw_fd())?) != self.id
            || identity(&stat(self.fd.as_raw_fd())?) != self.id
        {
            return Err(refuse("root identity changed"));
        }
        trusted(fd.as_raw_fd(), true)?;
        if volume(fd.as_raw_fd())? != self.volume {
            return Err(refuse("root mount policy changed"));
        }
        Ok(())
    }
    fn check_mount(&self, fd: RawFd) -> Result<()> {
        if stat(fd)?.st_dev as u64 != self.id.dev || volume(fd)? != self.volume {
            return Err(refuse("mount boundary in deletion plan"));
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Binding {
    name: OsString,
    id: Identity,
    kind: Kind,
}
#[derive(Debug)]
struct Directory {
    name: OsString,
    parent: Option<usize>,
    id: Identity,
    entries: Vec<Binding>,
}
#[derive(Debug)]
pub(super) struct Session {
    root: Arc<Root>,
    dirs: Vec<Directory>,
}
pub(super) fn scan(
    path: &Path,
    options: &Options,
    read_only: bool,
) -> Result<(Tree, Option<DeletionSession>)> {
    if read_only {
        return Ok((crate::scan::scan(path, options)?, None));
    }
    // Normalize trailing separators/CurDir before refusing a final symlink.
    let normalized: PathBuf = path.components().collect();
    let raw = CString::new(normalized.as_os_str().as_bytes()).map_err(|_| refuse("NUL in root"))?;
    let pin = match open(libc::AT_FDCWD, &raw, true) {
        Ok(fd) => fd,
        Err(_) => return Ok((crate::scan::scan(path, options)?, None)),
    };
    let pin_id = identity(&stat(pin.as_raw_fd())?);
    let canonical = std::fs::canonicalize(&normalized)?;
    // Preserve the caller's path spelling for scan predicates and refresh. Canonicalization
    // supplies separate deletion authority/review paths, never a rewrite of filter input.
    let tree = crate::scan::scan(path, options)?;
    if unsafe { libc::geteuid() } == 0
        || unsafe { libc::getuid() } != unsafe { libc::geteuid() }
        || tree.errors != 0
        || !tree.complete
        || tree.dirs.is_empty()
        || tree.dirs[0].identity != pin_id
        || options.cancelled.is_some_and(|f| f())
        || options.progress.as_ref().is_some_and(|p| p.is_cancelled())
    {
        return Ok((tree, None));
    }
    let root = match Root::acquire(&canonical, pin_id) {
        Ok(r) => Arc::new(r),
        Err(_) => return Ok((tree, None)),
    };
    let dirs = tree
        .dirs
        .iter()
        .map(|d| Directory {
            name: d.name.clone(),
            parent: d.parent,
            id: d.identity,
            entries: d
                .entries
                .iter()
                .map(|e| Binding {
                    name: e.name.clone(),
                    id: e.identity,
                    kind: e.kind,
                })
                .collect(),
        })
        .collect();
    Ok((
        tree,
        Some(DeletionSession {
            inner: Session { root, dirs },
        }),
    ))
}
#[derive(Debug)]
struct Node {
    parent: usize,
    name: OsString,
    stamp: Stamp,
    selected: bool,
    depth: usize,
}
#[derive(Debug)]
pub(super) struct Plan {
    root: Arc<Root>,
    nodes: Vec<Node>,
    created: Instant,
    memory: usize,
    manifest: Vec<usize>,
    hardlinks: HashMap<Identity, Vec<usize>>,
}
impl Plan {
    pub(super) fn manifest_path(&self, index: usize) -> Option<PathBuf> {
        self.manifest.get(index).map(|i| self.path(*i))
    }
    pub(super) fn check_age(&self) -> Result<()> {
        if self.created.elapsed() > MAX_AGE {
            Err(refuse("deletion plan expired; prepare and review it again"))
        } else {
            Ok(())
        }
    }
    fn components(&self, mut i: usize) -> Vec<usize> {
        let mut v = Vec::new();
        while i != 0 {
            v.push(i);
            i = self.nodes[i].parent;
        }
        v.reverse();
        v
    }
    fn path(&self, i: usize) -> PathBuf {
        let mut p = self.root.path.clone();
        for n in self.components(i) {
            p.push(&self.nodes[n].name);
        }
        p
    }
    fn open_dir(&self, i: usize) -> Result<OwnedFd> {
        self.open_dir_with_control(i, None)
    }
    fn open_dir_with_control(&self, i: usize, control: Option<&PrepareControl>) -> Result<OwnedFd> {
        if let Some(c) = control {
            c.check()?;
        }
        let mut fd = duplicate(self.root.fd.as_raw_fd())?;
        for n in self.components(i) {
            if let Some(c) = control {
                c.check()?;
            }
            let next = open(fd.as_raw_fd(), &cname(&self.nodes[n].name)?, true)?;
            let now = Stamp::from(&stat(next.as_raw_fd())?);
            if !self.nodes[n].stamp.same_object(now) {
                return Err(refuse("source ancestor changed"));
            }
            self.root.check_mount(next.as_raw_fd())?;
            fd = next;
        }
        Ok(fd)
    }
    fn budget(&mut self, extra: usize) -> Result<()> {
        self.memory = self.memory.saturating_add(extra);
        let needed = self.nodes.len() + 1;
        let capacity = self.nodes.capacity().max(needed.next_power_of_two());
        let slack = capacity
            .saturating_sub(needed)
            .saturating_mul(size_of::<Node>());
        let indices = needed.saturating_mul(size_of::<usize>());
        if self.nodes.len() > MAX_OBJECTS
            || self.memory.saturating_add(slack).saturating_add(indices) > MAX_MEMORY
        {
            return Err(refuse(
                "deletion manifest exceeds 1,000,000 objects or 256 MiB; select a smaller batch",
            ));
        }
        Ok(())
    }
}
impl Session {
    pub(super) fn prepare(
        &self,
        targets: &[Target],
        control: &PrepareControl,
    ) -> Result<PreparedPlan> {
        self.prepare_inner(targets, control, &mut |_| {})
    }
    fn prepare_inner(
        &self,
        targets: &[Target],
        control: &PrepareControl,
        hook: &mut impl FnMut(&str),
    ) -> Result<PreparedPlan> {
        control.check()?;
        self.root.validate_with_control(Some(control))?;
        if targets.is_empty() {
            return Err(refuse("empty selection"));
        }
        if targets.len() > MAX_OBJECTS {
            return Err(refuse("selection exceeds manifest object budget"));
        }
        let mut selection_memory = targets.len().saturating_mul(320);
        if selection_memory > MAX_MEMORY {
            return Err(refuse("selection exceeds manifest memory budget"));
        }
        let mut unique = HashSet::new();
        let mut selected = Vec::new();
        for t in targets {
            control.check()?;
            if !unique.insert(*t) {
                continue;
            }
            let d = self
                .dirs
                .get(t.directory)
                .ok_or_else(|| refuse("invalid directory selection"))?;
            let e = d
                .entries
                .get(t.entry)
                .ok_or_else(|| refuse("invalid entry selection"))?;
            let mut ancestry = Vec::new();
            let mut i = t.directory;
            while i != 0 {
                control.check()?;
                if ancestry.len() >= MAX_DEPTH {
                    return Err(refuse("selection depth exceeds 1024"));
                }
                ancestry.push(i);
                i = self.dirs[i]
                    .parent
                    .ok_or_else(|| refuse("invalid scan ancestry"))?;
            }
            ancestry.reverse();
            let mut names: Vec<OsString> = ancestry
                .iter()
                .map(|i| self.dirs[*i].name.clone())
                .collect();
            names.push(e.name.clone());
            selection_memory = selection_memory
                .saturating_add(
                    names
                        .iter()
                        .map(|n| n.len() + 2 * size_of::<OsString>())
                        .sum::<usize>(),
                )
                .saturating_add(ancestry.capacity() * size_of::<usize>());
            if selection_memory > MAX_MEMORY {
                return Err(refuse("selection paths exceed manifest memory budget"));
            }
            selected.push((names, ancestry, t.directory, t.entry));
        }
        selected.sort_by(|a, b| a.0.cmp(&b.0));
        control.check()?;
        let mut roots: Vec<(Vec<OsString>, Vec<usize>, usize, usize)> = Vec::new();
        for s in selected {
            control.check()?;
            if !roots.last().is_some_and(|r| s.0.starts_with(&r.0)) {
                roots.push(s);
            }
        }
        let mut plan = Plan {
            root: self.root.clone(),
            nodes: vec![Node {
                parent: 0,
                name: OsString::new(),
                stamp: Stamp::from(&stat(self.root.fd.as_raw_fd())?),
                selected: false,
                depth: 0,
            }],
            created: Instant::now(),
            memory: size_of::<Node>() + selection_memory,
            manifest: Vec::new(),
            hardlinks: HashMap::new(),
        };
        let mut ancestors = HashMap::from([(0, 0)]);
        let mut target_nodes = Vec::new();
        for (_, chain, d, e) in &roots {
            control.check()?;
            let mut parent = 0;
            for id in chain {
                control.check()?;
                if let Some(n) = ancestors.get(id) {
                    parent = *n;
                    continue;
                }
                let binding = &self.dirs[*id];
                let fd = plan.open_dir_with_control(parent, Some(control))?;
                let child = open(fd.as_raw_fd(), &cname(&binding.name)?, true)?;
                let stamp = Stamp::from(&stat(child.as_raw_fd())?);
                if stamp.id != binding.id {
                    return Err(refuse("stale scanned ancestor"));
                }
                self.root.check_mount(child.as_raw_fd())?;
                let n = plan.nodes.len();
                plan.memory = plan.memory.saturating_add(128);
                plan.budget(binding.name.len() + size_of::<Node>())?;
                plan.nodes.push(Node {
                    parent,
                    name: binding.name.clone(),
                    stamp,
                    selected: false,
                    depth: plan.nodes[parent].depth + 1,
                });
                ancestors.insert(*id, n);
                parent = n;
            }
            let binding = &self.dirs[*d].entries[*e];
            let fd = plan.open_dir_with_control(parent, Some(control))?;
            let stamp = Stamp::from(&stat_at(fd.as_raw_fd(), &cname(&binding.name)?)?);
            if stamp.id != binding.id || stamp.kind != binding.kind {
                return Err(refuse("selected row changed; refresh before staging"));
            }
            if stamp.kind == Kind::Other {
                return Err(refuse("special nodes are not deletion candidates"));
            }
            plan.budget(binding.name.len() + size_of::<Node>())?;
            let n = plan.nodes.len();
            plan.nodes.push(Node {
                parent,
                name: binding.name.clone(),
                stamp,
                selected: true,
                depth: plan.nodes[parent].depth + 1,
            });
            target_nodes.push(n);
        }
        plan.memory = plan
            .memory
            .saturating_sub(selection_memory)
            .saturating_sub(ancestors.len().saturating_sub(1) * 128);
        drop(ancestors);
        drop(roots);
        drop(unique);
        // Iterative breadth-first enumeration. At most a root, current directory and DIR stream are open.
        let mut i = 1;
        while i < plan.nodes.len() {
            control.check()?;
            if plan.nodes[i].selected && plan.nodes[i].stamp.kind == Kind::Directory {
                if plan.nodes[i].depth > MAX_DEPTH {
                    return Err(refuse("deletion depth exceeds 1024"));
                }
                let fd = plan.open_dir_with_control(i, Some(control))?;
                let before = Stamp::from(&stat(fd.as_raw_fd())?);
                let mut failure = None;
                let errors = crate::native::portable(fd.as_raw_fd(), &mut |raw| {
                    let result = (|| {
                        hook("entry");
                        control.check()?;
                        if plan.nodes.len() > MAX_OBJECTS {
                            return Err(refuse("deletion manifest exceeds 1,000,000 objects"));
                        }
                        let stamp = Stamp::from(&stat_at(fd.as_raw_fd(), &cname(&raw.name)?)?);
                        if stamp.id.dev != self.root.id.dev || stamp.kind == Kind::Other {
                            return Err(refuse("mount or special node in selected closure"));
                        }
                        plan.budget(size_of::<Node>() + raw.name.len())?;
                        let depth = plan.nodes[i].depth + 1;
                        if depth > MAX_DEPTH {
                            return Err(refuse("deletion depth exceeds 1024"));
                        }
                        plan.nodes.push(Node {
                            parent: i,
                            name: raw.name,
                            stamp,
                            selected: true,
                            depth,
                        });
                        Ok(())
                    })();
                    if let Err(e) = result {
                        failure = Some(e);
                        false
                    } else {
                        true
                    }
                })?;
                if let Some(e) = failure {
                    return Err(e);
                }
                if errors != 0 || before != Stamp::from(&stat(fd.as_raw_fd())?) {
                    return Err(refuse(
                        "directory changed or could not be completely enumerated while staging",
                    ));
                }
            }
            i += 1;
        }
        self.root.validate_with_control(Some(control))?;
        for (i, n) in plan.nodes.iter().enumerate() {
            control.check()?;
            if n.selected {
                plan.manifest.push(i);
            }
        }
        for &i in &plan.manifest {
            control.check()?;
            let n = &plan.nodes[i];
            if n.stamp.kind == Kind::File && n.stamp.links > 1 {
                let extra = 16
                    + if plan.hardlinks.contains_key(&n.stamp.id) {
                        0
                    } else {
                        128
                    };
                plan.memory = plan.memory.saturating_add(extra);
                if plan
                    .memory
                    .saturating_add(
                        plan.nodes.capacity().saturating_sub(plan.nodes.len()) * size_of::<Node>(),
                    )
                    .saturating_add(plan.manifest.capacity() * size_of::<usize>())
                    > MAX_MEMORY
                {
                    return Err(refuse(
                        "deletion hardlink index exceeds manifest memory budget",
                    ));
                }
                plan.hardlinks.entry(n.stamp.id).or_default().push(i);
            }
        }
        let count = plan.manifest.len();
        let mut seen = HashSet::new();
        let mut allocated_bytes = 0u64;
        for &i in &plan.manifest {
            control.check()?;
            let n = &plan.nodes[i];
            if n.stamp.kind != Kind::File || n.stamp.links <= 1 || seen.insert(n.stamp.id) {
                allocated_bytes = allocated_bytes.saturating_add(n.stamp.allocated);
            }
        }
        let mut paths = Vec::new();
        for &i in &target_nodes {
            control.check()?;
            let path = plan.path(i);
            plan.memory = plan
                .memory
                .saturating_add(path.capacity() + 2 * size_of::<PathBuf>());
            if plan
                .memory
                .saturating_add(
                    plan.nodes.capacity().saturating_sub(plan.nodes.len()) * size_of::<Node>(),
                )
                .saturating_add(plan.manifest.capacity() * size_of::<usize>())
                > MAX_MEMORY
            {
                return Err(refuse("review paths exceed manifest memory budget"));
            }
            paths.push(path);
        }
        let summary=PlanSummary{paths,target_count:target_nodes.len(),object_count:count,allocated_bytes,
            confirmation_phrase:format!("DELETE {}",target_nodes.len()),notes:vec![
                "Permanently removes the freshly reviewed complete subtree, including hidden/excluded entries. No secure erasure or reclaimed-space guarantee.".into(),
                "Plan expires 10 minutes after preparation. Fresh metadata versions are rechecked before each capture; inode reuse before a live pin is not universally distinguishable.".into(),
                "Concurrent changes can produce partial deletion or preserved recovery objects. Same-UID adversaries, root and untrusted filesystems are outside the isolation boundary.".into(),
                format!("Protected recovery storage will be created in {} only after confirmation.",self.root.path.display())]};
        control.check()?;
        plan.created = Instant::now();
        Ok(PreparedPlan {
            summary,
            inner: plan,
        })
    }
}

#[derive(Debug)]
struct Quarantine {
    fd: OwnedFd,
    name: CString,
    path: PathBuf,
    id: Identity,
    journal: File,
    record_buffer: Vec<u8>,
}
const MAX_JOURNAL_RECORD: usize = 64 * 1024;
struct BoundedRecord<'a>(&'a mut Vec<u8>);
impl Write for BoundedRecord<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length >= MAX_JOURNAL_RECORD)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "journal record exceeds 64 KiB",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
fn append_journal_record(
    writer: &mut impl Write,
    buffer: &mut Vec<u8>,
    value: &serde_json::Value,
) -> Result<()> {
    buffer.clear();
    serde_json::to_writer(BoundedRecord(buffer), value)?;
    buffer.push(b'\n');
    // One checked logical append, including newline. write_all still correctly handles
    // short/interrupted writes; a failed append is not retried as a new record.
    writer.write_all(buffer)
}
fn check_execution_cancelled(control: &DeleteControl) -> std::result::Result<(), SetupFailure> {
    if control.cancelled.load(Ordering::Acquire) {
        Err(SetupFailure {
            error: io::Error::new(
                io::ErrorKind::Interrupted,
                "deletion cancelled before capture",
            ),
            recovery: None,
            cancelled: true,
        })
    } else {
        Ok(())
    }
}
struct SetupFailure {
    error: io::Error,
    recovery: Option<PathBuf>,
    cancelled: bool,
}
impl From<io::Error> for SetupFailure {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            recovery: None,
            cancelled: false,
        }
    }
}
fn random_name(prefix: &str) -> CString {
    let mut b = [0u8; 16];
    unsafe { libc::arc4random_buf(b.as_mut_ptr().cast(), b.len()) };
    CString::new(format!("{prefix}{}", hex(&b))).unwrap()
}
fn hex(b: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 15) as usize] as char);
    }
    s
}
impl Quarantine {
    fn slot(i: usize) -> CString {
        CString::new(format!("object-{i:016x}")).unwrap()
    }
    fn create(
        plan: &Plan,
        control: &DeleteControl,
        hook: &mut impl FnMut(&str, usize, &Plan, &mut Quarantine),
    ) -> std::result::Result<Self, SetupFailure> {
        check_execution_cancelled(control)?;
        plan.root.validate()?;
        check_execution_cancelled(control)?;
        let name = random_name(".rdu-recovery-");
        // SAFETY: fresh random component; mkdir never reuses an existing directory.
        cvt(unsafe { libc::mkdirat(plan.root.fd.as_raw_fd(), name.as_ptr(), 0o700) })?;
        let recovery = plan
            .root
            .path
            .join(OsString::from_vec(name.to_bytes().to_vec()));
        Self::setup_created(plan, name, control, hook).map_err(|mut failure| {
            failure.recovery = Some(recovery);
            failure
        })
    }
    fn setup_created(
        plan: &Plan,
        name: CString,
        control: &DeleteControl,
        hook: &mut impl FnMut(&str, usize, &Plan, &mut Quarantine),
    ) -> std::result::Result<Self, SetupFailure> {
        let fd = open(plan.root.fd.as_raw_fd(), &name, true)?;
        let s = stat(fd.as_raw_fd())?;
        if s.st_uid != unsafe { libc::geteuid() } || kind(&s) != Kind::Directory {
            return Err(refuse("quarantine ownership verification failed").into());
        }
        // ACL reset is confined to this newly created owned quarantine, never user objects.
        let acl = Acl(unsafe { acl_init(0) });
        if acl.0.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        cvt(unsafe { acl_set_fd(fd.as_raw_fd(), acl.0) })?;
        cvt(unsafe { libc::fchmod(fd.as_raw_fd(), 0o700) })?;
        let s = stat(fd.as_raw_fd())?;
        if s.st_uid != unsafe { libc::geteuid() } || s.st_mode & 0o7777 != 0o700 {
            return Err(refuse("quarantine mode verification failed").into());
        }
        acl_safe(fd.as_raw_fd(), true)?;
        plan.root.check_mount(fd.as_raw_fd())?;
        let mut found = false;
        let errors = crate::native::portable(fd.as_raw_fd(), &mut |_| {
            found = true;
            false
        })?;
        if found || errors != 0 {
            return Err(refuse(
                "quarantine was populated before privacy was established; preserve it",
            )
            .into());
        }
        let journal_fd = unsafe {
            libc::openat(
                fd.as_raw_fd(),
                c"journal.jsonl".as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if journal_fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let path = plan
            .root
            .path
            .join(OsString::from_vec(name.to_bytes().to_vec()));
        let mut q = Self {
            fd,
            name,
            path,
            id: identity(&s),
            journal: unsafe { File::from_raw_fd(journal_fd) },
            record_buffer: Vec::with_capacity(4096),
        };
        q.record(serde_json::json!({"rdu_journal":1,"authority":"none after restart","root_hex":hex(plan.root.path.as_os_str().as_bytes()),"root_dev":plan.root.id.dev,"root_ino":plan.root.id.ino,"quarantine_dev":q.id.dev,"quarantine_ino":q.id.ino}))?;
        // Every possible capture has a durable predeclared source/slot intent before the first move.
        // Slot components are opaque node numbers under this operation's unpredictable fresh parent.
        let mut written_intents = 0;
        for (i, n) in plan.nodes.iter().enumerate().skip(1) {
            q.cancel_setup_if_requested(control, written_intents)?;
            q.record_unflushed(serde_json::json!({"node":i,"parent":n.parent,"name_hex":hex(n.name.as_bytes()),"dev":n.stamp.id.dev,"ino":n.stamp.id.ino,"selected":n.selected,"slot":if n.selected{Some(Self::slot(i).to_str().unwrap().to_owned())}else{None},"kind":format!("{:?}",n.stamp.kind)}))?;
            written_intents += 1;
            hook("intent_written", i, plan, &mut q);
        }
        q.cancel_setup_if_requested(control, written_intents)?;
        q.journal.sync_all()?;
        cvt(unsafe { libc::fsync(q.fd.as_raw_fd()) })?;
        cvt(unsafe { libc::fsync(plan.root.fd.as_raw_fd()) })?;
        q.cancel_setup_if_requested(control, written_intents)?;
        Ok(q)
    }
    fn validate(&self, plan: &Plan) -> Result<()> {
        plan.root.validate()?;
        let s = stat(self.fd.as_raw_fd())?;
        if identity(&s) != self.id
            || s.st_uid != unsafe { libc::geteuid() }
            || s.st_mode & 0o7777 != 0o700
            || identity(&stat_at(plan.root.fd.as_raw_fd(), &self.name)?) != self.id
        {
            return Err(refuse("quarantine identity/privacy changed"));
        }
        acl_safe(self.fd.as_raw_fd(), true)
    }
    fn record_unflushed(&mut self, value: serde_json::Value) -> Result<()> {
        append_journal_record(&mut self.journal, &mut self.record_buffer, &value)
    }
    fn cancel_setup_if_requested(
        &mut self,
        control: &DeleteControl,
        written_intents: usize,
    ) -> std::result::Result<(), SetupFailure> {
        if control.cancelled.load(Ordering::Acquire) {
            self.record(serde_json::json!({"setup_cancelled":true,"written_intents":written_intents,"captures":0}))?;
            self.journal.sync_all()?;
            return check_execution_cancelled(control);
        }
        Ok(())
    }
    // Result records are hints, not a transaction. All capture intents were fsynced together above.
    fn record(&mut self, value: serde_json::Value) -> Result<()> {
        self.record_unflushed(value)
    }
    fn cleanup(self, plan: &Plan) -> Result<()> {
        self.validate(plan)?;
        // Refuse cleanup on any unexpected entry. No recursive cleanup, even of our own prefix.
        let mut unexpected = false;
        let errors = crate::native::portable(self.fd.as_raw_fd(), &mut |r| {
            if r.name != "journal.jsonl" {
                unexpected = true;
            }
            true
        })?;
        if errors != 0 || unexpected {
            return Err(refuse("recovery contains retained or unexpected objects"));
        }
        let journal_id = identity(&stat(self.journal.as_raw_fd())?);
        if identity(&stat_at(self.fd.as_raw_fd(), c"journal.jsonl")?) != journal_id {
            return Err(refuse("journal identity changed"));
        }
        // SAFETY: protected immediate namespace, validated known journal identity; no excluded writer has access.
        cvt(unsafe { libc::unlinkat(self.fd.as_raw_fd(), c"journal.jsonl".as_ptr(), 0) })?;
        self.validate(plan)?;
        // Anchor is trusted and remains subject to the documented same-UID exclusion.
        cvt(unsafe {
            libc::unlinkat(
                plan.root.fd.as_raw_fd(),
                self.name.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        })
    }
}
fn rename_exclusive(from: RawFd, name: &CStr, to: RawFd, slot: &CStr) -> Result<()> {
    // SAFETY: validated components, live descriptors. Never fall back to overwrite/copy-and-delete.
    cvt(unsafe { libc::renameatx_np(from, name.as_ptr(), to, slot.as_ptr(), libc::RENAME_EXCL) })
}
pub(super) fn execute(prepared: PreparedPlan, control: &DeleteControl) -> DeleteReport {
    execute_inner(prepared, control, &mut |_, _, _, _| {})
}
fn execute_inner(
    prepared: PreparedPlan,
    control: &DeleteControl,
    hook: &mut impl FnMut(&str, usize, &Plan, &mut Quarantine),
) -> DeleteReport {
    execute_with_details(prepared, control, hook, &mut |plan, index| plan.path(index))
}
fn execute_with_details(
    prepared: PreparedPlan,
    control: &DeleteControl,
    hook: &mut impl FnMut(&str, usize, &Plan, &mut Quarantine),
    describe: &mut impl FnMut(&Plan, usize) -> PathBuf,
) -> DeleteReport {
    let mut plan = prepared.inner;
    let total = plan.manifest.len();
    control.total.store(total, Ordering::Relaxed);
    let mut report = DeleteReport {
        total,
        ..Default::default()
    };
    let mut detail_memory = 0usize;
    let mut q: Option<Quarantine> = None;
    let initial = plan.check_age().and_then(|_| plan.root.validate());
    if let Err(e) = initial {
        report.error = Some(e.to_string());
    }
    if control.cancelled.load(Ordering::Acquire) {
        report.cancelled = true;
    }
    if report.error.is_none() && !report.cancelled {
        match Quarantine::create(&plan, control, hook) {
            Ok(created) => {
                report.recovery = Some(created.path.clone());
                q = Some(created)
            }
            Err(e) => {
                if e.cancelled {
                    report.cancelled = true;
                } else {
                    report.error = Some(format!(
                        "quarantine setup failed before admission: {}",
                        e.error
                    ));
                }
                report.recovery = e.recovery;
            }
        }
    }
    // Reverse parent-before-child insertion order is a complete postorder, without recursive stack/FD growth.
    for position in (0..plan.manifest.len()).rev() {
        let i = plan.manifest[position];
        if control.cancelled.load(Ordering::Acquire) {
            report.cancelled = true;
        }
        if report.error.is_some() || report.cancelled {
            // The unprocessed prefix is known from the immutable manifest index. Account it
            // once; generating every hidden full path can otherwise dominate cancellation.
            let remaining = position + 1;
            report.not_attempted += remaining;
            control.completed.fetch_add(remaining, Ordering::Relaxed);
            let mut visible = 0;
            let slots = MAX_DETAILS.saturating_sub(report.outcomes.len());
            for next in (0..=position).rev().take(slots) {
                if detail_memory >= MAX_DETAIL_MEMORY {
                    break;
                }
                let path = describe(&plan, plan.manifest[next]);
                let detail = "stopped before capture".to_owned();
                let size = size_of::<ObjectOutcome>() + path.as_os_str().len() + detail.len();
                if detail_memory.saturating_add(size) > MAX_DETAIL_MEMORY {
                    break;
                }
                detail_memory += size;
                visible += 1;
                report.outcomes.push(ObjectOutcome {
                    path,
                    state: OutcomeState::NotAttempted,
                    detail,
                });
            }
            report.details_truncated |= visible < remaining;
            if let Some(q)=q.as_mut()
                && let Err(e)=q.record(serde_json::json!({"stopped_before_manifest_position":position,"next_node":i,"remaining_not_attempted":remaining,"cancelled":report.cancelled,"error":report.error})) {
                let previous=report.error.take().unwrap_or_else(||"execution cancelled".into());
                report.error=Some(format!("{previous}; stop-boundary journal append failed: {e}"));
            }
            break;
        }
        let describe_outcome =
            report.outcomes.len() < MAX_DETAILS && detail_memory < MAX_DETAIL_MEMORY;
        let path = if describe_outcome {
            describe(&plan, i)
        } else {
            PathBuf::new()
        };
        let mut outcome = {
            let quarantine = q.as_mut().expect("executor has private quarantine");
            match process(&mut plan, i, quarantine, control, hook) {
                Ok((state, detail)) => ObjectOutcome {
                    path,
                    state,
                    detail,
                },
                Err(e) => {
                    report.error = Some(e.to_string());
                    ObjectOutcome {
                        path,
                        state: OutcomeState::Uncertain,
                        detail: format!("execution stopped; inspect journal: {e}"),
                    }
                }
            }
        };
        if report.error.is_none() && !report.cancelled
            && let Some(q)=q.as_mut()
            && let Err(e)=q.record(serde_json::json!({"outcome":i,"state":format!("{:?}",outcome.state),"detail":outcome.detail})) {
                    report.error=Some(format!("result journal append failed; recovery state requires reconciliation: {e}"));
                    // A successful unlink is still known successful in this process, but no complete durable audit is promised.
                    if outcome.state!=OutcomeState::Purged {outcome.state=OutcomeState::Uncertain;}
        }
        match outcome.state {
            OutcomeState::Purged => report.purged += 1,
            OutcomeState::Preserved | OutcomeState::Restored => report.preserved += 1,
            OutcomeState::NotAttempted => report.not_attempted += 1,
            OutcomeState::Uncertain => report.uncertain += 1,
        }
        if outcome.state == OutcomeState::Purged {
            control.purged.fetch_add(1, Ordering::Relaxed);
        } else if matches!(
            outcome.state,
            OutcomeState::Preserved | OutcomeState::Restored | OutcomeState::Uncertain
        ) {
            control.preserved.fetch_add(1, Ordering::Relaxed);
        }
        control.completed.fetch_add(1, Ordering::Relaxed);
        let size =
            size_of::<ObjectOutcome>() + outcome.path.as_os_str().len() + outcome.detail.len();
        if describe_outcome && detail_memory.saturating_add(size) <= MAX_DETAIL_MEMORY {
            detail_memory += size;
            report.outcomes.push(outcome);
        } else {
            report.details_truncated = true;
        }
    }
    report.cancelled |= control.cancelled.load(Ordering::Acquire);
    if let Some(mut q) = q {
        if let Err(e)=q.record(serde_json::json!({"finished":true,"total":report.total,"purged":report.purged,"preserved":report.preserved,"not_attempted":report.not_attempted,"uncertain":report.uncertain,"cancelled":report.cancelled,"details_truncated":report.details_truncated,"error":report.error})) {
            let previous=report.error.take().map(|s|format!("{s}; ")).unwrap_or_default();
            report.error=Some(format!("{previous}journal summary append failed: {e}"));
        }
        if let Err(e) = q.journal.sync_all() {
            report.error = Some(format!(
                "journal result flush failed; reconcile recovery before further action: {e}"
            ));
        }
        let all_purged =
            report.error.is_none() && !report.details_truncated && report.purged == report.total;
        if all_purged {
            match q.cleanup(&plan) {
                Ok(()) => report.recovery = None,
                Err(e) => {
                    report.error = Some(format!("objects removed; recovery cleanup retained: {e}"))
                }
            }
        }
    }
    report
}
fn process(
    plan: &mut Plan,
    i: usize,
    q: &mut Quarantine,
    control: &DeleteControl,
    hook: &mut impl FnMut(&str, usize, &Plan, &mut Quarantine),
) -> Result<(OutcomeState, String)> {
    q.validate(plan)?;
    let parent = plan.open_dir(plan.nodes[i].parent)?;
    let name = cname(&plan.nodes[i].name)?;
    let expected = plan.nodes[i].stamp;
    if expected.kind != Kind::Directory {
        let current = match stat_at(parent.as_raw_fd(), &name) {
            Ok(s) => s,
            Err(e) => {
                return Ok((
                    OutcomeState::NotAttempted,
                    format!("cannot inspect current object: {e}"),
                ));
            }
        };
        if !expected.same_version(Stamp::from(&current)) {
            return Ok((
                OutcomeState::NotAttempted,
                "metadata version/type changed before pin; preserved without opening".into(),
            ));
        }
        if let Err(e) = nondirectory_source_safe(parent.as_raw_fd(), &current) {
            return Ok((OutcomeState::NotAttempted, e.to_string()));
        }
    }
    let pinned = match open(parent.as_raw_fd(), &name, expected.kind == Kind::Directory) {
        Ok(f) => f,
        Err(e) => {
            return Ok((
                OutcomeState::NotAttempted,
                format!("cannot pin current object: {e}"),
            ));
        }
    };
    let before = Stamp::from(&stat(pinned.as_raw_fd())?);
    if !expected.same_version(before) {
        return Ok((
            OutcomeState::NotAttempted,
            "metadata version changed since review; preserved".into(),
        ));
    }
    plan.root.check_mount(pinned.as_raw_fd())?;
    let slot = Quarantine::slot(i);
    q.record(serde_json::json!({"intent":i,"slot":slot.to_str().unwrap(),"expected_dev":before.id.dev,"expected_ino":before.id.ino}))?;
    hook("before_capture", i, plan, q);
    if control.cancelled.load(Ordering::Acquire) {
        return Ok((
            OutcomeState::NotAttempted,
            "cancelled before capture".into(),
        ));
    }
    if let Err(e) = rename_exclusive(parent.as_raw_fd(), &name, q.fd.as_raw_fd(), &slot) {
        q.record(serde_json::json!({"capture_failed":i,"error":e.to_string()}))?;
        return Ok((
            OutcomeState::NotAttempted,
            format!("no-overwrite capture failed: {e}"),
        ));
    }
    hook("after_capture", i, plan, q);
    let captured = Stamp::from(&stat_at(q.fd.as_raw_fd(), &slot)?);
    let live = Stamp::from(&stat(pinned.as_raw_fd())?);
    if !before.same_object(captured) || !live.same_object(captured) {
        q.record(serde_json::json!({"mismatch_preserved":i,"slot":slot.to_str().unwrap()}))?;
        let restored = rename_exclusive(q.fd.as_raw_fd(), &slot, parent.as_raw_fd(), &name).is_ok();
        q.record(serde_json::json!({"restored":restored,"node":i}))?;
        return Ok((
            if restored {
                OutcomeState::Restored
            } else {
                OutcomeState::Preserved
            },
            "captured replacement was never purged; restored without overwrite where possible"
                .into(),
        ));
    }
    q.record(serde_json::json!({"captured_verified":i,"slot":slot.to_str().unwrap()}))?;
    // Recheck source ancestry after capture. A concurrent ancestor move is not an atomic pathname transaction.
    let containment = q
        .validate(plan)
        .and_then(|_| plan.open_dir(plan.nodes[i].parent).map(|_| ()));
    if let Err(e) = containment {
        q.record(serde_json::json!({"containment_failed":i,"error":e.to_string()}))?;
        return Ok((
            OutcomeState::Preserved,
            format!("captured object retained after ancestor change: {e}"),
        ));
    }
    if control.cancelled.load(Ordering::Acquire) {
        let restored = rename_exclusive(q.fd.as_raw_fd(), &slot, parent.as_raw_fd(), &name).is_ok();
        q.record(serde_json::json!({"cancelled_after_capture":i,"restored":restored}))?;
        return Ok((
            if restored {
                OutcomeState::Restored
            } else {
                OutcomeState::Preserved
            },
            "cancelled after capture; object retained".into(),
        ));
    }
    hook("before_purge", i, plan, q);
    q.validate(plan)?;
    if control.cancelled.load(Ordering::Acquire) {
        q.record(serde_json::json!({"cancelled_before_purge":i,"slot":slot.to_str().unwrap()}))?;
        return Ok((
            OutcomeState::Preserved,
            "cancelled before purge; verified object retained in recovery".into(),
        ));
    }
    let flags = if expected.kind == Kind::Directory {
        libc::AT_REMOVEDIR
    } else {
        0
    };
    // SAFETY: only a verified pinned object at an opaque immediate slot in the sealed namespace.
    // Original subtree directory handles cannot replace this slot. A nonempty directory is preserved.
    let result = cvt(unsafe { libc::unlinkat(q.fd.as_raw_fd(), slot.as_ptr(), flags) });
    if let Err(e) = result {
        q.record(serde_json::json!({"purge_failed_preserved":i,"error":e.to_string()}))?;
        let restored = rename_exclusive(q.fd.as_raw_fd(), &slot, parent.as_raw_fd(), &name).is_ok();
        q.record(serde_json::json!({"restored":restored,"node":i}))?;
        return Ok((
            if restored {
                OutcomeState::Restored
            } else {
                OutcomeState::Preserved
            },
            format!("purge refused; no recursive fallback: {e}"),
        ));
    }
    // Only this retained live handle can supply the ctime/nlink change caused by our hardlink unlink.
    if expected.kind == Kind::File && before.links > 1 {
        let after = Stamp::from(&stat(pinned.as_raw_fd())?);
        if before.same_object(after)
            && before.mtime == after.mtime
            && before.size == after.size
            && let Some(indices) = plan.hardlinks.get(&expected.id)
        {
            for &index in indices {
                let n = &mut plan.nodes[index];
                if n.stamp == expected {
                    n.stamp.ctime = after.ctime;
                    n.stamp.links = after.links;
                }
            }
        }
    }
    q.record(serde_json::json!({"purged":i}))?;
    Ok((
        OutcomeState::Purged,
        "permanently removed verified protected slot; reclaimed bytes are not measured".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::ffi::OsStrExt};
    fn fixture() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("rdu-delete-disposable-race-")
            .tempdir_in("/tmp")
            .unwrap()
    }
    fn prepared(path: &Path, name: &str) -> PreparedPlan {
        let (tree, session) = DeletionSession::scan(path, &Options::default(), false).unwrap();
        if session.is_none() {
            panic!(
                "session unavailable: {:?}",
                Root::acquire(&std::fs::canonicalize(path).unwrap(), tree.dirs[0].identity)
            );
        }
        let entry = tree.dirs[0]
            .entries
            .iter()
            .position(|e| e.name == name)
            .unwrap();
        session
            .unwrap()
            .prepare(&[Target {
                directory: 0,
                entry,
            }])
            .unwrap()
    }
    #[test]
    fn replaced_between_live_pin_and_capture_is_restored_without_purge() {
        let d = fixture();
        fs::write(d.path().join("file"), b"original").unwrap();
        let p = prepared(d.path(), "file");
        let r = execute_inner(p, &DeleteControl::default(), &mut |boundary, _, _, _| {
            if boundary == "before_capture" {
                fs::rename(d.path().join("file"), d.path().join("saved")).unwrap();
                fs::write(d.path().join("file"), b"replacement").unwrap();
            }
        });
        assert!(
            r.outcomes.iter().all(|o| o.state == OutcomeState::Restored),
            "{r:?}"
        );
        assert_eq!(fs::read(d.path().join("file")).unwrap(), b"replacement");
        assert_eq!(fs::read(d.path().join("saved")).unwrap(), b"original");
    }
    #[test]
    fn retained_descendant_fd_cannot_replace_private_slot_and_new_child_survives() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        let raw = CString::new(d.path().join("selected").as_os_str().as_bytes()).unwrap();
        let retained = open(libc::AT_FDCWD, &raw, true).unwrap();
        let p = prepared(d.path(), "selected");
        let r = execute_inner(p, &DeleteControl::default(), &mut |boundary, _, _, _| {
            if boundary == "before_purge" {
                let fd = unsafe {
                    libc::openat(
                        retained.as_raw_fd(),
                        c"new".as_ptr(),
                        libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC,
                        0o600,
                    )
                };
                assert!(fd >= 0);
                drop(unsafe { OwnedFd::from_raw_fd(fd) });
            }
        });
        assert_eq!(r.outcomes[0].state, OutcomeState::Restored, "{r:?}");
        assert!(d.path().join("selected/new").exists());
    }
    #[test]
    fn cancel_at_final_purge_boundary_preserves_captured_object() {
        let d = fixture();
        fs::write(d.path().join("file"), b"keep").unwrap();
        let p = prepared(d.path(), "file");
        let c = DeleteControl::default();
        let r = execute_inner(p, &c, &mut |boundary, _, _, _| {
            if boundary == "before_purge" {
                c.cancel()
            }
        });
        assert!(
            r.outcomes.iter().all(|o| o.state != OutcomeState::Purged),
            "{r:?}"
        );
    }
    #[test]
    fn exclusive_rename_does_not_overwrite_destination() {
        let d = fixture();
        fs::write(d.path().join("source"), b"source").unwrap();
        fs::write(d.path().join("destination"), b"destination").unwrap();
        let name = CString::new(d.path().as_os_str().as_bytes()).unwrap();
        let fd = open(libc::AT_FDCWD, &name, true).unwrap();
        assert_eq!(
            rename_exclusive(fd.as_raw_fd(), c"source", fd.as_raw_fd(), c"destination")
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EEXIST)
        );
        assert_eq!(fs::read(d.path().join("source")).unwrap(), b"source");
        assert_eq!(
            fs::read(d.path().join("destination")).unwrap(),
            b"destination"
        );
    }
    #[test]
    fn expired_plan_cannot_confirm() {
        let d = fixture();
        fs::write(d.path().join("file"), b"keep").unwrap();
        let mut p = prepared(d.path(), "file");
        p.inner.created = Instant::now() - MAX_AGE - Duration::from_secs(1);
        let phrase = p.summary.confirmation_phrase.clone();
        assert!(p.confirm(&phrase).is_err());
    }
    #[test]
    fn recreated_source_prevents_restore_but_preserves_both_occupants() {
        let d = fixture();
        fs::write(d.path().join("file"), b"original").unwrap();
        let p = prepared(d.path(), "file");
        let r = execute_inner(p, &DeleteControl::default(), &mut |boundary, _, _, _| {
            if boundary == "before_capture" {
                fs::rename(d.path().join("file"), d.path().join("saved")).unwrap();
                fs::write(d.path().join("file"), b"replacement").unwrap();
            }
            if boundary == "after_capture" {
                fs::write(d.path().join("file"), b"new occupant").unwrap();
            }
        });
        assert_eq!(r.purged, 0);
        assert_eq!(r.preserved, 1);
        assert_eq!(fs::read(d.path().join("file")).unwrap(), b"new occupant");
        assert_eq!(fs::read(d.path().join("saved")).unwrap(), b"original");
        assert!(fs::read_dir(r.recovery.unwrap()).unwrap().any(|e| {
            let p = e.unwrap().path();
            p.file_name().unwrap() != "journal.jsonl" && fs::read(p).unwrap() == b"replacement"
        }));
    }
    #[test]
    fn ancestor_rename_after_capture_preserves_object_for_recovery() {
        let d = fixture();
        fs::create_dir(d.path().join("parent")).unwrap();
        fs::write(d.path().join("parent/file"), b"keep").unwrap();
        let (tree, s) = DeletionSession::scan(d.path(), &Options::default(), false).unwrap();
        let directory = tree.dirs[0]
            .entries
            .iter()
            .find(|e| e.name == "parent")
            .unwrap()
            .child
            .unwrap();
        let p = s
            .unwrap()
            .prepare(&[Target {
                directory,
                entry: 0,
            }])
            .unwrap();
        let r = execute_inner(p, &DeleteControl::default(), &mut |boundary, _, _, _| {
            if boundary == "after_capture" {
                fs::rename(d.path().join("parent"), d.path().join("moved")).unwrap()
            }
        });
        assert_eq!(r.purged, 0);
        assert_eq!(r.preserved, 1);
        assert!(r.recovery.is_some());
    }
    #[test]
    fn inherited_deny_acl_is_reset_only_on_owned_quarantine() {
        let d = fixture();
        fs::write(d.path().join("file"), b"delete").unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone deny delete,file_inherit,directory_inherit"])
                .arg(d.path())
                .status()
                .unwrap()
                .success()
        );
        let p = prepared(d.path(), "file");
        let mut observed = false;
        let r = execute_inner(p, &DeleteControl::default(), &mut |boundary, _, plan, _| {
            if boundary == "before_capture" {
                let q = fs::read_dir(d.path())
                    .unwrap()
                    .map(|e| e.unwrap())
                    .find(|e| e.file_name().as_bytes().starts_with(b".rdu-recovery-"))
                    .unwrap();
                let fd = open(
                    plan.root.fd.as_raw_fd(),
                    &cname(&q.file_name()).unwrap(),
                    true,
                )
                .unwrap();
                acl_safe(fd.as_raw_fd(), true).unwrap();
                observed = true;
            }
        });
        assert!(
            std::process::Command::new("/bin/chmod")
                .arg("-N")
                .arg(d.path())
                .status()
                .unwrap()
                .success()
        );
        assert!(observed);
        assert_eq!(r.purged, 1, "{r:?}");
        assert!(r.error.is_none(), "{r:?}");
    }
    #[test]
    fn unexpected_quarantine_occupant_is_never_cleaned_recursively() {
        let d = fixture();
        fs::write(d.path().join("file"), b"delete").unwrap();
        let p = prepared(d.path(), "file");
        let r = execute_inner(p, &DeleteControl::default(), &mut |boundary, _, _, _| {
            if boundary == "before_purge" {
                let q = fs::read_dir(d.path())
                    .unwrap()
                    .map(|e| e.unwrap())
                    .find(|e| e.file_name().as_bytes().starts_with(b".rdu-recovery-"))
                    .unwrap();
                fs::write(q.path().join("unexpected"), b"keep").unwrap();
            }
        });
        assert_eq!(r.purged, 1);
        assert!(r.error.is_some());
        assert_eq!(
            fs::read(r.recovery.unwrap().join("unexpected")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn interrupted_execution_leaves_preflushed_intents_and_captured_object() {
        let d = fixture();
        fs::write(d.path().join("file"), b"keep").unwrap();
        let p = prepared(d.path(), "file");
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            execute_inner(p, &DeleteControl::default(), &mut |boundary, _, _, _| {
                if boundary == "after_capture" {
                    panic!("disposable fixture crash boundary")
                }
            })
        }));
        assert!(interrupted.is_err());
        let q = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap())
            .find(|e| e.file_name().as_bytes().starts_with(b".rdu-recovery-"))
            .unwrap();
        let journal = fs::read_to_string(q.path().join("journal.jsonl")).unwrap();
        assert!(journal.contains("none after restart"));
        assert!(journal.contains("object-0000000000000001"));
        assert_eq!(
            fs::read(q.path().join("object-0000000000000001")).unwrap(),
            b"keep"
        );
        // A fresh scan displays recovery. There is no journal-import/resume/purge authority API.
        let tree = crate::scan::scan(d.path(), &Options::default()).unwrap();
        assert!(tree.complete);
        assert!(q.path().join("object-0000000000000001").exists());
    }
    #[test]
    fn concurrent_source_swaps_never_purge_unconfirmed_replacement() {
        use std::sync::atomic::AtomicBool;
        for _ in 0..24 {
            let d = fixture();
            fs::write(d.path().join("victim"), b"original").unwrap();
            fs::write(d.path().join("other"), b"sentinel").unwrap();
            let p = prepared(d.path(), "victim");
            let raw = CString::new(d.path().as_os_str().as_bytes()).unwrap();
            let fd = Arc::new(open(libc::AT_FDCWD, &raw, true).unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let worker_fd = fd.clone();
            let worker_stop = stop.clone();
            let worker = std::thread::spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    unsafe {
                        libc::renameatx_np(
                            worker_fd.as_raw_fd(),
                            c"victim".as_ptr(),
                            worker_fd.as_raw_fd(),
                            c"other".as_ptr(),
                            libc::RENAME_SWAP,
                        )
                    };
                    std::thread::yield_now();
                }
            });
            let _ = execute_inner(p, &DeleteControl::default(), &mut |_, _, _, _| {});
            stop.store(true, Ordering::Release);
            worker.join().unwrap();
            let mut sentinel = false;
            for entry in fs::read_dir(d.path()).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    sentinel |= fs::read(entry.path()).unwrap() == b"sentinel";
                } else {
                    for object in fs::read_dir(entry.path()).unwrap() {
                        let object = object.unwrap();
                        if object.file_name() != "journal.jsonl" {
                            sentinel |= fs::read(object.path()).unwrap() == b"sentinel";
                        }
                    }
                }
            }
            assert!(
                sentinel,
                "unconfirmed replacement must survive source swapping"
            );
        }
    }
    #[test]
    fn event_handles_pin_final_symlink_identity_and_are_nonblocking() {
        let d = fixture();
        fs::write(d.path().join("file"), b"fixture").unwrap();
        std::os::unix::fs::symlink("file", d.path().join("link")).unwrap();
        let root_name = CString::new(d.path().as_os_str().as_bytes()).unwrap();
        let root = open(libc::AT_FDCWD, &root_name, true).unwrap();
        for name in [c"file", c"link"] {
            let pin = open(root.as_raw_fd(), name, false).unwrap();
            assert_eq!(
                identity(&stat(pin.as_raw_fd()).unwrap()),
                identity(&stat_at(root.as_raw_fd(), name).unwrap())
            );
            let flags = unsafe { libc::fcntl(pin.as_raw_fd(), libc::F_GETFL) };
            assert!(flags >= 0);
            assert_ne!(flags & libc::O_NONBLOCK, 0);
        }
    }
    #[test]
    fn nondirectory_pins_refuse_foreign_mutable_source_namespace() {
        use std::os::unix::fs::PermissionsExt;
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        fs::write(d.path().join("selected/file"), b"keep").unwrap();
        fs::set_permissions(d.path().join("selected"), fs::Permissions::from_mode(0o777)).unwrap();
        let p = prepared(d.path(), "selected");
        let r = execute_inner(p, &DeleteControl::default(), &mut |_, _, _, _| {});
        assert_eq!(r.purged, 0, "{r:?}");
        assert_eq!(fs::read(d.path().join("selected/file")).unwrap(), b"keep");
    }
    #[test]
    fn cancellation_mid_enumeration_releases_prepare_without_writes() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        for i in 0..32 {
            fs::write(d.path().join(format!("selected/{i}")), b"keep").unwrap();
        }
        let (tree, s) = DeletionSession::scan(d.path(), &Options::default(), false).unwrap();
        let entry = tree.dirs[0]
            .entries
            .iter()
            .position(|e| e.name == "selected")
            .unwrap();
        let c = PrepareControl::default();
        let mut observed = 0;
        let error = s
            .unwrap()
            .inner
            .prepare_inner(
                &[Target {
                    directory: 0,
                    entry,
                }],
                &c,
                &mut |event| {
                    if event == "entry" {
                        observed += 1;
                        c.cancel();
                    }
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(observed, 1);
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(d.path().join("selected")).unwrap().count(), 32);
    }
    fn readonly_journal(q: &mut Quarantine) {
        let fd = unsafe {
            libc::openat(
                q.fd.as_raw_fd(),
                c"journal.jsonl".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(fd >= 0);
        q.journal = unsafe { File::from_raw_fd(fd) };
    }
    #[test]
    fn journal_append_failure_before_capture_moves_nothing() {
        let d = fixture();
        fs::write(d.path().join("file"), b"keep").unwrap();
        let p = prepared(d.path(), "file");
        let mut plan = p.inner;
        let mut q = Quarantine::create(&plan, &DeleteControl::default(), &mut |_, _, _, _| {})
            .unwrap_or_else(|e| panic!("{}", e.error));
        readonly_journal(&mut q);
        let error = process(
            &mut plan,
            1,
            &mut q,
            &DeleteControl::default(),
            &mut |_, _, _, _| {},
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EBADF));
        assert_eq!(fs::read(d.path().join("file")).unwrap(), b"keep");
        assert!(!q.path.join("object-0000000000000001").exists());
    }
    #[test]
    fn journal_append_failure_after_capture_preserves_and_stops() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        fs::write(d.path().join("selected/file"), b"keep").unwrap();
        let p = prepared(d.path(), "selected");
        let r = execute_inner(p, &DeleteControl::default(), &mut |event, _, _, q| {
            if event == "after_capture" {
                readonly_journal(q)
            }
        });
        assert!(r.error.is_some());
        assert_eq!(r.purged, 0);
        assert_eq!(r.uncertain, 1);
        assert_eq!(r.not_attempted, 1);
        let recovery = r.recovery.unwrap();
        assert!(recovery.join("journal.jsonl").exists());
        assert_eq!(
            fs::read(recovery.join("object-0000000000000002")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn final_journal_flush_failure_keeps_recovery_and_reports_known_purge() {
        let d = fixture();
        fs::write(d.path().join("file"), b"delete").unwrap();
        let p = prepared(d.path(), "file");
        let mut pair = [-1; 2];
        assert_eq!(unsafe { libc::pipe(pair.as_mut_ptr()) }, 0);
        let reader = unsafe { OwnedFd::from_raw_fd(pair[0]) };
        let mut writer = Some(unsafe { File::from_raw_fd(pair[1]) });
        let r = execute_inner(p, &DeleteControl::default(), &mut |event, _, _, q| {
            if event == "before_purge" {
                q.journal = writer.take().unwrap()
            }
        });
        drop(reader);
        assert!(r.error.as_ref().unwrap().contains("flush failed"));
        assert_eq!(r.purged, 1);
        assert_eq!(r.uncertain, 0);
        assert!(!d.path().join("file").exists());
        assert!(r.recovery.unwrap().join("journal.jsonl").exists());
    }
    #[test]
    fn fifo_substitution_is_preserved_without_a_nondirectory_open() {
        let d = fixture();
        fs::write(d.path().join("file"), b"original").unwrap();
        let p = prepared(d.path(), "file");
        fs::rename(d.path().join("file"), d.path().join("saved")).unwrap();
        let raw = CString::new(d.path().join("file").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        let r = execute_inner(p, &DeleteControl::default(), &mut |_, _, _, _| {});
        assert_eq!(r.purged, 0);
        assert_eq!(r.not_attempted, 1);
        assert_eq!(
            kind(
                &stat_at(
                    open(
                        libc::AT_FDCWD,
                        &CString::new(d.path().as_os_str().as_bytes()).unwrap(),
                        true
                    )
                    .unwrap()
                    .as_raw_fd(),
                    c"file"
                )
                .unwrap()
            ),
            Kind::Other
        );
        assert_eq!(fs::read(d.path().join("saved")).unwrap(), b"original");
    }
    #[test]
    fn source_parent_policy_rejects_foreign_modes_and_device_shaped_metadata() {
        let d = fixture();
        fs::write(d.path().join("file"), b"keep").unwrap();
        let raw = CString::new(d.path().as_os_str().as_bytes()).unwrap();
        let fd = open(libc::AT_FDCWD, &raw, true).unwrap();
        let directory = stat(fd.as_raw_fd()).unwrap();
        let mut object = stat_at(fd.as_raw_fd(), c"file").unwrap();
        let uid = unsafe { libc::geteuid() };
        let mut foreign = directory;
        foreign.st_uid = uid.wrapping_add(1);
        assert!(!source_namespace_mode_safe(&foreign, &object, uid));
        foreign.st_uid = 0;
        foreign.st_mode = libc::S_IFDIR | 0o1777;
        assert!(source_namespace_mode_safe(&foreign, &object, uid));
        object.st_uid = uid.wrapping_add(1);
        assert!(!source_namespace_mode_safe(&foreign, &object, uid));
        for node_type in [libc::S_IFCHR, libc::S_IFBLK, libc::S_IFIFO, libc::S_IFSOCK] {
            object.st_mode = node_type | 0o600;
            assert!(nondirectory_source_safe(fd.as_raw_fd(), &object).is_err());
        }
    }
    #[test]
    fn allow_acl_in_source_folder_is_unprovable_and_retained() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        fs::write(d.path().join("selected/file"), b"keep").unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone allow write,delete_child"])
                .arg(d.path().join("selected"))
                .status()
                .unwrap()
                .success()
        );
        let p = prepared(d.path(), "selected");
        let r = execute_inner(p, &DeleteControl::default(), &mut |_, _, _, _| {});
        assert!(
            std::process::Command::new("/bin/chmod")
                .arg("-N")
                .arg(d.path().join("selected"))
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(r.purged, 0);
        assert_eq!(fs::read(d.path().join("selected/file")).unwrap(), b"keep");
    }
    #[test]
    fn mount_policy_refuses_remote_readonly_ownership_ignored_and_unknown_filesystems() {
        let local = libc::MNT_LOCAL as u32;
        assert!(volume_allowed(b"apfs", local));
        assert!(!volume_allowed(b"apfs", 0));
        assert!(!volume_allowed(
            b"apfs",
            local | libc::MNT_IGNORE_OWNERSHIP as u32
        ));
        assert!(!volume_allowed(b"apfs", local | libc::MNT_RDONLY as u32));
        for name in [b"nfs".as_slice(), b"fuse", b"hfs", b"unknown"] {
            assert!(!volume_allowed(name, local));
        }
    }
    #[test]
    fn stopped_large_manifest_describes_only_bounded_visible_outcomes() {
        for cancelled in [true, false] {
            let d = fixture();
            fs::write(d.path().join("file"), b"keep").unwrap();
            let mut p = prepared(d.path(), "file");
            let stamp = p.inner.nodes[1].stamp;
            const COUNT: usize = 100_000;
            for i in 2..=COUNT {
                p.inner.nodes.push(Node {
                    parent: 0,
                    name: format!("synthetic-{i}").into(),
                    stamp,
                    selected: true,
                    depth: 1,
                });
            }
            p.inner.manifest = (1..=COUNT).collect();
            p.summary.object_count = COUNT;
            let c = DeleteControl::default();
            if cancelled {
                c.cancel();
            } else {
                p.inner.created = Instant::now() - MAX_AGE - Duration::from_secs(1);
            }
            let mut described = 0;
            let r = execute_with_details(p, &c, &mut |_, _, _, _| {}, &mut |plan, index| {
                described += 1;
                plan.path(index)
            });
            assert!(
                described <= MAX_DETAILS + 1,
                "described {described} hidden paths after stop"
            );
            assert_eq!(r.total, COUNT);
            assert_eq!(r.not_attempted, COUNT);
            assert_eq!(r.purged + r.preserved + r.uncertain, 0);
            assert_eq!(r.cancelled, cancelled);
            assert_eq!(r.error.is_some(), !cancelled);
            assert!(r.details_truncated);
            assert_eq!(r.outcomes.len(), MAX_DETAILS);
            assert_eq!(c.snapshot().completed, COUNT);
            assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn cancellation_records_remaining_manifest_boundary_and_exact_summary() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        fs::write(d.path().join("selected/a"), b"a").unwrap();
        fs::write(d.path().join("selected/b"), b"b").unwrap();
        let p = prepared(d.path(), "selected");
        let c = DeleteControl::default();
        let r = execute_inner(p, &c, &mut |event, _, _, _| {
            if event == "after_capture" {
                c.cancel()
            }
        });
        assert!(r.cancelled);
        assert!(r.error.is_none(), "{r:?}");
        assert_eq!(r.total, 3);
        assert_eq!(r.preserved, 1);
        assert_eq!(r.purged + r.uncertain, 0);
        assert_eq!(r.not_attempted, 2);
        assert_eq!(r.outcomes.len(), 3);
        assert_eq!(c.snapshot().completed, 3);
        let journal = fs::read_to_string(r.recovery.unwrap().join("journal.jsonl")).unwrap();
        let records: Vec<serde_json::Value> = journal
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let boundary = records
            .iter()
            .find(|r| r.get("stopped_before_manifest_position").is_some())
            .unwrap();
        assert_eq!(boundary["remaining_not_attempted"], 2);
        let final_record = records.last().unwrap();
        assert_eq!(final_record["not_attempted"], 2);
        assert_eq!(final_record["preserved"], 1);
        assert_eq!(fs::read(d.path().join("selected/a")).unwrap(), b"a");
        assert_eq!(fs::read(d.path().join("selected/b")).unwrap(), b"b");
    }
    #[derive(Default)]
    struct CountingWriter {
        calls: usize,
        bytes: Vec<u8>,
        fail: bool,
    }
    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            self.calls += 1;
            if self.fail {
                return Err(io::Error::other("injected record append failure"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn journal_record_is_one_checked_write_including_newline() {
        let mut writer = CountingWriter::default();
        let mut buffer = Vec::new();
        let value = serde_json::json!({"node":7,"name_hex":"616263","selected":true,"slot":"object-0000000000000007"});
        append_journal_record(&mut writer, &mut buffer, &value).unwrap();
        assert_eq!(
            writer.calls, 1,
            "JSON tokens must not become destination writes"
        );
        assert_eq!(writer.bytes.last(), Some(&b'\n'));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&writer.bytes).unwrap(),
            value
        );
        append_journal_record(&mut writer, &mut buffer, &value).unwrap();
        assert_eq!(writer.calls, 2);
        assert_eq!(writer.bytes.iter().filter(|b| **b == b'\n').count(), 2);
        let mut failing = CountingWriter {
            fail: true,
            ..Default::default()
        };
        assert!(append_journal_record(&mut failing, &mut buffer, &value).is_err());
        assert_eq!(failing.calls, 1);
        assert!(failing.bytes.is_empty());
    }
    #[test]
    fn oversized_record_never_reaches_destination_and_buffer_is_reusable() {
        let mut writer = CountingWriter::default();
        let mut buffer = Vec::new();
        let value = serde_json::json!({"oversized":"x".repeat(MAX_JOURNAL_RECORD)});
        assert!(append_journal_record(&mut writer, &mut buffer, &value).is_err());
        assert_eq!(writer.calls, 0);
        assert!(buffer.len() < MAX_JOURNAL_RECORD);
        assert!(buffer.capacity() <= MAX_JOURNAL_RECORD);
        let small = serde_json::json!({"after_error":true});
        append_journal_record(&mut writer, &mut buffer, &small).unwrap();
        assert_eq!(writer.calls, 1);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&writer.bytes).unwrap(),
            small
        );
    }
    #[test]
    fn cancellation_marker_append_failure_retains_data_and_reports_error() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        fs::write(d.path().join("selected/a"), b"keep").unwrap();
        let p = prepared(d.path(), "selected");
        let c = DeleteControl::default();
        let mut captures = 0;
        let r = execute_inner(p, &c, &mut |event, _, _, q| {
            if event == "intent_written" {
                readonly_journal(q);
                c.cancel();
            }
            if event == "before_capture" {
                captures += 1;
            }
        });
        assert_eq!(captures, 0);
        assert!(r.cancelled);
        assert!(r.error.is_some());
        assert_eq!(r.total, 2);
        assert_eq!(r.not_attempted, 2);
        assert_eq!(r.purged + r.preserved + r.uncertain, 0);
        assert!(r.recovery.unwrap().join("journal.jsonl").exists());
        assert_eq!(fs::read(d.path().join("selected/a")).unwrap(), b"keep");
    }
    #[test]
    fn intent_manifest_cancellation_stops_writing_and_prevents_every_capture() {
        let d = fixture();
        fs::create_dir(d.path().join("selected")).unwrap();
        for i in 0..4 {
            fs::write(d.path().join(format!("selected/{i}")), b"keep").unwrap();
        }
        let p = prepared(d.path(), "selected");
        let c = DeleteControl::default();
        let mut intents = 0;
        let mut captures = 0;
        let r = execute_inner(p, &c, &mut |event, _, _, _| {
            if event == "intent_written" {
                intents += 1;
                c.cancel();
            }
            if event == "before_capture" {
                captures += 1;
            }
        });
        assert_eq!(
            intents, 1,
            "cancelled intent writing must stop at the next bounded checkpoint"
        );
        assert_eq!(captures, 0);
        assert!(r.cancelled);
        assert!(r.error.is_none(), "{r:?}");
        assert_eq!(r.total, 5);
        assert_eq!(r.not_attempted, 5);
        assert_eq!(r.purged + r.preserved + r.uncertain, 0);
        assert_eq!(fs::read_dir(d.path().join("selected")).unwrap().count(), 4);
        let recovery = r.recovery.unwrap();
        assert_eq!(fs::read_dir(&recovery).unwrap().count(), 1);
        let journal = fs::read_to_string(recovery.join("journal.jsonl")).unwrap();
        let records: Vec<serde_json::Value> = journal
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            records.iter().filter(|r| r.get("node").is_some()).count(),
            1
        );
        assert!(
            records
                .iter()
                .any(|r| r.get("setup_cancelled") == Some(&serde_json::Value::Bool(true)))
        );
    }
}
