//! Terminal state independent of any GUI: scrollback, the input line with a cursor,
//! command history and paste handling. The egui widget only maps events onto this.

#[derive(Debug, Default, Clone)]
pub struct TermModel {
    pub scrollback: Vec<String>,
    input: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// `Some(i)` while browsing history; `draft` holds what was typed before.
    hist_pos: Option<usize>,
    draft: Vec<char>,
    /// IME composition text shown at the cursor but not yet part of the input.
    pub preedit: String,
}

const MAX_SCROLLBACK: usize = 5_000;

impl TermModel {
    pub fn input(&self) -> String {
        self.input.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_input(&mut self, s: &str) {
        self.input = s.chars().collect();
        self.cursor = self.input.len();
        self.hist_pos = None;
    }

    pub fn push_output(&mut self, text: &str) {
        for l in text.lines() {
            self.scrollback.push(l.to_string());
        }
        if self.scrollback.len() > MAX_SCROLLBACK {
            let extra = self.scrollback.len() - MAX_SCROLLBACK;
            self.scrollback.drain(..extra);
        }
    }

    /// Insert at the cursor. Control characters (including newlines) are dropped here:
    /// line breaks go through [`Self::paste`] or [`Self::take_line`].
    pub fn insert(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.input.insert(self.cursor, c);
            self.cursor += 1;
        }
        self.hist_pos = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.input.len());
    }

    /// Start of the previous word (words are runs of non-whitespace).
    fn word_start_before(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.input[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.input[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start_before();
    }

    /// Move to the end of the next word.
    pub fn word_right(&mut self) {
        let n = self.input.len();
        let mut i = self.cursor;
        while i < n && self.input[i].is_whitespace() {
            i += 1;
        }
        while i < n && !self.input[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    /// Delete the word before the cursor (Ctrl+W, Alt+Backspace).
    pub fn delete_word_back(&mut self) {
        let start = self.word_start_before();
        self.input.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.input.len();
    }

    pub fn clear_line(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.hist_pos = None;
    }

    /// Delete from the start of the line to the cursor (Ctrl+U).
    pub fn kill_to_start(&mut self) {
        self.input.drain(..self.cursor);
        self.cursor = 0;
    }

    /// Finish the line: returns it, records it in history, and clears the input.
    pub fn take_line(&mut self) -> String {
        let line = self.input();
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        self.clear_line();
        line
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let i = match self.hist_pos {
            None => {
                self.draft = self.input.clone();
                self.history.len() - 1
            }
            Some(i) => i.saturating_sub(1),
        };
        self.hist_pos = Some(i);
        self.input = self.history[i].chars().collect();
        self.cursor = self.input.len();
    }

    pub fn history_next(&mut self) {
        let Some(i) = self.hist_pos else { return };
        if i + 1 < self.history.len() {
            self.hist_pos = Some(i + 1);
            self.input = self.history[i + 1].chars().collect();
        } else {
            self.hist_pos = None;
            self.input = std::mem::take(&mut self.draft);
        }
        self.cursor = self.input.len();
    }

    /// Pasted text may contain several lines. Every completed line is returned, in order,
    /// for the caller to run one at a time; the trailing partial line stays in the input.
    pub fn paste(&mut self, text: &str) -> Vec<String> {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut parts = text.split('\n').peekable();
        let mut lines = Vec::new();
        while let Some(part) = parts.next() {
            self.insert(part);
            if parts.peek().is_some() {
                lines.push(self.take_line());
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_at_the_cursor() {
        let mut t = TermModel::default();
        t.insert("shw");
        t.left();
        t.insert("o");
        assert_eq!(t.input(), "show");
        t.home();
        t.delete();
        assert_eq!(t.input(), "how");
        t.end();
        t.backspace();
        assert_eq!(t.input(), "ho");
        t.kill_to_start();
        assert_eq!((t.input(), t.cursor()), (String::new(), 0));
    }

    #[test]
    fn word_movement_and_deletion() {
        let mut t = TermModel::default();
        t.insert("show  ip   route");
        t.word_left();
        assert_eq!(t.cursor(), 11, "start of `route`");
        t.word_left();
        assert_eq!(t.cursor(), 6, "start of `ip`");
        t.word_left();
        t.word_left();
        assert_eq!(t.cursor(), 0, "stops at the line start");
        t.word_right();
        assert_eq!(t.cursor(), 4, "end of `show`");
        t.word_right();
        assert_eq!(t.cursor(), 8, "end of `ip`");
        t.end();
        t.delete_word_back();
        assert_eq!(t.input(), "show  ip   ");
        t.delete_word_back();
        assert_eq!(t.input(), "show  ");
        t.delete_word_back();
        t.delete_word_back();
        assert_eq!(t.input(), "");
    }

    #[test]
    fn delete_word_back_in_the_middle_keeps_the_tail() {
        let mut t = TermModel::default();
        t.insert("conf term");
        for _ in 0..4 {
            t.left();
        }
        t.delete_word_back();
        assert_eq!((t.input().as_str(), t.cursor()), ("term", 0));
    }

    #[test]
    fn unicode_input_is_by_char_not_byte() {
        let mut t = TermModel::default();
        t.insert("é日");
        t.left();
        t.backspace();
        assert_eq!(t.input(), "日");
        assert_eq!(t.cursor(), 0);
    }

    #[test]
    fn control_characters_never_enter_the_input() {
        let mut t = TermModel::default();
        t.insert("a\tb\nc\u{7}");
        assert_eq!(t.input(), "abc");
    }

    #[test]
    fn history_walks_back_and_restores_the_draft() {
        let mut t = TermModel::default();
        for l in ["one", "two", "two", "three"] {
            t.insert(l);
            t.take_line();
        }
        t.insert("dra");
        t.history_prev();
        assert_eq!(t.input(), "three");
        t.history_prev();
        t.history_prev();
        assert_eq!(t.input(), "one", "duplicate `two` recorded once");
        t.history_prev();
        assert_eq!(t.input(), "one", "stops at the oldest");
        t.history_next();
        t.history_next();
        t.history_next();
        assert_eq!(t.input(), "dra", "draft restored");
    }

    #[test]
    fn blank_lines_are_not_recorded() {
        let mut t = TermModel::default();
        t.insert("  ");
        t.take_line();
        t.history_prev();
        assert_eq!(t.input(), "");
    }

    #[test]
    fn paste_returns_completed_lines_and_keeps_the_tail() {
        let mut t = TermModel::default();
        t.insert("conf");
        let lines = t.paste("igure terminal\r\nhostname r1\nint");
        assert_eq!(lines, vec!["configure terminal", "hostname r1"]);
        assert_eq!(t.input(), "int");
    }

    #[test]
    fn paste_with_trailing_newline_runs_the_last_line() {
        let mut t = TermModel::default();
        assert_eq!(t.paste("a\nb\n"), vec!["a", "b"]);
        assert_eq!(t.input(), "");
    }

    #[test]
    fn scrollback_is_bounded() {
        let mut t = TermModel::default();
        for i in 0..MAX_SCROLLBACK + 50 {
            t.push_output(&i.to_string());
        }
        assert_eq!(t.scrollback.len(), MAX_SCROLLBACK);
        assert_eq!(t.scrollback[0], "50");
    }
}
