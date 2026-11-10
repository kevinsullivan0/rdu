use ratatui::{crossterm::event::KeyCode, text::Line};
/// A review is display state, never deletion authority. Only the service can confirm a plan.
pub(super) struct Review {
    source: String,
    pub(super) phrase: String,
    pub(super) typed: String,
    pub(super) lines: Vec<String>,
    pub(super) scroll: usize,
    pub(super) seen_bottom: bool,
    pub(super) adequate: bool,
    pub(super) error: String,
    geometry: (u16, u16),
    visible: usize,
    seen: Vec<bool>,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReviewAction {
    None,
    Cancel,
    Confirm,
}
impl Review {
    pub(super) fn new(source: String, phrase: String) -> Self {
        Self {
            source,
            phrase,
            typed: String::new(),
            lines: Vec::new(),
            scroll: 0,
            seen_bottom: false,
            adequate: false,
            error: String::new(),
            geometry: (0, 0),
            visible: 0,
            seen: Vec::new(),
        }
    }
    pub(super) fn layout(&mut self, width: u16, height: u16) {
        if self.geometry == (width, height) {
            return;
        }
        self.geometry = (width, height);
        self.typed.clear();
        self.scroll = 0;
        self.seen_bottom = false;
        self.adequate = width >= 60 && height >= 14;
        self.visible = height.saturating_sub(12) as usize;
        self.lines = wrap(&self.source, width.saturating_sub(4).max(1) as usize);
        self.seen = vec![false; self.lines.len()];
        self.mark_visible();
    }
    fn mark_visible(&mut self) {
        for line in self.seen.iter_mut().skip(self.scroll).take(self.visible) {
            *line = true;
        }
        self.seen_bottom = self.adequate && self.seen.iter().all(|line| *line);
    }
    pub(super) fn key(&mut self, key: KeyCode) -> ReviewAction {
        let max = self.lines.len().saturating_sub(self.visible);
        match key {
            KeyCode::Esc => return ReviewAction::Cancel,
            KeyCode::Enter if !self.adequate => {
                self.error = "Enlarge terminal to review deletion".into();
            }
            KeyCode::Enter if self.typed.is_empty() => return ReviewAction::Cancel,
            KeyCode::Enter => {
                if self.adequate && self.seen_bottom && self.typed == self.phrase {
                    return ReviewAction::Confirm;
                }
                self.error = if !self.adequate {
                    "Enlarge terminal to review deletion"
                } else if !self.seen_bottom {
                    "Read every target: scroll to the end first"
                } else {
                    "Confirmation does not match"
                }
                .into();
            }
            KeyCode::Down => self.scroll = (self.scroll + 1).min(max),
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(self.visible.max(1)).min(max)
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.visible.max(1)),
            KeyCode::End => self.scroll = max,
            KeyCode::Home => self.scroll = 0,
            KeyCode::Backspace => {
                self.typed.pop();
            }
            KeyCode::Char(c) if self.adequate && !c.is_control() && self.typed.len() < 128 => {
                self.typed.push(c)
            }
            _ => {}
        }
        self.mark_visible();
        ReviewAction::None
    }
}
pub(super) fn wrap(source: &str, width: usize) -> Vec<String> {
    let mut output = Vec::new();
    for line in source.split('\n') {
        let mut row = String::new();
        let mut columns = 0;
        for c in line.chars() {
            let cell = Line::raw(c.to_string()).width();
            if columns + cell > width && !row.is_empty() {
                output.push(std::mem::take(&mut row));
                columns = 0;
            }
            row.push(c);
            columns += cell;
        }
        output.push(row);
    }
    output
}
