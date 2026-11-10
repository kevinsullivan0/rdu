const HELP: &str = "↑↓ / j k  Select     Enter / l  Open directory\n← / h / Backspace  Parent (restores selection)\nPgUp PgDn / g G  Page / first / last\ns  Size sort   n  Name sort   c  Recursive item count sort\na  Allocated / apparent bytes    /  Filter\ni  Selection details    b  Staging basket\nSpace / d  Stage (observational; no filesystem writes)\nr  Refresh    ?  Help    q / Esc / Ctrl-C  Quit\n\nDeletion needs a separate full target review and exact typed confirmation.\nImports and incomplete scans are always READ ONLY.\nSymlinks are displayed as links and never traversed.\nD  Final deletion review (separate from staging)\n↑↓ / PgUp PgDn scroll this help. Other keys return.";
use super::{
    App, Drawer,
    format::{human_size, safe_text},
    view::Sort,
};
use crate::scan::{Kind, Tree};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Sparkline, Table, TableState, Wrap,
    },
};

pub(super) fn accent(app: &App) -> Style {
    if app.options.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
}
pub(super) fn decorations(app: &App, text: &str) -> String {
    if !app.options.ascii {
        return text.to_owned();
    }
    text.replace('·', "|")
        .replace("↑↓", "Up/Down")
        .replace('↑', "Up")
        .replace('↓', "Down")
        .replace('←', "Left")
        .replace('→', "Right")
        .replace('—', "-")
        .replace('×', "x")
}
fn block<'a>(app: &App, title: impl Into<Line<'a>>) -> Block<'a> {
    let title: Line<'a> = title.into();
    let title = if app.options.ascii {
        Line::from(decorations(app, &title.to_string()))
    } else {
        title
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .border_style(accent(app));
    if app.options.ascii {
        block.border_set(ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        })
    } else {
        block
    }
}
pub(super) fn directory_path(tree: &Tree, mut id: usize) -> std::path::PathBuf {
    let mut names = Vec::new();
    while let Some(parent) = tree.dirs[id].parent {
        names.push(&tree.dirs[id].name);
        id = parent;
    }
    let mut path = tree.root.clone();
    for name in names.into_iter().rev() {
        path.push(name);
    }
    path
}

pub(super) fn browser(frame: &mut Frame, tree: &Tree, app: &mut App) {
    let area = frame.area();
    if area.width < 25 || area.height < 6 {
        let name = app
            .view
            .visible
            .get(app.view.selected)
            .map(|&id| {
                let entry = &tree.dirs[app.view.dir].entries[id];
                format!(
                    "> {}{}",
                    safe_text(&entry.name),
                    if entry.kind == Kind::Directory {
                        "/"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_else(|| "No entries".into());
        let bytes = app
            .view
            .visible
            .get(app.view.selected)
            .map(|&id| app.view.bytes(&tree.dirs[app.view.dir].entries[id]))
            .unwrap_or(0);
        let mode = if app.partial {
            "PARTIAL RO"
        } else if app.imported {
            "IMPORTED RO"
        } else if app.read_only() || app.session.is_none() {
            "READ ONLY"
        } else {
            "LIVE STAGING"
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(mode, accent(app)),
                Line::styled(name, Style::default().add_modifier(Modifier::REVERSED)),
                Line::raw(human_size(bytes)),
                Line::raw("j/k Enter q"),
            ]),
            area,
        );
        return;
    }
    let mode = if app.imported {
        "IMPORTED · READ ONLY"
    } else if app.partial {
        "PARTIAL · READ ONLY"
    } else if app.options.read_only || app.session.is_none() {
        "READ ONLY"
    } else {
        "LIVE · STAGING"
    };
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(if area.height >= 16 { 3 } else { 1 }),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" rdu ", accent(app)),
                Span::raw(decorations(app, mode)),
                Span::raw(if app.options.filtered {
                    if app.options.ascii {
                        " | FILTERED SCAN"
                    } else {
                        " · FILTERED SCAN"
                    }
                } else {
                    ""
                }),
            ]),
            Line::from(format!(
                " Path: {}",
                safe_text(directory_path(tree, app.view.dir).as_os_str())
            )),
        ]),
        chunks[0],
    );
    let dir = &tree.dirs[app.view.dir];
    let total = if app.view.apparent {
        dir.apparent
    } else {
        dir.allocated
    };
    let stats = format!(
        "Here {}   Files {}   Dirs {}   Errors {}   Scan {:.2}s",
        human_size(total),
        tree.files,
        tree.dirs.len(),
        tree.errors,
        tree.elapsed.as_secs_f64()
    );
    let stats = Paragraph::new(stats).style(accent(app));
    frame.render_widget(
        if area.height >= 16 {
            stats.block(block(app, " Scan complete "))
        } else {
            stats
        },
        chunks[1],
    );
    if app.view.help {
        let help = super::review::wrap(
            &decorations(app, HELP),
            chunks[2].width.saturating_sub(2).max(1) as usize,
        );
        app.help_limit = help
            .len()
            .saturating_sub(chunks[2].height.saturating_sub(2) as usize);
        app.help_scroll = app.help_scroll.min(app.help_limit);
        frame.render_widget(
            Paragraph::new(
                help.into_iter()
                    .skip(app.help_scroll)
                    .map(Line::raw)
                    .collect::<Vec<_>>(),
            )
            .block(block(app, " Navigation & safety · ↑↓ scroll ")),
            chunks[2],
        );
    } else if app.drawer == Drawer::Basket {
        basket(frame, tree, app, chunks[2]);
    } else {
        let wide = area.width >= 110 && area.height >= 24;
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if wide {
                vec![Constraint::Min(30), Constraint::Length(34)]
            } else {
                vec![Constraint::Min(1)]
            })
            .split(chunks[2]);
        if !wide && app.drawer != Drawer::None {
            details(frame, tree, app, body[0]);
        } else {
            table(frame, tree, app, body[0]);
        }
        if wide {
            details(frame, tree, app, body[1]);
        }
    }
    let filter = format!(
        "{} / {} {} Filter: {}{} {}",
        app.view
            .selected
            .saturating_add(usize::from(!app.view.visible.is_empty())),
        app.view.visible.len(),
        if app.options.ascii { "|" } else { "·" },
        app.view.filter,
        if app.view.filtering { "_" } else { "" },
        app.message
    );
    frame.render_widget(Paragraph::new(vec![Line::raw(filter), Line::raw(decorations(app,if area.width >= 80 { "↑↓ Select  Enter Open  / Filter  a Metric  s/n/c Sort  Space Stage  b Basket  ? Help  q Quit" } else { "↑↓ Enter / a b ? q · r Refresh" }))]), chunks[3]);
}

fn table(frame: &mut Frame, tree: &Tree, app: &mut App, area: Rect) {
    let dir = &tree.dirs[app.view.dir];
    let total = if app.view.apparent {
        dir.apparent
    } else {
        dir.allocated
    };
    let wide = area.width >= 70;
    let medium = area.width >= 45;
    let count = wide && (app.options.show_count || matches!(app.view.sort, Sort::Count));
    let metric = if app.view.apparent {
        "apparent"
    } else {
        "allocated"
    };
    let sort = match app.view.sort {
        Sort::Size => "size",
        Sort::Name => "name",
        Sort::Count => "item count",
    };
    let title = format!(" Files · {metric} · sort:{sort} ");
    let capacity = area
        .height
        .saturating_sub(if area.height >= 5 { 2 } else { 0 })
        .saturating_sub(if area.height >= 3 { 1 } else { 0 }) as usize;
    let included = app.included_by_stage(tree, app.view.dir);
    let (first, rows) = viewport_rows(
        &app.view.visible,
        app.view.selected,
        app.view.scroll,
        capacity,
        |id| {
            let entry = &dir.entries[id];
            let bytes = app.view.bytes(entry);
            let fraction = if total == 0 {
                0.0
            } else {
                bytes as f64 / total as f64
            };
            let suffix = match entry.kind {
                Kind::Directory => "/",
                Kind::Symlink => "@",
                Kind::Other => "*",
                Kind::File => "",
            };
            let name = format!(
                "{}{}{}",
                {
                    let mark = app.staged_state(app.view.dir, id, included);
                    if mark.is_empty() {
                        String::new()
                    } else {
                        format!("{mark} ")
                    }
                },
                safe_text(&entry.name),
                suffix
            );
            let name_style = if app.options.no_color {
                Style::default()
            } else {
                match entry.kind {
                    Kind::Directory => Style::default().fg(Color::Cyan),
                    Kind::Symlink => Style::default().fg(Color::Magenta),
                    _ => Style::default(),
                }
            };
            let mut cells = vec![
                Cell::from(name).style(name_style),
                Cell::from(human_size(bytes)),
            ];
            if medium {
                cells.push(Cell::from(format!("{:>5.1}%", fraction * 100.0)));
            }
            if wide {
                let filled = ((fraction * 12.0).round() as usize).min(12);
                let bar = if app.options.ascii {
                    format!("{}{}", "#".repeat(filled), "-".repeat(12 - filled))
                } else {
                    format!("{}{}", "█".repeat(filled), "░".repeat(12 - filled))
                };
                cells.push(Cell::from(bar).style(if app.options.no_color {
                    Style::default()
                } else {
                    Style::default().fg(Color::Blue)
                }));
            }
            if count {
                cells.push(Cell::from(
                    entry
                        .child
                        .map(|id| app.view.counts[id].to_string())
                        .unwrap_or_else(|| (if app.options.ascii { "-" } else { "—" }).into()),
                ));
            }
            Row::new(cells)
        },
    );
    let mut widths = vec![Constraint::Min(5), Constraint::Length(11)];
    let mut headers = vec!["Name", "Size"];
    if medium {
        widths.push(Constraint::Length(7));
        headers.push("Share");
    }
    if wide {
        widths.push(Constraint::Length(12));
        headers.push("Usage");
    }
    if count {
        widths.push(Constraint::Length(6));
        headers.push("Items");
    }
    let mut table = Table::new(rows, widths)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD))
        .highlight_symbol("> ")
        .column_spacing(1);
    if area.height >= 5 {
        table = table.block(block(app, title));
    }
    if area.height >= 3 {
        table = table.header(Row::new(headers).style(accent(app)));
    }
    let mut state = TableState::default().with_selected(if app.view.visible.is_empty() {
        None
    } else {
        Some(app.view.selected.saturating_sub(first))
    });
    frame.render_stateful_widget(table, area, &mut state);
    app.view.scroll = first;
    if area.width > 3 && area.height > 4 && !app.options.ascii {
        let mut scroll = ScrollbarState::new(app.view.visible.len()).position(app.view.selected);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(ratatui::layout::Margin::new(0, 1)),
            &mut scroll,
        );
    }
}
fn details(frame: &mut Frame, tree: &Tree, app: &mut App, area: Rect) {
    let sections =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    let text = if let Some(&id) = app.view.visible.get(app.view.selected) {
        let entry = &tree.dirs[app.view.dir].entries[id];
        let identity = if app.drawer == Drawer::Details {
            format!(
                "\nDevice {}\nInode {}",
                entry.identity.dev, entry.identity.ino
            )
        } else {
            String::new()
        };
        format!(
            "{}\n\n{:?}\nAllocated  {}\nApparent   {}{identity}\n\nDisk usage is not guaranteed reclaimed space.",
            safe_text(&entry.name),
            entry.kind,
            human_size(entry.allocated),
            human_size(entry.apparent)
        )
    } else {
        "No matching entries".into()
    };
    let target = if app.drawer == Drawer::Details {
        area
    } else {
        sections[0]
    };
    let lines = super::review::wrap(&text, target.width.saturating_sub(2).max(1) as usize);
    app.detail_limit = lines
        .len()
        .saturating_sub(target.height.saturating_sub(2) as usize);
    app.detail_scroll = app.detail_scroll.min(app.detail_limit);
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(app.detail_scroll)
                .map(Line::raw)
                .collect::<Vec<_>>(),
        )
        .block(block(
            app,
            if app.drawer == Drawer::Details {
                " Selection · ↑↓ scroll · i/Esc return "
            } else {
                " Selection · i "
            },
        )),
        target,
    );
    if app.drawer == Drawer::Details {
        return;
    }

    let basket = if app.read_only() || app.session.is_none() {
        "READ ONLY\nStaging disabled\nA fresh complete live scan with supported safety policy is required.".into()
    } else if app.staged.is_empty() {
        "No staged targets\n\nSpace / d  Stage selection\nStaging changes nothing on disk.".into()
    } else {
        let mut text = decorations(
            app,
            &format!(
                "{} targets · {}\nSelected disk usage {}\n",
                app.staged.len(),
                if app.preparing {
                    "CHECKING"
                } else if app.prepared.is_some() {
                    "READY"
                } else if app.prepare_failed {
                    "FAILED"
                } else {
                    "STAGED · D reviews"
                },
                app.prepared
                    .as_ref()
                    .map(|p| human_size(p.summary().allocated_bytes))
                    .unwrap_or_else(|| "pending".into())
            ),
        );
        let limit = sections[1].height.saturating_sub(6).max(1) as usize;
        let first = app.basket_selected.saturating_sub(limit.saturating_sub(1));
        for (i, target) in app.staged.iter().enumerate().skip(first).take(limit) {
            text.push_str(&format!(
                "{} {}\n",
                if i == app.basket_selected { ">" } else { " " },
                safe_text(&tree.dirs[target.directory].entries[target.entry].name)
            ));
        }
        text.push_str("\nD  Final target review\nSpace  Remove selected basket target");
        text
    };
    let basket = Paragraph::new(basket)
        .wrap(Wrap { trim: false })
        .block(block(app, " Basket · b "));
    frame.render_widget(basket, sections[1]);
}

pub(super) fn loading(
    frame: &mut Frame,
    app: &App,
    path: &std::path::Path,
    elapsed: f64,
    counters: &str,
    rates: &[u64],
) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(2),
    ])
    .split(frame.area());
    frame.render_widget(
        Paragraph::new(format!(
            "rdu  SCANNING  {:.1}s\nPath: {}",
            elapsed,
            safe_text(path.as_os_str())
        ))
        .style(accent(app)),
        areas[0],
    );
    frame.render_widget(
        Paragraph::new(decorations(app, counters))
            .wrap(Wrap { trim: false })
            .block(block(app, " Live metadata progress ")),
        areas[1],
    );
    if !app.options.no_progress && app.options.ascii {
        let max = rates.iter().copied().max().unwrap_or(1).max(1);
        let levels = b".:-=+*#%@";
        let points: String = rates
            .iter()
            .rev()
            .take(areas[2].width.saturating_sub(2) as usize)
            .rev()
            .map(|rate| levels[(((*rate as u128) * 8 / max as u128).min(8)) as usize] as char)
            .collect();
        frame.render_widget(
            Paragraph::new(points)
                .style(accent(app))
                .block(block(app, " Entries/s | recent samples ")),
            areas[2],
        );
    } else if !app.options.no_progress {
        frame.render_widget(
            Sparkline::default()
                .data(rates)
                .style(accent(app))
                .block(block(
                    app,
                    if rates.is_empty() {
                        " Entries/s · collecting rate "
                    } else {
                        " Entries/s · recent samples "
                    },
                )),
            areas[2],
        );
    }
    frame.render_widget(
        Paragraph::new(decorations(
            app,
            "Unknown total · no ETA\nx Stop and keep partial results (read only)    q Quit",
        )),
        areas[3],
    );
}

pub(super) fn review(frame: &mut Frame, app: &App, review: &mut super::review::Review) {
    let area = frame.area();
    review.layout(area.width, area.height);
    if !review.adequate {
        frame.render_widget(Paragraph::new(decorations(app,"Deletion confirmation disabled\nEnlarge terminal to review deletion (60×14 minimum)\nEsc Cancel")).wrap(Wrap{trim:false}).block(block(app," Final review ")),area);
        return;
    }
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(5),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(Paragraph::new(decorations(app,"PERMANENT DELETION · Final review\nOnly these freshly prepared objects are admitted.\nUnexpected objects are preserved; partial failure may need recovery.")).style(accent(app)),sections[0]);
    let visible = review
        .lines
        .iter()
        .skip(review.scroll)
        .take(sections[1].height.saturating_sub(2) as usize)
        .cloned()
        .map(Line::raw)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible).block(block(app, " Exact targets & policy · ↑↓ PgUp PgDn ")),
        sections[1],
    );
    let ready = if review.seen_bottom {
        "Full review displayed"
    } else {
        "Scroll through the entire review to enable confirmation"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{}\nType exactly: {}\n> {}",
            if review.error.is_empty() {
                ready
            } else {
                &review.error
            },
            review.phrase,
            review.typed
        ))
        .block(block(app, " Confirmation ")),
        sections[2],
    );
    frame.render_widget(Paragraph::new(decorations(app,"Esc Cancel · Enter with empty field = Cancel\nm Inspect manifest · Enter commits an exact reviewed plan")),sections[3]);
}
fn basket(frame: &mut Frame, tree: &Tree, app: &App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .split(area);
    let state = if app.preparing {
        "CHECKING"
    } else if app.prepared.is_some() {
        "READY"
    } else {
        "STAGED | D prepares review"
    };
    frame.render_widget(Paragraph::new(decorations(app,&format!("{} top-level targets · {state}\nStaging is observational; filesystem remains unchanged.\nSelected disk usage is not guaranteed reclaimed space.",app.staged.len()))).wrap(Wrap{trim:false}),sections[0]);
    let capacity = sections[1].height.saturating_sub(2) as usize;
    let first = app
        .basket_selected
        .saturating_add(1)
        .saturating_sub(capacity.max(1));
    let rows = app.staged.iter().skip(first).take(capacity).map(|target| {
        Row::new([safe_text(
            &tree.dirs[target.directory].entries[target.entry].name,
        )])
    });
    let table = Table::new(rows, [Constraint::Min(1)])
        .block(block(app, " Staging basket "))
        .highlight_symbol("> ")
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD));
    let mut selection = TableState::default().with_selected(if app.staged.is_empty() {
        None
    } else {
        Some(app.basket_selected.saturating_sub(first))
    });
    frame.render_stateful_widget(table, sections[1], &mut selection);
    if let Some(target) = app.staged.get(app.basket_selected) {
        let path = directory_path(tree, target.directory)
            .join(&tree.dirs[target.directory].entries[target.entry].name);
        frame.render_widget(
            Paragraph::new(safe_text(path.as_os_str()))
                .wrap(Wrap { trim: false })
                .block(block(app, " Selected target ")),
            sections[2],
        );
    }
    frame.render_widget(
        Paragraph::new(decorations(
            app,
            "↑↓ Select · Space Remove · D Final review · b/Esc Return",
        )),
        sections[3],
    );
}

pub(super) fn deleting(frame: &mut Frame, app: &App) {
    let Some(control) = &app.deleting else {
        return;
    };
    let progress = control.snapshot();
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(frame.area());
    frame.render_widget(Paragraph::new("Executing confirmed plan\nCancellation stops new mutations; already completed work remains.").wrap(Wrap{trim:false}).style(accent(app)),areas[0]);
    let ratio = if progress.total == 0 {
        0.0
    } else {
        progress.completed as f64 / progress.total as f64
    };
    frame.render_widget(
        ratatui::widgets::Gauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .label(format!(
                "{} / {} manifest objects",
                progress.completed, progress.total
            ))
            .gauge_style(accent(app))
            .block(block(app, " Known plan progress ")),
        areas[1],
    );
    frame.render_widget(Paragraph::new(format!("Purged {}\nPreserved {}\n\nWait for exact outcomes and recovery locations before leaving.",progress.purged,progress.preserved)).wrap(Wrap{trim:false}),areas[2]);
    frame.render_widget(
        Paragraph::new(
            "x / q / Esc / Ctrl-C  Request cancellation\nCancellation does not undo earlier work.",
        ),
        areas[3],
    );
}

pub(super) fn report(frame: &mut Frame, app: &App) {
    let Some(report) = &app.report else {
        return;
    };
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(frame.area());
    let title = if report.cancelled {
        "CANCELLED · PARTIAL OUTCOMES"
    } else if super::report_is_complete(report) {
        "CONFIRMED OPERATION FINISHED"
    } else {
        "PARTIAL · PRESERVED / FAILED / UNCERTAIN OBJECTS"
    };
    let recovery = report
        .recovery
        .as_ref()
        .map(|p| safe_text(p.as_os_str()))
        .unwrap_or_else(|| "none reported".into());
    let error = report
        .error
        .as_deref()
        .map(|s| safe_text(std::ffi::OsStr::new(s)))
        .unwrap_or_default();
    let truncation = if report.details_truncated && report.recovery.is_some() {
        "Details truncated · inspect recovery storage; its journal may be incomplete"
    } else if report.details_truncated {
        "Details truncated · no recovery storage reported"
    } else {
        "All per-object details are available below"
    };
    frame.render_widget(
        Paragraph::new(decorations(
            app,
            &format!(
                "{title}\nTotal {} · Purged {} · Preserved {}\nNot attempted {} · Uncertain {}",
                report.total,
                report.purged,
                report.preserved,
                report.not_attempted,
                report.uncertain
            ),
        ))
        .style(accent(app)),
        areas[0],
    );
    let mut lines = vec![
        Line::raw(format!("Recovery: {recovery}")),
        Line::raw(decorations(app, truncation)),
        Line::raw(error),
        Line::raw(""),
    ];
    for outcome in &report.outcomes {
        lines.push(Line::raw(format!(
            "{:?}  {}",
            outcome.state,
            safe_text(outcome.path.as_os_str())
        )));
        lines.push(Line::raw(safe_text(std::ffi::OsStr::new(&outcome.detail))));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.report_scroll.min(u16::MAX as usize) as u16, 0))
            .block(block(app, " Exact per-object outcomes ")),
        areas[1],
    );
    frame.render_widget(Paragraph::new(decorations(app,"↑↓ / PgUp PgDn  Scroll · r  Fresh rescan · q  Quit\nRecovery objects are preserved; no automatic purge.")),areas[2]);
}

pub(super) fn manifest(
    frame: &mut Frame,
    app: &App,
    reader: &mut super::review::Review,
    index: usize,
    count: usize,
) {
    let area = frame.area();
    reader.layout(area.width, area.height);
    let areas = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(Paragraph::new(decorations(app,&format!("Fresh immutable manifest · object {} of {}\nIncludes hidden and scan-excluded descendants",index+1,count))).style(accent(app)),areas[0]);
    frame.render_widget(
        Paragraph::new(
            reader
                .lines
                .iter()
                .skip(reader.scroll)
                .cloned()
                .map(Line::raw)
                .collect::<Vec<_>>(),
        )
        .block(block(app, " Exact escaped path ")),
        areas[1],
    );
    frame.render_widget(Paragraph::new(decorations(app,"n/p  Next/previous object · ↑↓  Scroll path\nEsc / m  Return to final review · no changes made")),areas[2]);
}

/// Format only visible rows; global selection and scroll never become row-local authority.
pub(super) fn viewport_rows<T>(
    entries: &[usize],
    selected: usize,
    scroll: usize,
    capacity: usize,
    mut format: impl FnMut(usize) -> T,
) -> (usize, Vec<T>) {
    if entries.is_empty() || capacity == 0 {
        return (0, Vec::new());
    }
    let selected = selected.min(entries.len() - 1);
    let mut first = scroll.min(entries.len() - 1);
    if selected < first {
        first = selected;
    } else if selected >= first.saturating_add(capacity) {
        first = selected.saturating_add(1).saturating_sub(capacity);
    }
    first = first.min(entries.len().saturating_sub(capacity));
    let end = first.saturating_add(capacity).min(entries.len());
    (
        first,
        entries[first..end]
            .iter()
            .map(|&index| format(index))
            .collect(),
    )
}
