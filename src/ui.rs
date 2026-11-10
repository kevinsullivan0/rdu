//! Ratatui application. The UI never receives filesystem mutation primitives.
mod format;
mod render;
mod review;
mod terminal;
mod view;

use crate::delete::{DeleteControl, DeleteReport, DeletionSession, PreparedPlan, Target};
use crate::scan::{Options, Tree};
pub use format::safe_text;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::HashSet, sync::Arc};
use std::{io, path::PathBuf};
use view::{Sort, View};

#[derive(Clone, Copy, Debug, Default)]
pub enum UiSort {
    #[default]
    Size,
    Name,
    Count,
}
#[derive(Clone, Debug, Default)]
pub struct UiOptions {
    pub apparent: bool,
    pub read_only: bool,
    pub no_color: bool,
    pub no_progress: bool,
    pub show_count: bool,
    pub filtered: bool,
    pub ascii: bool,
    pub sort: UiSort,
    pub reverse: bool,
}

struct App {
    view: View,
    options: UiOptions,
    partial: bool,
    imported: bool,
    drawer: Drawer,
    message: String,
    session: Option<Arc<DeletionSession>>,
    staged: Vec<Target>,
    staged_exact: HashSet<Target>,
    staged_roots: HashSet<usize>,
    prepare_failed: bool,
    prepared: Option<PreparedPlan>,
    preparing: bool,
    review: Option<review::Review>,
    deleting: Option<Arc<DeleteControl>>,
    report: Option<DeleteReport>,
    report_scroll: usize,
    basket_selected: usize,
    manifest: Option<review::Review>,
    manifest_index: usize,
    help_scroll: usize,
    help_limit: usize,
    detail_scroll: usize,
    detail_limit: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drawer {
    None,
    Details,
    Basket,
}

impl App {
    fn new(options: UiOptions) -> Self {
        let mut view = View::new(options.apparent);
        view.reverse = options.reverse;
        view.sort = match options.sort {
            UiSort::Name => Sort::Name,
            UiSort::Count => Sort::Count,
            UiSort::Size => Sort::Size,
        };
        Self {
            view,
            options,
            partial: false,
            imported: false,
            drawer: Drawer::None,
            message: String::new(),
            session: None,
            staged: Vec::new(),
            staged_exact: HashSet::new(),
            staged_roots: HashSet::new(),
            prepare_failed: false,
            prepared: None,
            preparing: false,
            review: None,
            deleting: None,
            report: None,
            report_scroll: 0,
            basket_selected: 0,
            manifest: None,
            manifest_index: 0,
            help_scroll: 0,
            help_limit: 0,
            detail_scroll: 0,
            detail_limit: 0,
        }
    }
    fn rebuild(&mut self, tree: &Tree) {
        let selected = self.view.visible.get(self.view.selected).copied();
        self.view.rebuild(tree);
        if let Some(selected) = selected {
            self.view.selected = self
                .view
                .visible
                .iter()
                .position(|&i| i == selected)
                .unwrap_or(0);
        }
    }
    fn read_only(&self) -> bool {
        self.options.read_only || self.partial || self.imported
    }
    fn staged_state(&self, directory: usize, entry: usize, included: bool) -> &'static str {
        let target = Target { directory, entry };
        if self.staged_exact.contains(&target) {
            return if self.preparing {
                "[CHECK]"
            } else if self.prepare_failed {
                "[FAILED]"
            } else {
                "[STAGED]"
            };
        }
        if included { "[INCLUDED]" } else { "" }
    }
    fn included_by_stage(&self, tree: &Tree, mut directory: usize) -> bool {
        if self.staged_roots.is_empty() {
            return false;
        }
        loop {
            if self.staged_roots.contains(&directory) {
                return true;
            }
            let Some(parent) = tree.dirs[directory].parent else {
                return false;
            };
            directory = parent;
        }
    }
    fn clear_staged(&mut self) {
        self.staged.clear();
        self.staged_exact.clear();
        self.staged_roots.clear();
    }
    fn sync_stage_marks(&mut self, tree: &Tree) {
        self.staged_exact = self.staged.iter().copied().collect();
        self.staged_roots = self
            .staged
            .iter()
            .filter_map(|target| tree.dirs[target.directory].entries[target.entry].child)
            .collect();
    }

    fn change_stage(&mut self, tree: &Tree, remove: bool) -> Option<()> {
        if self.read_only() || self.session.is_none() {
            self.message = "READ ONLY or unsupported deletion policy: staging disabled".into();
            return None;
        }
        if self.preparing {
            self.message = "Checking staged targets; wait or refresh to discard".into();
            return None;
        }
        let target = if remove {
            *self.staged.get(self.basket_selected)?
        } else {
            Target {
                directory: self.view.dir,
                entry: *self.view.visible.get(self.view.selected)?,
            }
        };
        if let Some(index) = self.staged.iter().position(|item| *item == target) {
            self.staged.remove(index);
        } else {
            if self.included_by_stage(tree, target.directory) {
                self.message = "Selection is included by a staged ancestor".into();
                return None;
            }
            self.staged
                .retain(|child| !covered(tree, target, child.directory));
            self.staged.push(target);
        }
        self.sync_stage_marks(tree);
        self.prepared = None;
        self.review = None;
        self.prepare_failed = false;
        self.basket_selected = self
            .basket_selected
            .min(self.staged.len().saturating_sub(1));
        self.message = format!(
            "Staged in memory: {} targets; D prepares final review",
            self.staged.len()
        );
        Some(())
    }
    fn start_prepare(&mut self) -> Option<Vec<Target>> {
        if self.read_only() || self.session.is_none() || self.preparing || self.staged.is_empty() {
            self.message = "Stage live targets before opening final review".into();
            return None;
        }
        self.prepared = None;
        self.preparing = true;
        self.prepare_failed = false;
        self.message = "CHECKING fresh full closure; x cancels preparation".into();
        Some(self.staged.clone())
    }

    fn open_review(&mut self) {
        if self.read_only() || self.preparing {
            self.message = "A complete freshly prepared live plan is required".into();
            return;
        }
        if let Some(plan) = &self.prepared {
            let summary = plan.summary();
            let mut text = format!(
                "{} top-level targets {} {} manifest objects\nSelected disk usage {} (not guaranteed reclaimed space)\n\n",
                summary.target_count,
                if self.options.ascii { "|" } else { "·" },
                summary.object_count,
                format::human_size(summary.allocated_bytes)
            );
            for (i, path) in summary.paths.iter().enumerate() {
                text.push_str(&format!("{}. {}\n", i + 1, safe_text(path.as_os_str())));
            }
            text.push_str("\nFolders include ALL contents, including scan-excluded items.\nPress m to inspect the exact fresh object manifest before confirming.\n\nPolicy and limitations\n");
            for note in &summary.notes {
                text.push_str(&format!("{}\n", safe_text(std::ffi::OsStr::new(note))));
            }
            self.review = Some(review::Review::new(
                text,
                summary.confirmation_phrase.clone(),
            ));
        } else {
            self.message = "Stage targets and wait for preparation before final review".into();
        }
    }
    fn key(&mut self, tree: &Tree, key: KeyEvent, height: u16) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'd'))
        {
            return Action::Quit;
        }
        if self.view.help {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = (self.help_scroll + 1).min(self.help_limit)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => {
                    self.help_scroll = self
                        .help_scroll
                        .saturating_add(height.saturating_sub(9).max(1) as usize)
                        .min(self.help_limit)
                }
                KeyCode::PageUp => {
                    self.help_scroll = self
                        .help_scroll
                        .saturating_sub(height.saturating_sub(9).max(1) as usize)
                }
                KeyCode::Home => self.help_scroll = 0,
                KeyCode::End => self.help_scroll = self.help_limit,
                _ => self.view.help = false,
            }
            return Action::None;
        }
        if self.drawer == Drawer::Basket && !self.view.filtering {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.basket_selected =
                        (self.basket_selected + 1).min(self.staged.len().saturating_sub(1));
                    return Action::None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.basket_selected = self.basket_selected.saturating_sub(1);
                    return Action::None;
                }
                KeyCode::Char(' ' | 'd') => return Action::Unstage,
                _ => {}
            }
        }
        if self.view.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.view.filter.clear();
                    self.view.filtering = false;
                }
                KeyCode::Enter => self.view.filtering = false,
                KeyCode::Backspace => {
                    self.view.filter.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.view.filter.clear()
                }
                KeyCode::Char(c)
                    if !c.is_control()
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.view.filter.push(c)
                }
                _ => {}
            }
            self.view.selected = 0;
            self.view.visible.clear();
            self.rebuild(tree);
            return Action::None;
        }
        if self.drawer == Drawer::Details {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.detail_scroll = (self.detail_scroll + 1).min(self.detail_limit);
                    return Action::None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                    return Action::None;
                }
                KeyCode::PageDown => {
                    self.detail_scroll = self
                        .detail_scroll
                        .saturating_add(height.saturating_sub(9).max(1) as usize)
                        .min(self.detail_limit);
                    return Action::None;
                }
                KeyCode::PageUp => {
                    self.detail_scroll = self
                        .detail_scroll
                        .saturating_sub(height.saturating_sub(9).max(1) as usize);
                    return Action::None;
                }
                KeyCode::Home => {
                    self.detail_scroll = 0;
                    return Action::None;
                }
                KeyCode::End => {
                    self.detail_scroll = self.detail_limit;
                    return Action::None;
                }
                _ => {}
            }
        }
        self.detail_scroll = 0;
        match key.code {
            KeyCode::Esc if self.drawer != Drawer::None => self.drawer = Drawer::None,
            KeyCode::Esc | KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('?') => self.view.help = true,
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.view.selected = self
                    .view
                    .selected
                    .saturating_add(height.saturating_sub(9).max(1) as usize)
                    .min(self.view.visible.len().saturating_sub(1));
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.view.selected = self
                    .view
                    .selected
                    .saturating_sub(height.saturating_sub(9).max(1) as usize);
            }
            KeyCode::Char('b') => {
                self.drawer = if self.drawer == Drawer::Basket {
                    Drawer::None
                } else {
                    Drawer::Basket
                }
            }
            KeyCode::Char('i') => {
                self.drawer = if self.drawer == Drawer::Details {
                    Drawer::None
                } else {
                    Drawer::Details
                }
            }
            KeyCode::Char('/') => {
                self.view.filtering = true;
                self.view.filter.clear();
                self.rebuild(tree);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.view.selected =
                    (self.view.selected + 1).min(self.view.visible.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.view.selected = self.view.selected.saturating_sub(1)
            }
            KeyCode::Home | KeyCode::Char('g') => self.view.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.view.selected = self.view.visible.len().saturating_sub(1)
            }
            KeyCode::PageDown => {
                self.view.selected = self
                    .view
                    .selected
                    .saturating_add(height.saturating_sub(9) as usize)
                    .min(self.view.visible.len().saturating_sub(1))
            }
            KeyCode::PageUp => {
                self.view.selected = self
                    .view
                    .selected
                    .saturating_sub(height.saturating_sub(9) as usize)
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.view.enter(tree),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => self.view.parent(tree),
            KeyCode::Char('s') => {
                self.view.sort = Sort::Size;
                self.rebuild(tree);
            }
            KeyCode::Char('n') => {
                self.view.sort = Sort::Name;
                self.rebuild(tree);
            }
            KeyCode::Char('c') => {
                self.view.sort = Sort::Count;
                self.rebuild(tree);
            }
            KeyCode::Char('a') => {
                self.view.apparent = !self.view.apparent;
                self.rebuild(tree);
            }
            KeyCode::Char('r') => return Action::Refresh,
            KeyCode::Char('D') => return Action::Review,
            KeyCode::Char('x') if self.preparing => return Action::StopPreparation,
            KeyCode::Char(' ' | 'd') => return Action::Stage,
            _ => {}
        }
        Action::None
    }
}
#[derive(PartialEq, Eq)]
enum Action {
    None,
    Quit,
    Refresh,
    Stage,
    Unstage,
    Review,
    StopPreparation,
}

/// Browse an offline snapshot. Imported trees never confer deletion authority.
pub fn run_snapshot(tree: Tree, mut options: UiOptions) -> io::Result<u8> {
    options.read_only = true;
    terminal::run_snapshot(tree, options)
}

/// Backward-compatible entry point for a borrowed, read-only tree.
pub fn run(tree: &Tree, apparent: bool) -> io::Result<()> {
    terminal::run_borrowed(
        tree,
        UiOptions {
            apparent,
            read_only: true,
            ..UiOptions::default()
        },
    )
}

/// Start terminal loading immediately; scanning and completion use background events.
pub fn run_path(path: PathBuf, options: Options, ui: UiOptions) -> io::Result<u8> {
    terminal::run_path(path, options, ui)
}

#[cfg(test)]
mod tests;

fn covered(tree: &Tree, parent: Target, mut directory: usize) -> bool {
    let Some(child) = tree.dirs[parent.directory].entries[parent.entry].child else {
        return false;
    };
    loop {
        if directory == child {
            return true;
        }
        let Some(parent) = tree.dirs[directory].parent else {
            return false;
        };
        directory = parent;
    }
}

fn report_is_complete(report: &DeleteReport) -> bool {
    !report.cancelled
        && report.error.is_none()
        && report.total > 0
        && report.total == report.purged
        && report.preserved == 0
        && report.not_attempted == 0
        && report.uncertain == 0
        && report
            .outcomes
            .iter()
            .all(|o| o.state == crate::delete::OutcomeState::Purged)
}
