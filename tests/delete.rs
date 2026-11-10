#![cfg(target_os = "macos")]
use rdu::{
    delete::{self, DeleteControl, DeletionSession, OutcomeState, Target},
    scan::Options,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

fn fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("rdu-delete-disposable-")
        .tempdir_in("/tmp")
        .unwrap()
}
fn session(p: &Path, name: &str) -> (DeletionSession, Target) {
    let (tree, session) = DeletionSession::scan(p, &Options::default(), false).unwrap();
    let entry = tree.dirs[0]
        .entries
        .iter()
        .position(|e| e.name == name)
        .unwrap();
    (
        session.unwrap(),
        Target {
            directory: 0,
            entry,
        },
    )
}
#[test]
fn preparing_and_wrong_confirmation_never_write() {
    let d = fixture();
    fs::write(d.path().join("file"), b"keep").unwrap();
    let before: Vec<_> = fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let (s, t) = session(d.path(), "file");
    let plan = s.prepare(&[t]).unwrap();
    assert_eq!(plan.summary().object_count, 1);
    assert!(plan.confirm("delete 1").is_err());
    assert_eq!(fs::read(d.path().join("file")).unwrap(), b"keep");
    let after: Vec<_> = fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
}
#[test]
fn confirmed_tree_removes_only_reviewed_objects_and_symlink_itself() {
    let d = fixture();
    let outside = fixture();
    fs::write(outside.path().join("keep"), b"outside").unwrap();
    fs::create_dir_all(d.path().join("selected/empty")).unwrap();
    fs::write(d.path().join("selected/file"), b"data").unwrap();
    fs::hard_link(
        d.path().join("selected/file"),
        d.path().join("keep-hardlink"),
    )
    .unwrap();
    symlink(outside.path(), d.path().join("selected/link")).unwrap();
    let (s, t) = session(d.path(), "selected");
    let plan = s.prepare(&[t]).unwrap();
    assert_eq!(plan.summary().object_count, 4);
    let phrase = plan.summary().confirmation_phrase.clone();
    let report = delete::execute(plan.confirm(&phrase).unwrap(), &DeleteControl::default());
    assert!(report.error.is_none(), "{report:?}");
    assert!(
        report
            .outcomes
            .iter()
            .all(|o| o.state == OutcomeState::Purged),
        "{report:?}"
    );
    assert!(!d.path().join("selected").exists());
    assert_eq!(fs::read(d.path().join("keep-hardlink")).unwrap(), b"data");
    assert_eq!(fs::read(outside.path().join("keep")).unwrap(), b"outside");
}
#[test]
fn readonly_and_incomplete_scans_have_no_authority() {
    let d = fixture();
    fs::write(d.path().join("f"), b"x").unwrap();
    assert!(
        DeletionSession::scan(d.path(), &Options::default(), true)
            .unwrap()
            .1
            .is_none()
    );
    let options = Options {
        max_entries: 1,
        ..Options::default()
    };
    assert!(
        DeletionSession::scan(d.path(), &options, false)
            .unwrap()
            .1
            .is_none()
    );
}
#[test]
fn replaced_after_prepare_is_preserved() {
    let d = fixture();
    fs::write(d.path().join("file"), b"old").unwrap();
    let (s, t) = session(d.path(), "file");
    let p = s.prepare(&[t]).unwrap();
    fs::rename(d.path().join("file"), d.path().join("original")).unwrap();
    fs::write(d.path().join("file"), b"replacement").unwrap();
    let phrase = p.summary().confirmation_phrase.clone();
    let report = delete::execute(p.confirm(&phrase).unwrap(), &DeleteControl::default());
    assert!(
        report
            .outcomes
            .iter()
            .all(|o| o.state != OutcomeState::Purged)
    );
    assert_eq!(fs::read(d.path().join("original")).unwrap(), b"old");
    assert_eq!(fs::read(d.path().join("file")).unwrap(), b"replacement");
}
#[test]
fn cancellation_before_execution_has_no_filesystem_effect() {
    let d = fixture();
    fs::write(d.path().join("file"), b"keep").unwrap();
    let (s, t) = session(d.path(), "file");
    let p = s.prepare(&[t]).unwrap();
    let phrase = p.summary().confirmation_phrase.clone();
    let c = DeleteControl::default();
    c.cancel();
    let report = delete::execute(p.confirm(&phrase).unwrap(), &c);
    assert!(report.cancelled);
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
}
#[test]
fn large_closure_is_supported_with_bounded_active_handles() {
    let d = fixture();
    fs::create_dir(d.path().join("selected")).unwrap();
    for i in 0..1024 {
        fs::write(d.path().join(format!("selected/{i}")), b"x").unwrap();
    }
    let (s, t) = session(d.path(), "selected");
    let p = s.prepare(&[t]).unwrap();
    assert_eq!(p.summary().object_count, 1025);
    let phrase = p.summary().confirmation_phrase.clone();
    let report = delete::execute(p.confirm(&phrase).unwrap(), &DeleteControl::default());
    assert!(report.error.is_none(), "{report:?}");
    assert!(!d.path().join("selected").exists());
}
#[test]
fn two_hardlinks_in_one_plan_are_both_removed() {
    let d = fixture();
    fs::create_dir(d.path().join("selected")).unwrap();
    fs::write(d.path().join("selected/a"), b"data").unwrap();
    fs::hard_link(d.path().join("selected/a"), d.path().join("selected/b")).unwrap();
    let (s, t) = session(d.path(), "selected");
    let p = s.prepare(&[t]).unwrap();
    let phrase = p.summary().confirmation_phrase.clone();
    let report = delete::execute(p.confirm(&phrase).unwrap(), &DeleteControl::default());
    assert!(
        report
            .outcomes
            .iter()
            .all(|o| o.state == OutcomeState::Purged),
        "{report:?}"
    );
}
#[test]
fn final_root_symlink_never_mints_authority_even_with_trailing_separator() {
    let d = fixture();
    let link = fixture();
    symlink(d.path(), link.path().join("root")).unwrap();
    for p in [link.path().join("root"), link.path().join("root/")] {
        assert!(
            DeletionSession::scan(&p, &Options::default(), false)
                .map(|(_, s)| s.is_none())
                .unwrap_or(true)
        );
    }
}
#[test]
fn fifo_is_refused_without_opening_for_contents() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let d = fixture();
    let p = CString::new(d.path().join("fifo").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
    let (s, t) = session(d.path(), "fifo");
    assert!(s.prepare(&[t]).is_err());
}
#[test]
fn writable_anchor_is_refused() {
    let d = fixture();
    fs::write(d.path().join("file"), b"keep").unwrap();
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        DeletionSession::scan(d.path(), &Options::default(), false)
            .unwrap()
            .1
            .is_none()
    );
    assert_eq!(fs::read(d.path().join("file")).unwrap(), b"keep");
}

#[test]
fn deep_tree_deletion_uses_bounded_descriptors_under_low_limit() {
    use std::{
        ffi::CString,
        io::Write,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::{ffi::OsStrExt, process::CommandExt},
        },
        process::Command,
    };
    if std::env::var_os("RDU_DELETE_LOW_FD_CHILD").is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.args([
            "--exact",
            "deep_tree_deletion_uses_bounded_descriptors_under_low_limit",
            "--test-threads=1",
        ]);
        child.env("RDU_DELETE_LOW_FD_CHILD", "1");
        unsafe {
            child.pre_exec(|| {
                let mut limit = std::mem::zeroed();
                if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                limit.rlim_cur = 64;
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let out = child.output().unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let d = fixture();
    fs::create_dir(d.path().join("selected")).unwrap();
    for i in 0..256 {
        fs::write(d.path().join(format!("selected/file-{i}")), b"x").unwrap();
    }
    let raw = CString::new(d.path().join("selected").as_os_str().as_bytes()).unwrap();
    let fd = unsafe {
        libc::open(
            raw.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    assert!(fd >= 0);
    let mut parent = unsafe { OwnedFd::from_raw_fd(fd) };
    for _ in 0..240 {
        assert_eq!(
            unsafe { libc::mkdirat(parent.as_raw_fd(), c"nested".as_ptr(), 0o700) },
            0
        );
        let next = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c"nested".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(next >= 0);
        parent = unsafe { OwnedFd::from_raw_fd(next) };
    }
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c"leaf".as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY | libc::O_CLOEXEC,
            0o600,
        )
    };
    assert!(fd >= 0);
    let mut leaf = unsafe { fs::File::from_raw_fd(fd) };
    leaf.write_all(b"leaf").unwrap();
    drop(leaf);
    drop(parent);
    let options = Options {
        threads: 1,
        ..Default::default()
    };
    let (tree, s) = DeletionSession::scan(d.path(), &options, false).unwrap();
    assert!(tree.complete, "scan errors {}", tree.errors);
    let t = Target {
        directory: 0,
        entry: tree.dirs[0]
            .entries
            .iter()
            .position(|e| e.name == "selected")
            .unwrap(),
    };
    let p = s.unwrap().prepare(&[t]).unwrap();
    assert_eq!(p.summary().object_count, 498);
    assert!(
        p.manifest_path(p.summary().object_count - 1)
            .unwrap()
            .as_os_str()
            .len()
            > 1024
    );
    let phrase = p.summary().confirmation_phrase.clone();
    let report = delete::execute(p.confirm(&phrase).unwrap(), &DeleteControl::default());
    assert!(report.error.is_none(), "{report:?}");
    assert!(
        report
            .outcomes
            .iter()
            .all(|o| o.state == OutcomeState::Purged),
        "{report:?}"
    );
    assert!(!d.path().join("selected").exists());
}

#[test]
fn dangerous_acl_is_not_treated_as_private_by_mode_bits() {
    let d = fixture();
    fs::write(d.path().join("file"), b"keep").unwrap();
    let status = std::process::Command::new("/bin/chmod")
        .args(["+a", "everyone allow write,delete_child,directory_inherit"])
        .arg(d.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        DeletionSession::scan(d.path(), &Options::default(), false)
            .unwrap()
            .1
            .is_none()
    );
    assert_eq!(fs::read(d.path().join("file")).unwrap(), b"keep");
    assert!(
        std::process::Command::new("/bin/chmod")
            .arg("-N")
            .arg(d.path())
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn large_reports_keep_exact_totals_and_retain_full_journal() {
    let d = fixture();
    fs::create_dir(d.path().join("selected")).unwrap();
    for i in 0..4100 {
        fs::write(d.path().join(format!("selected/{i}")), b"x").unwrap();
    }
    let (s, t) = session(d.path(), "selected");
    let p = s.prepare(&[t]).unwrap();
    let phrase = p.summary().confirmation_phrase.clone();
    let r = delete::execute(p.confirm(&phrase).unwrap(), &DeleteControl::default());
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.total, 4101);
    assert_eq!(r.purged, 4101);
    assert_eq!(r.not_attempted + r.preserved + r.uncertain, 0);
    assert!(r.details_truncated);
    assert_eq!(r.outcomes.len(), 4096);
    assert!(!d.path().join("selected").exists());
    let journal = fs::read_to_string(r.recovery.unwrap().join("journal.jsonl")).unwrap();
    assert_eq!(
        journal
            .lines()
            .filter(|l| serde_json::from_str::<serde_json::Value>(l)
                .unwrap()
                .get("purged")
                .is_some())
            .count(),
        4102
    );
}

#[test]
fn cancelled_preparation_mints_no_plan_and_never_writes() {
    let d = fixture();
    fs::write(d.path().join("file"), b"keep").unwrap();
    let (s, t) = session(d.path(), "file");
    let control = delete::PrepareControl::default();
    control.cancel();
    assert_eq!(
        s.prepare_with_control(&[t], &control).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    assert_eq!(fs::read(d.path().join("file")).unwrap(), b"keep");
}

#[test]
fn writable_and_readonly_scans_preserve_the_same_root_spelling() {
    let d = fixture();
    fs::write(d.path().join("keep"), b"fixture").unwrap();
    let (writable, session) = DeletionSession::scan(d.path(), &Options::default(), false).unwrap();
    let (readonly, authority) = DeletionSession::scan(d.path(), &Options::default(), true).unwrap();
    assert_eq!(writable.root, readonly.root);
    assert_eq!(writable.dirs[0].identity, readonly.dirs[0].identity);
    assert!(session.is_some());
    assert!(authority.is_none());
    let entry = writable.dirs[0]
        .entries
        .iter()
        .position(|e| e.name == "keep")
        .unwrap();
    let plan = session
        .unwrap()
        .prepare(&[Target {
            directory: 0,
            entry,
        }])
        .unwrap();
    assert_eq!(
        plan.summary().paths[0],
        fs::canonicalize(d.path()).unwrap().join("keep")
    );
}
