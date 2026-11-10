use rdu::scan::{Backend, Kind, Options, scan};
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, PermissionsExt, symlink},
    },
};

fn opts(backend: Backend) -> Options {
    Options {
        backend,
        ..Options::default()
    }
}
fn oracle(
    path: &std::path::Path,
    seen: &mut std::collections::HashSet<(u64, u64)>,
) -> (u64, u64, u64) {
    let m = fs::symlink_metadata(path).unwrap();
    if m.is_file() && m.nlink() > 1 && !seen.insert((m.dev(), m.ino())) {
        return (0, 0, 1);
    }
    let mut t = (m.blocks() * 512, m.size(), u64::from(!m.is_dir()));
    if m.is_dir() {
        for e in fs::read_dir(path).unwrap() {
            let q = oracle(&e.unwrap().path(), seen);
            t.0 += q.0;
            t.1 += q.1;
            t.2 += q.2;
        }
    }
    t
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    fs::create_dir(p.join("a")).unwrap();
    fs::create_dir(p.join("b")).unwrap();
    fs::write(p.join("a/file"), b"hello").unwrap();
    fs::hard_link(p.join("a/file"), p.join("b/link")).unwrap();
    let f = fs::File::create(p.join("sparse")).unwrap();
    f.set_len(8 * 1024 * 1024).unwrap();
    symlink("a", p.join("directory-link")).unwrap();
    symlink("missing", p.join("broken")).unwrap();
    symlink(".", p.join("cycle")).unwrap();
    let mut name = b"bad\x1b]52;c;xx\x07\n".to_vec();
    if cfg!(target_os = "linux") {
        name.push(255);
    }
    fs::write(p.join(OsString::from_vec(name)), b"x").unwrap();
    d
}
#[test]
fn exact_metadata_hardlinks_sparse_and_hostile_names() {
    let d = fixture();
    let t = scan(d.path(), &opts(Backend::Portable)).unwrap();
    let o = oracle(d.path(), &mut Default::default());
    assert_eq!((t.dirs[0].allocated, t.dirs[0].apparent, t.files), o);
    assert_eq!(t.dirs.len(), 3);
    assert_eq!(t.errors, 0);
    assert_eq!(
        t.dirs[0]
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Symlink)
            .count(),
        3
    );
}
#[test]
fn thread_counts_agree() {
    let d = fixture();
    let mut o = opts(Backend::Portable);
    o.threads = 1;
    let a = scan(d.path(), &o).unwrap();
    o.threads = 8;
    let b = scan(d.path(), &o).unwrap();
    assert_eq!(
        (a.dirs[0].allocated, a.dirs[0].apparent, a.files),
        (b.dirs[0].allocated, b.dirs[0].apparent, b.files)
    );
}
#[test]
fn missing_and_symlink_root_rejected() {
    let d = tempfile::tempdir().unwrap();
    assert!(scan(&d.path().join("none"), &opts(Backend::Portable)).is_err());
    symlink(d.path(), d.path().join("link")).unwrap();
    assert!(scan(&d.path().join("link"), &opts(Backend::Portable)).is_err());
}
#[test]
fn permission_errors_are_partial() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("denied")).unwrap();
    fs::set_permissions(d.path().join("denied"), fs::Permissions::from_mode(0o0)).unwrap();
    let t = scan(d.path(), &opts(Backend::Portable));
    fs::set_permissions(d.path().join("denied"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(t.unwrap().errors > 0);
}

#[test]
fn symlink_root_with_trailing_components_rejected() {
    let d = tempfile::tempdir().unwrap();
    symlink(d.path(), d.path().join("link")).unwrap();
    for suffix in ["link/", "link/."] {
        assert!(
            scan(&d.path().join(suffix), &opts(Backend::Portable)).is_err(),
            "{suffix}"
        );
    }
}

#[test]
fn deep_tree_matches_oracle() {
    let d = tempfile::tempdir().unwrap();
    let mut p = d.path().to_owned();
    for _ in 0..120 {
        p.push("d");
        fs::create_dir(&p).unwrap();
        fs::write(p.join("x"), b"x").unwrap();
    }
    let t = scan(d.path(), &opts(Backend::Auto)).unwrap();
    let o = oracle(d.path(), &mut Default::default());
    assert_eq!((t.dirs[0].allocated, t.dirs[0].apparent, t.files), o);
    assert_eq!(t.dirs.len(), 121);
    assert_eq!(t.errors, 0);
}

#[cfg(target_os = "macos")]
#[test]
fn resource_fork_bulk_matches_stat() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("resource");
    fs::write(&file, b"main fork").unwrap();
    let path = std::ffi::CString::new(file.as_os_str().as_encoded_bytes()).unwrap();
    let payload = vec![42u8; 16384];
    assert_eq!(
        unsafe {
            libc::setxattr(
                path.as_ptr(),
                c"com.apple.ResourceFork".as_ptr(),
                payload.as_ptr().cast(),
                payload.len(),
                0,
                0,
            )
        },
        0
    );
    let a = scan(d.path(), &opts(Backend::Portable)).unwrap();
    let b = scan(d.path(), &opts(Backend::Bulk)).unwrap();
    assert_eq!(
        (a.dirs[0].allocated, a.dirs[0].apparent, a.errors),
        (b.dirs[0].allocated, b.dirs[0].apparent, b.errors)
    );
}

#[test]
fn fifo_is_metadata_only() {
    let d = tempfile::tempdir().unwrap();
    let path =
        std::ffi::CString::new(d.path().join("pipe").as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let t = scan(d.path(), &opts(Backend::Auto)).unwrap();
    assert_eq!(t.files, 1);
    assert_eq!(t.dirs[0].entries[0].kind, Kind::Other);
    assert_eq!(t.errors, 0);
}

#[test]
fn concurrent_directory_symlink_swaps_do_not_escape() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("outside-sentinel"), b"not in root").unwrap();
    let child = d.path().join("child");
    fs::create_dir(&child).unwrap();
    for i in 0..200 {
        fs::write(child.join(format!("f{i}")), b"x").unwrap();
    }
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let parked = d.path().join("parked");
            while !stop.load(Ordering::Relaxed) {
                if fs::rename(&child, &parked).is_ok() {
                    let _ = symlink(outside.path(), &child);
                    let _ = fs::remove_file(&child);
                    let _ = fs::rename(&parked, &child);
                }
            }
        });
        for _ in 0..15 {
            let t = scan(d.path(), &opts(Backend::Auto)).unwrap();
            assert!(
                !t.dirs
                    .iter()
                    .flat_map(|d| d.entries.iter())
                    .any(|e| e.name == "outside-sentinel")
            );
        }
        stop.store(true, Ordering::Relaxed);
    });
}
#[test]
fn limits_are_explicit() {
    let d = fixture();
    let mut o = opts(Backend::Portable);
    o.max_entries = 2;
    assert!(scan(d.path(), &o).unwrap().errors > 0);
    o.max_entries = 100;
    o.max_depth = 0;
    assert!(scan(d.path(), &o).unwrap().errors > 0);
}
#[cfg(target_os = "macos")]
#[test]
fn bulk_matches_portable() {
    let d = fixture();
    let a = scan(d.path(), &opts(Backend::Portable)).unwrap();
    let b = scan(d.path(), &opts(Backend::Bulk)).unwrap();
    assert_eq!(
        (a.dirs[0].allocated, a.dirs[0].apparent, a.files, a.errors),
        (b.dirs[0].allocated, b.dirs[0].apparent, b.files, b.errors)
    );
}
