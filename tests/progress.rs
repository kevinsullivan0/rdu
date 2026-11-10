use rdu::scan::{self, Options, ScanPhase, ScanProgress, TreeOrigin};
use std::{fs, os::unix::fs::MetadataExt, sync::Arc};

#[test]
fn retained_identities_match_real_nodes_and_live_origin() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("child")).unwrap();
    fs::write(root.path().join("file"), b"data").unwrap();
    let tree = scan::scan(root.path(), &Options::default()).unwrap();
    let metadata = root.path().symlink_metadata().unwrap();
    assert_eq!(tree.origin, TreeOrigin::Live);
    assert!(tree.complete);
    assert_eq!(tree.dirs[0].identity.dev, metadata.dev());
    assert_eq!(tree.dirs[0].identity.ino, metadata.ino());
    for entry in &tree.dirs[0].entries {
        let metadata = root.path().join(&entry.name).symlink_metadata().unwrap();
        assert_eq!(entry.identity.dev, metadata.dev());
        assert_eq!(entry.identity.ino, metadata.ino());
        if let Some(child) = entry.child {
            assert_eq!(tree.dirs[child].identity, entry.identity);
        }
    }
}

#[test]
fn progress_counts_work_without_changing_accounting() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("child")).unwrap();
    for i in 0..700 {
        fs::write(root.path().join("child").join(i.to_string()), b"data").unwrap();
    }
    let plain = scan::scan(root.path(), &Options::default()).unwrap();
    let progress = Arc::new(ScanProgress::new());
    let options = Options {
        progress: Some(progress.clone()),
        ..Options::default()
    };
    let tree = scan::scan(root.path(), &options.clone()).unwrap();
    let snapshot = progress.snapshot();
    assert_eq!(snapshot.phase, ScanPhase::Complete);
    assert_eq!(snapshot.entries_seen, 701);
    assert_eq!(snapshot.files_seen, 700);
    assert_eq!(snapshot.directories_discovered, 2);
    assert_eq!(snapshot.directories_completed, 2);
    assert_eq!(snapshot.active_workers, 0);
    assert_eq!(snapshot.errors, tree.errors);
    assert!(snapshot.elapsed >= tree.elapsed);
    assert_eq!(
        (tree.dirs[0].allocated, tree.dirs[0].apparent, tree.files),
        (plain.dirs[0].allocated, plain.dirs[0].apparent, plain.files)
    );
}

#[test]
fn pre_cancelled_progress_returns_truthful_partial_tree() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"data").unwrap();
    let progress = Arc::new(ScanProgress::new());
    progress.cancel();
    let tree = scan::scan(
        root.path(),
        &Options {
            progress: Some(progress.clone()),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(progress.is_cancelled());
    assert!(!tree.complete);
    assert!(tree.errors > 0);
    assert_eq!(tree.files, 0);
    let snapshot = progress.snapshot();
    assert_eq!(snapshot.phase, ScanPhase::Cancelled);
    assert_eq!(snapshot.active_workers, 0);
    assert_eq!(
        snapshot.directories_completed,
        snapshot.directories_discovered
    );
}
