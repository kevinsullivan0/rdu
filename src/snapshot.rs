//! Versioned flat, byte-preserving offline snapshots. Import never grants live authority.
use crate::scan::{Directory, Entry, Identity, Kind, Tree, TreeOrigin};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::OpenOptions,
    io::{self, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    time::Duration,
};
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_NODES: usize = 1_000_000;
const MAX_NAMES: usize = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 1024;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Id {
    dev: u64,
    ino: u64,
}
impl From<Identity> for Id {
    fn from(i: Identity) -> Self {
        Self {
            dev: i.dev,
            ino: i.ino,
        }
    }
}
impl From<Id> for Identity {
    fn from(i: Id) -> Self {
        Self {
            dev: i.dev,
            ino: i.ino,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
}
impl From<Kind> for FileKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::File => Self::File,
            Kind::Directory => Self::Directory,
            Kind::Symlink => Self::Symlink,
            Kind::Other => Self::Other,
        }
    }
}
impl From<FileKind> for Kind {
    fn from(k: FileKind) -> Self {
        match k {
            FileKind::File => Self::File,
            FileKind::Directory => Self::Directory,
            FileKind::Symlink => Self::Symlink,
            FileKind::Other => Self::Other,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    name: Vec<u8>,
    kind: FileKind,
    identity: Id,
    allocated: u64,
    apparent: u64,
    child: Option<usize>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Folder {
    name: Vec<u8>,
    parent: Option<usize>,
    identity: Id,
    allocated: u64,
    apparent: u64,
    errors: u64,
    entries: Vec<Row>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: String,
    version: u32,
    root: Vec<u8>,
    files: u64,
    errors: u64,
    complete: bool,
    elapsed_nanos: u64,
    dirs: Vec<Folder>,
}
fn check_name(bytes: &[u8], component: bool, total: &mut usize) -> io::Result<()> {
    if bytes.len() > 65536 {
        return Err(invalid("snapshot name exceeds 64 KiB"));
    }
    *total = total
        .checked_add(bytes.len())
        .ok_or_else(|| invalid("name bytes overflow"))?;
    if *total > MAX_NAMES {
        return Err(invalid("snapshot names exceed 16 MiB"));
    }
    if bytes.contains(&0)
        || (component
            && (bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/')))
    {
        return Err(invalid("invalid snapshot name"));
    }
    Ok(())
}
// Both serialization and deserialization pass this gate before acquiring output
// authority or exposing indices to rendering. No format limit is import-only.
fn validate(value: &Snapshot) -> io::Result<()> {
    if value.schema != "rdu-snapshot" || value.version != 1 {
        return Err(invalid(
            "unsupported snapshot schema; expected rdu-snapshot version 1",
        ));
    }
    let mut name_bytes = 0;
    check_name(&value.root, false, &mut name_bytes)?;
    let dir_count = value.dirs.len();
    if dir_count == 0 || dir_count > MAX_NODES {
        return Err(invalid("invalid snapshot directory count"));
    }
    let mut nodes = dir_count;
    let mut depths = Vec::<usize>::with_capacity(dir_count);
    let mut incoming = vec![0u8; dir_count];
    let mut actual_files = 0u64;
    for (id, dir) in value.dirs.iter().enumerate() {
        let depth = match dir.parent {
            None if id == 0 => 0,
            Some(p) if p < id => depths[p] + 1,
            _ => {
                return Err(invalid(
                    "snapshot parent graph is not a rooted ordered tree",
                ));
            }
        };
        if depth > MAX_DEPTH {
            return Err(invalid("snapshot depth exceeds 1024"));
        }
        depths.push(depth);
        check_name(&dir.name, id > 0, &mut name_bytes)?;
        nodes = nodes
            .checked_add(dir.entries.len())
            .ok_or_else(|| invalid("snapshot node count overflow"))?;
        if nodes > MAX_NODES {
            return Err(invalid("snapshot exceeds one million nodes"));
        }
        let mut names = HashSet::new();
        for entry in &dir.entries {
            check_name(&entry.name, true, &mut name_bytes)?;
            if !names.insert(entry.name.as_slice()) {
                return Err(invalid("duplicate snapshot entry name"));
            }
            let directory = matches!(entry.kind, FileKind::Directory);
            if !directory {
                actual_files += 1
            }
            if let Some(child) = entry.child {
                if child <= id || child >= dir_count || !directory {
                    return Err(invalid("invalid snapshot child index"));
                }
                let target = &value.dirs[child];
                if target.parent != Some(id)
                    || target.name != entry.name
                    || target.identity != entry.identity
                    || incoming[child] != 0
                {
                    return Err(invalid("inconsistent snapshot child graph"));
                }
                incoming[child] = 1;
            }
        }
    }
    if incoming.iter().skip(1).any(|&n| n != 1) {
        return Err(invalid("unreachable snapshot directory"));
    }
    if value.files != actual_files {
        return Err(invalid("inconsistent snapshot file count"));
    }
    Ok(())
}
fn wire(tree: &Tree) -> io::Result<Snapshot> {
    let mut nodes = tree.dirs.len();
    let mut names = tree.root.as_os_str().as_bytes().len();
    for d in &tree.dirs {
        nodes = nodes.saturating_add(d.entries.len());
        names = names.saturating_add(d.name.as_bytes().len());
        for e in &d.entries {
            names = names.saturating_add(e.name.as_bytes().len());
        }
    }
    if nodes > MAX_NODES || names > MAX_NAMES {
        return Err(invalid("tree exceeds snapshot node/name limits"));
    }
    let snapshot = Snapshot {
        schema: "rdu-snapshot".into(),
        version: 1,
        root: tree.root.as_os_str().as_bytes().to_vec(),
        files: tree.files,
        errors: tree.errors,
        complete: tree.complete,
        elapsed_nanos: tree.elapsed.as_nanos().min(u64::MAX as u128) as u64,
        dirs: tree
            .dirs
            .iter()
            .map(|d| Folder {
                name: d.name.as_bytes().to_vec(),
                parent: d.parent,
                identity: d.identity.into(),
                allocated: d.allocated,
                apparent: d.apparent,
                errors: d.errors,
                entries: d
                    .entries
                    .iter()
                    .map(|e| Row {
                        name: e.name.as_bytes().to_vec(),
                        kind: e.kind.into(),
                        identity: e.identity.into(),
                        allocated: e.allocated,
                        apparent: e.apparent,
                        child: e.child,
                    })
                    .collect(),
            })
            .collect(),
    };
    validate(&snapshot)?;
    Ok(snapshot)
}
pub fn export(tree: &Tree, path: &Path) -> io::Result<()> {
    let value = wire(tree)?;
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) as u64 > MAX_BYTES - 1 {
                return Err(invalid("snapshot exceeds 64 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = Bounded(Vec::new());
    serde_json::to_writer(&mut encoded, &value)?;
    let mut out = io::BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?,
    );
    out.write_all(&encoded.0)?;
    writeln!(out)?;
    out.flush()
}
pub fn import(path: &Path) -> io::Result<Tree> {
    let data = crate::input::read_regular(path, MAX_BYTES, "snapshot", "64 MiB")?;
    // Decode directly to typed rows, avoiding a JSON Value allocation for every byte/integer.
    let value: Snapshot = serde_json::from_slice(&data)?;
    drop(data);
    validate(&value)?;
    let root = PathBuf::from(OsString::from_vec(value.root));
    let dirs = value
        .dirs
        .into_iter()
        .map(|dir| Directory {
            name: OsString::from_vec(dir.name),
            parent: dir.parent,
            identity: dir.identity.into(),
            allocated: dir.allocated,
            apparent: dir.apparent,
            errors: dir.errors,
            entries: dir
                .entries
                .into_iter()
                .map(|entry| Entry {
                    name: OsString::from_vec(entry.name),
                    kind: entry.kind.into(),
                    identity: entry.identity.into(),
                    allocated: entry.allocated,
                    apparent: entry.apparent,
                    child: entry.child,
                })
                .collect(),
        })
        .collect();
    Ok(Tree {
        root,
        dirs,
        files: value.files,
        errors: value.errors,
        elapsed: Duration::from_nanos(value.elapsed_nanos),
        origin: TreeOrigin::Imported,
        complete: value.complete,
    })
}
