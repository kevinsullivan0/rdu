//! Single named owned fixture for the explicit 48-FD subprocess check.
use rdu::scan::{Backend, Options, scan};
use std::{fs, os::unix::fs::MetadataExt, path::Path};
fn total(path: &Path) -> (u64, u64, u64) {
    let m = fs::symlink_metadata(path).unwrap();
    let mut sum = (m.blocks() * 512, m.size(), u64::from(!m.is_dir()));
    if m.is_dir() {
        for child in fs::read_dir(path).unwrap() {
            let value = total(&child.unwrap().path());
            sum.0 += value.0;
            sum.1 += value.1;
            sum.2 += value.2;
        }
    }
    sum
}
#[test]
fn portable_owned_capabilities_scan_wide_deep_fixture() {
    let fixture = tempfile::tempdir().unwrap();
    for branch in 0..16 {
        let mut directory = fixture.path().join(branch.to_string());
        for _ in 0..12 {
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("file"), b"fixture").unwrap();
            directory = directory.join("deep");
        }
    }
    let tree = scan(
        fixture.path(),
        &Options {
            threads: 8,
            backend: Backend::Portable,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(tree.complete, "{} errors", tree.errors);
    assert_eq!(tree.dirs.len(), 193);
    assert_eq!(
        (tree.dirs[0].allocated, tree.dirs[0].apparent, tree.files),
        total(fixture.path())
    );
}
