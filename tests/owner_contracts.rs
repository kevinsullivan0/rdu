//! Architecture B contracts: owned fixtures only; no deletion service or timing campaigns.
use rdu::scan::{Backend, Kind, Options, ScanPhase, ScanProgress, Tree, scan};
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn paths(tree: &Tree) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::with_capacity(tree.dirs.len());
    for directory in &tree.dirs {
        paths.push(match directory.parent {
            Some(parent) => paths[parent].join(&directory.name),
            None => tree.root.clone(),
        });
    }
    paths
}
fn oracle(root: &Path, seen: &mut HashSet<(u64, u64)>) -> (u64, u64, u64) {
    let metadata = fs::symlink_metadata(root).unwrap();
    if metadata.is_file() && metadata.nlink() > 1 && !seen.insert((metadata.dev(), metadata.ino()))
    {
        return (0, 0, 1);
    }
    let mut total = (
        metadata.blocks() * 512,
        metadata.size(),
        u64::from(!metadata.is_dir()),
    );
    if metadata.is_dir() {
        for child in fs::read_dir(root).unwrap() {
            let child = oracle(&child.unwrap().path(), seen);
            total.0 += child.0;
            total.1 += child.1;
            total.2 += child.2;
        }
    }
    total
}
fn check_edges_and_global_charge(tree: &Tree) {
    type Charge = (u64, u64, usize, u64, u64);
    let paths = paths(tree);
    let mut link_charges: HashMap<(u64, u64), Charge> = HashMap::new();
    for (id, directory) in tree.dirs.iter().enumerate() {
        let own = fs::symlink_metadata(&paths[id]).unwrap();
        let mut expected = (own.blocks() * 512, own.size());
        for entry in &directory.entries {
            let meta = fs::symlink_metadata(paths[id].join(&entry.name)).unwrap();
            assert_eq!(
                (entry.identity.dev, entry.identity.ino),
                (meta.dev(), meta.ino())
            );
            if let Some(child) = entry.child {
                assert!(child > id && child < tree.dirs.len());
                assert_eq!(tree.dirs[child].parent, Some(id));
                assert_eq!(
                    (entry.allocated, entry.apparent),
                    (tree.dirs[child].allocated, tree.dirs[child].apparent)
                );
            }
            if entry.kind == Kind::File && meta.nlink() > 1 {
                let charge = link_charges.entry((meta.dev(), meta.ino())).or_insert((
                    0,
                    0,
                    0,
                    meta.blocks() * 512,
                    meta.size(),
                ));
                charge.0 += entry.allocated;
                charge.1 += entry.apparent;
                charge.2 += usize::from(entry.allocated != 0 || entry.apparent != 0);
            } else if entry.child.is_none() {
                assert_eq!(
                    (entry.allocated, entry.apparent),
                    (meta.blocks() * 512, meta.size())
                );
            }
            expected.0 += entry.allocated;
            expected.1 += entry.apparent;
        }
        assert_eq!((directory.allocated, directory.apparent), expected);
    }
    for (_, charge) in link_charges {
        assert_eq!((charge.0, charge.1), (charge.3, charge.4));
        assert_eq!(charge.2, 1);
    }
    let root = oracle(&tree.root, &mut HashSet::new());
    assert_eq!(
        (tree.dirs[0].allocated, tree.dirs[0].apparent, tree.files),
        root
    );
}
#[test]
fn every_directory_has_consistent_totals_and_cross_owner_hardlinks_charge_once() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("shared"), b"shared-payload").unwrap();
    for branch in 0..40 {
        let base = fixture.path().join(format!("branch-{branch}"));
        fs::create_dir(&base).unwrap();
        fs::hard_link(fixture.path().join("shared"), base.join("link")).unwrap();
        for level in 0..3 {
            let directory = base.join(format!("d{level}"));
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("own"), b"own").unwrap();
            fs::hard_link(fixture.path().join("shared"), directory.join("link")).unwrap();
        }
    }
    for threads in [1, 8, 16, 64] {
        let tree = scan(
            fixture.path(),
            &Options {
                threads,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(tree.complete);
        assert_eq!(tree.errors, 0);
        check_edges_and_global_charge(&tree);
    }
}
#[test]
fn cancellation_after_root_admits_all_children_preserves_every_placeholder() {
    const CHILDREN: usize = 128;
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn cancelled() -> bool {
        CALLS.fetch_add(1, Ordering::Relaxed) > CHILDREN
    }
    let fixture = tempfile::tempdir().unwrap();
    for i in 0..CHILDREN {
        let directory = fixture.path().join(i.to_string());
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("not-visited"), b"keep").unwrap();
    }
    for threads in [1, 8, 64] {
        CALLS.store(0, Ordering::Relaxed);
        let progress = Arc::new(ScanProgress::new());
        let tree = scan(
            fixture.path(),
            &Options {
                threads,
                backend: Backend::Portable,
                cancelled: Some(cancelled),
                progress: Some(progress.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!tree.complete);
        assert_eq!(tree.dirs.len(), CHILDREN + 1);
        assert_eq!(tree.errors, CHILDREN as u64);
        assert_eq!(tree.files, 0);
        for child in &tree.dirs[0].entries {
            let id = child.child.unwrap();
            assert_eq!(tree.dirs[id].parent, Some(0));
            assert!(tree.dirs[id].entries.is_empty());
            assert_eq!(tree.dirs[id].errors, 1);
        }
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.phase, ScanPhase::Cancelled);
        assert_eq!(snapshot.directories_discovered, CHILDREN as u64 + 1);
        assert_eq!(snapshot.directories_completed, CHILDREN as u64 + 1);
        assert_eq!(snapshot.active_workers, 0);
    }
}
