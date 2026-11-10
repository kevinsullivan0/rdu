use super::format::safe_text as escaped;
use crate::scan::Tree;
#[derive(Clone, Copy)]
pub(super) enum Sort {
    Size,
    Count,
    Name,
}

pub(super) struct View {
    pub(super) dir: usize,
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) visible: Vec<usize>,
    pub(super) sort: Sort,
    pub(super) apparent: bool,
    pub(super) reverse: bool,
    pub(super) counts: Vec<usize>,
    pub(super) filter: String,
    pub(super) filtering: bool,
    pub(super) help: bool,
    // Parent directory, immutable entry index, previous scroll offset.
    pub(super) history: Vec<(usize, usize, usize)>,
}

impl View {
    pub(super) fn new(apparent: bool) -> Self {
        Self {
            dir: 0,
            selected: 0,
            scroll: 0,
            visible: Vec::new(),
            sort: Sort::Size,
            apparent,
            reverse: false,
            counts: Vec::new(),
            filter: String::new(),
            filtering: false,
            help: false,
            history: Vec::new(),
        }
    }
    pub(super) fn rebuild(&mut self, tree: &Tree) {
        use std::os::unix::ffi::OsStrExt;
        if self.counts.len() != tree.dirs.len() {
            self.counts = tree.dirs.iter().map(|dir| dir.entries.len()).collect();
            for id in (0..tree.dirs.len()).rev() {
                if let Some(parent) = tree.dirs[id].parent {
                    self.counts[parent] = self.counts[parent].saturating_add(self.counts[id]);
                }
            }
        }
        let counts = &self.counts;
        let entries = &tree.dirs[self.dir].entries;
        let filter = self.filter.to_lowercase();
        self.visible = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                filter.is_empty() || escaped(&entry.name).to_lowercase().contains(&filter)
            })
            .map(|(index, _)| index)
            .collect();
        let apparent = self.apparent;
        let sort = self.sort;
        self.visible.sort_unstable_by(|&a, &b| {
            let a = &entries[a];
            let b = &entries[b];
            let names = || a.name.as_bytes().cmp(b.name.as_bytes());
            match sort {
                Sort::Name => names(),
                Sort::Count => b
                    .child
                    .map(|id| counts[id])
                    .unwrap_or(0)
                    .cmp(&a.child.map(|id| counts[id]).unwrap_or(0))
                    .then_with(names),
                Sort::Size => {
                    let a_size = if apparent { a.apparent } else { a.allocated };
                    let b_size = if apparent { b.apparent } else { b.allocated };
                    b_size.cmp(&a_size).then_with(names)
                }
            }
        });
        if self.reverse {
            self.visible.reverse();
        }
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.selected);
    }
    pub(super) fn bytes(&self, entry: &crate::scan::Entry) -> u64 {
        if self.apparent {
            entry.apparent
        } else {
            entry.allocated
        }
    }
    pub(super) fn enter(&mut self, tree: &Tree) {
        if let Some((entry, child)) = self.visible.get(self.selected).and_then(|&index| {
            tree.dirs[self.dir].entries[index]
                .child
                .map(|child| (index, child))
        }) {
            self.history.push((self.dir, entry, self.scroll));
            self.dir = child;
            self.selected = 0;
            self.scroll = 0;
            self.filter.clear();
            self.rebuild(tree);
        }
    }
    pub(super) fn parent(&mut self, tree: &Tree) {
        if let Some((dir, entry, scroll)) = self.history.pop() {
            self.dir = dir;
            self.filter.clear();
            self.rebuild(tree);
            self.selected = self
                .visible
                .iter()
                .position(|&index| index == entry)
                .unwrap_or(0);
            self.scroll = scroll.min(self.selected);
        }
    }
}
