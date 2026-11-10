use rdu::scan::{Backend, Options, Tree, scan};
use std::{fs, path::Path};

fn check_tree(tree: &Tree, limit: usize) {
    assert!(tree.dirs.iter().map(|d| d.entries.len()).sum::<usize>() < limit);
    for (parent_id, parent) in tree.dirs.iter().enumerate() {
        for entry in &parent.entries {
            if let Some(child) = entry.child {
                assert!(child > parent_id && child < tree.dirs.len());
                assert_eq!(tree.dirs[child].parent, Some(parent_id));
                assert_eq!(entry.allocated, tree.dirs[child].allocated);
                assert_eq!(entry.apparent, tree.dirs[child].apparent);
            }
        }
    }
}

fn populate(root: &Path) {
    for branch in 0..8 {
        let base = root.join(format!("b{branch}"));
        fs::create_dir(&base).unwrap();
        for sibling in 0..6 {
            let mut p = base.join(format!("s{sibling}"));
            for _ in 0..3 {
                fs::create_dir(&p).unwrap();
                fs::write(p.join("file"), b"contents").unwrap();
                p = p.join("deep");
            }
        }
    }
}

#[test]
fn wide_deep_tree_keeps_links_and_totals_for_all_worker_counts() {
    let fixture = tempfile::tempdir().unwrap();
    populate(fixture.path());
    let mut expected = None;
    for backend in [Backend::Portable, Backend::Auto] {
        for threads in [1, 2, 8, 64] {
            let tree = scan(
                fixture.path(),
                &Options {
                    threads,
                    backend,
                    ..Options::default()
                },
            )
            .unwrap();
            check_tree(&tree, usize::MAX);
            assert_eq!(tree.errors, 0);
            let totals = (
                tree.dirs[0].allocated,
                tree.dirs[0].apparent,
                tree.files,
                tree.dirs.len(),
            );
            if let Some(previous) = expected {
                assert_eq!(totals, previous);
            }
            expected = Some(totals);
            assert_eq!(tree.files, 144);
            assert_eq!(tree.dirs.len(), 153);
        }
    }
}

#[test]
fn entry_budget_is_hard_bound_under_concurrent_children() {
    let fixture = tempfile::tempdir().unwrap();
    populate(fixture.path());
    for threads in [1, 2, 8, 64] {
        for max_entries in [1, 2, 10, 50, 150] {
            let tree = scan(
                fixture.path(),
                &Options {
                    threads,
                    max_entries,
                    ..Options::default()
                },
            )
            .unwrap();
            check_tree(&tree, max_entries);
            assert!(tree.errors > 0);
        }
    }
}

#[test]
fn depth_budget_and_cancelled_continuations_are_partial() {
    let fixture = tempfile::tempdir().unwrap();
    populate(fixture.path());
    for threads in [1, 8, 64] {
        let tree = scan(
            fixture.path(),
            &Options {
                threads,
                max_depth: 1,
                ..Options::default()
            },
        )
        .unwrap();
        check_tree(&tree, usize::MAX);
        assert_eq!(tree.dirs.len(), 9);
        assert!(tree.errors > 0);
        let tree = scan(
            fixture.path(),
            &Options {
                threads,
                cancelled: Some(|| true),
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(tree.dirs.len(), 1);
        assert_eq!(tree.errors, 1);
        assert_eq!(tree.files, 0);
    }
}

#[test]
fn very_large_entry_limits_and_one_filesystem_match_complete_scan() {
    let fixture = tempfile::tempdir().unwrap();
    populate(fixture.path());
    for threads in [1, 8, 64] {
        for max_entries in [297, usize::MAX / 2, usize::MAX / 2 + 1, usize::MAX] {
            let tree = scan(
                fixture.path(),
                &Options {
                    threads,
                    max_entries,
                    one_file_system: true,
                    ..Options::default()
                },
            )
            .unwrap();
            check_tree(&tree, max_entries);
            assert_eq!(tree.errors, 0);
            assert_eq!(tree.files, 144);
            assert_eq!(tree.dirs.len(), 153);
        }
    }
}

#[test]
fn intermittent_cancellation_keeps_completed_links_valid() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn occasional_cancel() -> bool {
        CALLS.fetch_add(1, Ordering::Relaxed) % 5 == 4
    }
    let fixture = tempfile::tempdir().unwrap();
    populate(fixture.path());
    for threads in [1, 8, 64] {
        let tree = scan(
            fixture.path(),
            &Options {
                threads,
                cancelled: Some(occasional_cancel),
                ..Options::default()
            },
        )
        .unwrap();
        check_tree(&tree, usize::MAX);
        assert!(tree.errors > 0);
    }
}
