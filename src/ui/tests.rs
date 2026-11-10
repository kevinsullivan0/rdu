use super::*;
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
fn fixture() -> (tempfile::TempDir, Tree) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("alpha")).unwrap();
    std::fs::write(root.path().join("alpha/nested"), b"nested").unwrap();
    std::fs::write(root.path().join("zebra"), b"zebra").unwrap();
    let tree = crate::scan::scan(root.path(), &Options::default()).unwrap();
    (root, tree)
}
fn render_tree(tree: &Tree, app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| render::browser(f, tree, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn text(buf: &ratatui::buffer::Buffer) -> String {
    let mut text = String::new();
    let mut skip = 0;
    for cell in &buf.content {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        text.push_str(cell.symbol());
        skip = ratatui::text::Line::raw(cell.symbol())
            .width()
            .saturating_sub(1);
    }
    text
}

#[test]
fn browser_exposes_read_only_and_staging_state() {
    let (_root, tree) = fixture();
    let mut app = App::new(UiOptions {
        read_only: true,
        ..Default::default()
    });
    app.rebuild(&tree);
    let buf = render_tree(&tree, &mut app, 120, 30);
    let rendered = text(&buf);
    assert!(rendered.contains("READ ONLY"));
    assert!(rendered.contains("Staging disabled"));
    assert!(rendered.contains("alpha/"));
    assert!(buf.content.iter().any(|c| c.fg == Color::Cyan));
    assert!(
        buf.content
            .iter()
            .any(|c| c.modifier.contains(Modifier::REVERSED))
    );
}
#[test]
fn responsive_layout_and_monochrome_preserve_names_and_selection() {
    let (_root, tree) = fixture();
    for (w, h) in [
        (120, 36),
        (85, 24),
        (45, 12),
        (30, 8),
        (20, 5),
        (12, 4),
        (1, 1),
    ] {
        let mut app = App::new(UiOptions {
            no_color: true,
            read_only: true,
            ..Default::default()
        });
        app.rebuild(&tree);
        let buf = render_tree(&tree, &mut app, w, h);
        assert!(
            buf.content
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
        if w >= 12 && h >= 4 {
            assert!(text(&buf).contains("alpha/"), "{w}x{h}: {}", text(&buf));
        }
    }
}
#[test]
fn navigation_preserves_parent_entry_after_child_sort_change() {
    let (_root, tree) = fixture();
    let mut app = App::new(UiOptions::default());
    app.rebuild(&tree);
    app.view.filter = "alpha".into();
    app.view.rebuild(&tree);
    app.view.enter(&tree);
    app.view.sort = Sort::Name;
    app.rebuild(&tree);
    app.view.parent(&tree);
    assert_eq!(
        tree.dirs[0].entries[app.view.visible[app.view.selected]].name,
        "alpha"
    );
}
#[test]
fn hostile_names_are_losslessly_escaped() {
    use std::os::unix::ffi::OsStrExt;
    assert_eq!(
        safe_text(std::ffi::OsStr::from_bytes(b"a\x1b\n\xff\\")),
        "a\\x1b\\x0a\\xff\\\\"
    );
    assert_eq!(safe_text(std::ffi::OsStr::new("a\u{202e}b")), "a\\u{202e}b");
}
#[test]
fn loading_has_truthful_unknown_total_and_no_fabricated_bytes() {
    let mut terminal = Terminal::new(TestBackend::new(100, 25)).unwrap();
    let app = App::new(UiOptions::default());
    terminal
        .draw(|f| {
            render::loading(
                f,
                &app,
                std::path::Path::new("/fixture"),
                0.5,
                "Files observed 75\nDirectories 2 / 4\nErrors 0",
                &[4, 8, 20],
            )
        })
        .unwrap();
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Unknown total"));
    assert!(rendered.contains("Files observed 75"));
    assert!(!rendered.contains("%"));
    assert!(!rendered.contains("GiB"));
}
#[test]
fn review_defaults_to_cancel_and_requires_full_list_and_large_terminal() {
    use super::review::{Review, ReviewAction};
    let mut review = Review::new(
        (0..50)
            .map(|i| format!("/fixture/target{i}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "DELETE 50".into(),
    );
    review.layout(80, 20);
    for c in "DELETE 50".chars() {
        review.key(KeyCode::Char(c));
    }
    assert_eq!(
        review.key(KeyCode::Enter),
        ReviewAction::None,
        "unread targets must block confirmation"
    );
    review.key(KeyCode::End);
    assert_eq!(
        review.key(KeyCode::Enter),
        ReviewAction::None,
        "End cannot skip the middle of the review"
    );
    review.key(KeyCode::Home);
    for _ in 0..10 {
        review.key(KeyCode::PageDown);
    }
    assert_eq!(review.key(KeyCode::Enter), ReviewAction::Confirm);
    review.layout(35, 8);
    assert_eq!(
        review.key(KeyCode::Enter),
        ReviewAction::None,
        "short terminal blocks confirmation"
    );
    review.layout(80, 20);
    assert!(review.typed.is_empty());
    assert_eq!(
        review.key(KeyCode::Enter),
        ReviewAction::Cancel,
        "Enter defaults Cancel after resize"
    );
}

#[test]
fn read_only_stage_keeps_fixture_unchanged_and_has_no_plan() {
    let (root, tree) = fixture();
    let before = std::fs::read(root.path().join("alpha/nested")).unwrap();
    let mut app = App::new(UiOptions {
        read_only: true,
        ..Default::default()
    });
    app.rebuild(&tree);
    assert!(app.change_stage(&tree, false).is_none());
    assert!(app.staged.is_empty());
    assert!(app.prepared.is_none());
    assert_eq!(
        std::fs::read(root.path().join("alpha/nested")).unwrap(),
        before
    );
}

#[cfg(target_os = "macos")]
#[test]
fn staging_uses_parent_name_identity_and_subsumes_descendants_without_writes() {
    let (root, _) = fixture();
    std::fs::hard_link(root.path().join("zebra"), root.path().join("alias")).unwrap();
    let path = root.path().canonicalize().unwrap();
    let (tree, session) =
        crate::delete::DeletionSession::scan(&path, &Options::default(), false).unwrap();
    let Some(session) = session else {
        panic!("owned local fixture did not receive live deletion authority")
    };
    let before = std::fs::read(path.join("alpha/nested")).unwrap();
    let mut app = App::new(UiOptions::default());
    app.session = Some(Arc::new(session));
    app.rebuild(&tree);
    for name in ["zebra", "alias"] {
        app.view.selected = app
            .view
            .visible
            .iter()
            .position(|&i| tree.dirs[0].entries[i].name == name)
            .unwrap();
        app.change_stage(&tree, false).unwrap();
        let targets = app.staged.clone();
        assert!(!app.preparing);
        assert!(
            app.prepared.is_none(),
            "Space only changes the in-memory basket"
        );
        app.prepared = Some(app.session.as_ref().unwrap().prepare(&targets).unwrap());
        app.preparing = false;
    }
    assert_eq!(
        app.staged.len(),
        2,
        "two hardlink names remain distinct staged targets"
    );
    app.staged.clear();
    app.prepared = None;
    app.view.selected = app
        .view
        .visible
        .iter()
        .position(|&i| tree.dirs[0].entries[i].name == "alpha")
        .unwrap();
    app.view.enter(&tree);
    app.change_stage(&tree, false).unwrap();
    let targets = app.staged.clone();
    app.prepared = Some(app.session.as_ref().unwrap().prepare(&targets).unwrap());
    app.preparing = false;
    app.view.parent(&tree);
    app.change_stage(&tree, false).unwrap();
    let targets = app.staged.clone();
    assert_eq!(targets.len(), 1, "staging ancestor subsumes prior child");
    let plan = app.session.as_ref().unwrap().prepare(&targets).unwrap();
    assert_eq!(plan.summary().target_count, 1);
    assert_eq!(plan.summary().object_count, 2);
    assert_eq!(std::fs::read(path.join("alpha/nested")).unwrap(), before);
    assert_eq!(
        std::fs::read_dir(&path).unwrap().count(),
        3,
        "no stage journal/quarantine created"
    );
}

#[test]
fn recursive_count_sort_and_reversed_parent_selection_are_stable() {
    let (root, _) = fixture();
    std::fs::create_dir(root.path().join("alpha/deep")).unwrap();
    for n in 0..4 {
        std::fs::write(root.path().join(format!("alpha/deep/{n}")), b"").unwrap();
    }
    std::fs::create_dir(root.path().join("beta")).unwrap();
    for n in 0..3 {
        std::fs::write(root.path().join(format!("beta/{n}")), b"").unwrap();
    }
    let tree = crate::scan::scan(root.path(), &Options::default()).unwrap();
    let mut app = App::new(UiOptions {
        sort: UiSort::Count,
        reverse: true,
        ..Default::default()
    });
    app.rebuild(&tree);
    let alpha = tree.dirs[0]
        .entries
        .iter()
        .position(|e| e.name == "alpha")
        .unwrap();
    let child = tree.dirs[0].entries[alpha].child.unwrap();
    assert_eq!(app.view.counts[child], 6);
    app.view.selected = app.view.visible.iter().position(|&i| i == alpha).unwrap();
    app.view.enter(&tree);
    app.view.parent(&tree);
    assert_eq!(app.view.visible[app.view.selected], alpha);
}

#[test]
fn help_scrolls_in_a_short_terminal_and_control_page_keys_remain_navigation() {
    let (_root, tree) = fixture();
    let mut app = App::new(UiOptions::default());
    app.rebuild(&tree);
    app.view.help = true;
    assert!(matches!(
        app.key(&tree, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 12),
        Action::None
    ));
    assert!(
        app.view.help,
        "arrow keys must scroll help instead of dismissing it"
    );
    app.key(&tree, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 12);
    app.key(
        &tree,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        12,
    );
    assert!(app.drawer == Drawer::None, "Ctrl-B remains page navigation");
}

#[test]
fn truncated_report_uses_full_totals_and_preserves_partial_status() {
    let mut app = App::new(UiOptions::default());
    app.report = Some(crate::delete::DeleteReport {
        total: 5000,
        purged: 4096,
        preserved: 904,
        details_truncated: true,
        recovery: Some(PathBuf::from("/fixture/recovery")),
        outcomes: vec![crate::delete::ObjectOutcome {
            path: PathBuf::from("/fixture/removed"),
            state: crate::delete::OutcomeState::Purged,
            detail: "removed".into(),
        }],
        ..Default::default()
    });
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|f| render::report(f, &app)).unwrap();
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("PARTIAL"), "{rendered}");
    for expected in [
        "Total 5000",
        "Purged 4096",
        "Preserved 904",
        "Details truncated",
        "journal",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
}

#[test]
fn selection_details_scroll_full_names_and_show_identity() {
    let (_root, tree) = fixture();
    let mut app = App::new(UiOptions::default());
    app.rebuild(&tree);
    app.drawer = Drawer::Details;
    render_tree(&tree, &mut app, 60, 12);
    assert!(app.detail_limit > 0);
    app.key(&tree, KeyEvent::new(KeyCode::End, KeyModifiers::NONE), 12);
    assert_eq!(app.detail_scroll, app.detail_limit);
    let rendered = text(&render_tree(&tree, &mut app, 60, 12));
    assert!(rendered.contains("reclaimed space"), "{rendered}");
    app.detail_scroll = 0;
    let rendered = text(&render_tree(&tree, &mut app, 85, 24));
    assert!(rendered.contains("Device"));
    assert!(rendered.contains("Inode"));
}

#[test]
fn actual_styled_buffer_can_be_exported_for_visual_review() {
    let (root, _) = fixture();
    std::fs::create_dir(root.path().join("media")).unwrap();
    std::fs::write(root.path().join("media/video.bin"), vec![0u8; 256 * 1024]).unwrap();
    std::fs::create_dir(root.path().join("research")).unwrap();
    std::fs::write(root.path().join("research/notes.txt"), vec![0u8; 32768]).unwrap();
    std::os::unix::fs::symlink("alpha", root.path().join("latest")).unwrap();
    let mut tree = crate::scan::scan(root.path(), &Options::default()).unwrap();
    tree.root = PathBuf::from("/fixture/Projects");
    let mut app = App::new(UiOptions {
        read_only: true,
        show_count: true,
        ..Default::default()
    });
    app.rebuild(&tree);
    let buffer = render_tree(&tree, &mut app, 120, 30);
    assert!(text(&buffer).contains("media/"));
    assert!(buffer.content.iter().any(|cell| cell.fg == Color::Blue));
    if let Some(path) = std::env::var_os("RDU_UI_SCREENSHOT") {
        use std::fmt::Write;
        fn color(c: Color, default: &str) -> String {
            match c {
                Color::Reset => default.into(),
                Color::Black => "#121722".into(),
                Color::Red => "#f07178".into(),
                Color::Green => "#a8cc8c".into(),
                Color::Yellow => "#e8bd78".into(),
                Color::Blue => "#82aaff".into(),
                Color::Magenta => "#c792ea".into(),
                Color::Cyan => "#80cbc4".into(),
                Color::White => "#d9e1ef".into(),
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                _ => "#d9e1ef".into(),
            }
        }
        fn xml(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }
        let mut svg = String::from(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1112\" height=\"512\" viewBox=\"0 0 1112 512\"><rect width=\"1112\" height=\"512\" rx=\"12\" fill=\"#121722\"/><g font-family=\"Menlo,DejaVu Sans Mono,monospace\" font-size=\"14\">",
        );
        for (i, cell) in buffer.content.iter().enumerate() {
            let x = 16 + (i % 120) * 9;
            let y = 16 + (i / 120) * 16;
            let mut fg = color(cell.fg, "#d9e1ef");
            let mut bg = color(cell.bg, "#121722");
            if cell.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if bg != "#121722" {
                write!(
                    svg,
                    "<rect x=\"{x}\" y=\"{y}\" width=\"9\" height=\"16\" fill=\"{bg}\"/>"
                )
                .unwrap();
            }
            if cell.symbol() != " " {
                write!(
                    svg,
                    "<text x=\"{x}\" y=\"{}\" fill=\"{fg}\" font-weight=\"{}\">{}</text>",
                    y + 13,
                    if cell.modifier.contains(Modifier::BOLD) {
                        "700"
                    } else {
                        "400"
                    },
                    xml(cell.symbol())
                )
                .unwrap();
            }
        }
        svg.push_str("</g></svg>");
        std::fs::write(path, svg).unwrap();
    }
}

#[test]
fn viewport_formats_only_visible_rows_in_a_million_entry_directory() {
    let entries: Vec<_> = (0..1_000_000).collect();
    let mut visited = Vec::new();
    let (first, rows) = render::viewport_rows(&entries, 999_999, 0, 18, |index| {
        visited.push(index);
        index
    });
    assert_eq!(first, 999_982);
    assert_eq!(visited.len(), 18);
    assert_eq!(rows.len(), 18);
    assert_eq!(rows.last(), Some(&999_999));
    let (first, rows) = render::viewport_rows(&entries, 400_000, 999_982, 18, |index| index);
    assert_eq!(first, 400_000);
    assert_eq!(rows.first(), Some(&400_000));
    let (_, rows) = render::viewport_rows(&entries, 0, 0, 0, |_| {
        panic!("zero viewport must not format rows")
    });
    assert!(rows.is_empty());
}

#[test]
fn ascii_decorations_cover_all_views_but_preserve_unicode_names() {
    let (root, tree) = fixture();
    let options = UiOptions {
        ascii: true,
        read_only: true,
        show_count: true,
        ..Default::default()
    };
    let mut app = App::new(options.clone());
    app.rebuild(&tree);
    assert!(text(&render_tree(&tree, &mut app, 120, 30)).is_ascii());
    app.view.help = true;
    assert!(text(&render_tree(&tree, &mut app, 90, 24)).is_ascii());
    app.view.help = false;
    app.drawer = Drawer::Basket;
    assert!(text(&render_tree(&tree, &mut app, 90, 24)).is_ascii());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| {
            render::loading(
                f,
                &app,
                std::path::Path::new("/fixture"),
                1.0,
                "Phase Scanning",
                &[1, 3, 9],
            )
        })
        .unwrap();
    assert!(text(terminal.backend().buffer()).is_ascii());
    let mut review =
        super::review::Review::new("1 target\n/fixture/file".into(), "DELETE 1".into());
    terminal
        .draw(|f| render::review(f, &app, &mut review))
        .unwrap();
    assert!(text(terminal.backend().buffer()).is_ascii());
    std::fs::write(root.path().join("café文·—"), b"name").unwrap();
    let tree = crate::scan::scan(root.path(), &Options::default()).unwrap();
    let mut app = App::new(options);
    app.rebuild(&tree);
    assert!(text(&render_tree(&tree, &mut app, 120, 30)).contains("café文·—"));
}

#[test]
fn viewport_selection_maps_to_the_original_global_entry() {
    let (_root, mut tree) = fixture();
    tree.dirs[0].entries.clear();
    for index in 0..2000 {
        tree.dirs[0].entries.push(crate::scan::Entry {
            identity: Default::default(),
            name: format!("row-{index:04}").into(),
            kind: crate::scan::Kind::File,
            allocated: index + 1,
            apparent: index + 1,
            child: None,
        });
    }
    let mut app = App::new(UiOptions {
        sort: UiSort::Name,
        read_only: true,
        ..Default::default()
    });
    app.rebuild(&tree);
    app.view.selected = 1999;
    let rendered = text(&render_tree(&tree, &mut app, 80, 12));
    assert!(rendered.contains("row-1999"));
    assert!(rendered.contains("2000 / 2000"));
    assert!(app.view.scroll >= 1990);
    app.key(&tree, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 12);
    let rendered = text(&render_tree(&tree, &mut app, 80, 12));
    assert!(rendered.contains("row-0000"));
    assert_eq!(app.view.scroll, 0);
}

#[test]
fn recovery_location_remains_accessible_when_it_wraps_beyond_the_header() {
    let mut app = App::new(UiOptions::default());
    app.report = Some(crate::delete::DeleteReport {
        total: 1,
        preserved: 1,
        recovery: Some(PathBuf::from(format!(
            "/fixture/{}/RECOVERY-END",
            "component/".repeat(30)
        ))),
        ..Default::default()
    });
    let mut terminal = Terminal::new(TestBackend::new(45, 12)).unwrap();
    let mut found = false;
    for scroll in 0..100 {
        app.report_scroll = scroll;
        terminal.draw(|f| render::report(f, &app)).unwrap();
        found |= text(terminal.backend().buffer()).contains("RECOVERY-END");
        if found {
            break;
        }
    }
    assert!(
        found,
        "full recovery location must be available through report scrolling"
    );
}
