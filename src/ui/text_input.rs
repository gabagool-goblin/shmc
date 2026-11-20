//! The config screen's line editor. The cursor is a char index, never a byte offset.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    /// The cursor is clamped to the text.
    pub fn at(text: impl Into<String>, cursor: usize) -> Self {
        let mut input = Self::new(text);
        input.cursor = cursor.min(input.cursor);
        input
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    /// The caller has already dropped control characters.
    pub fn insert_str(&mut self, s: &str) {
        let at = self.byte(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    fn remove(&mut self, from: usize, to: usize) {
        let (a, b) = (self.byte(from), self.byte(to));
        self.text.replace_range(a..b, "");
        self.cursor = from;
    }

    /// Letters and digits are one word, punctuation another, so `http://x` is not one.
    fn word_start(&self, from: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = from.min(chars.len());
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let word = chars[i - 1].is_alphanumeric();
            while i > 0 && !chars[i - 1].is_whitespace() && chars[i - 1].is_alphanumeric() == word {
                i -= 1;
            }
        }
        i
    }

    fn word_end(&self, from: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = from.min(chars.len());
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() {
            let word = chars[i].is_alphanumeric();
            while i < chars.len() && !chars[i].is_whitespace() && chars[i].is_alphanumeric() == word
            {
                i += 1;
            }
        }
        i
    }

    /// Apply an editing key; true when the text changed (not for a cursor move).
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let len = self.char_count();
        match key.code {
            KeyCode::Left if alt || ctrl => self.cursor = self.word_start(self.cursor),
            KeyCode::Right if alt || ctrl => self.cursor = self.word_end(self.cursor),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = len,
            KeyCode::Backspace if self.cursor > 0 => {
                self.remove(self.cursor - 1, self.cursor);
                return true;
            }
            KeyCode::Delete if self.cursor < len => {
                self.remove(self.cursor, self.cursor + 1);
                return true;
            }
            KeyCode::Char('u') if ctrl && self.cursor > 0 => {
                self.remove(0, self.cursor);
                return true;
            }
            KeyCode::Char('w') if ctrl && self.cursor > 0 => {
                self.remove(self.word_start(self.cursor), self.cursor);
                return true;
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.insert(c);
                return true;
            }
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn typing_goes_in_at_the_cursor() {
        let mut t = TextInput::new("ac");
        t.handle(key(KeyCode::Left));
        assert!(t.handle(key(KeyCode::Char('b'))));
        assert_eq!((t.text(), t.cursor()), ("abc", 2));
        t.insert_str("XY");
        assert_eq!((t.text(), t.cursor()), ("abXYc", 4));
    }

    #[test]
    fn backspace_and_delete_stop_at_the_edges() {
        let mut t = TextInput::new("ab");
        t.handle(key(KeyCode::Home));
        assert!(!t.handle(key(KeyCode::Backspace)));
        assert!(t.handle(key(KeyCode::Delete)));
        assert_eq!(t.text(), "b");
        t.handle(key(KeyCode::End));
        assert!(!t.handle(key(KeyCode::Delete)));
        assert!(t.handle(key(KeyCode::Backspace)));
        assert_eq!((t.text(), t.cursor()), ("", 0));
        assert!(!t.handle(key(KeyCode::Backspace)));
    }

    #[test]
    fn ctrl_u_w_cut_before_cursor() {
        let mut t = TextInput::at("one two  three", 8);
        assert!(t.handle(ctrl('w')));
        assert_eq!((t.text(), t.cursor()), ("one  three", 4));
        let mut t = TextInput::at("http://host:32400", 17);
        t.handle(ctrl('w'));
        assert_eq!(t.text(), "http://host:");
        t.handle(ctrl('w'));
        assert_eq!(t.text(), "http://host");
        t.handle(ctrl('w'));
        t.handle(ctrl('w'));
        assert_eq!(t.text(), "http");
        let mut t = TextInput::at("abc def", 4);
        assert!(t.handle(ctrl('u')));
        assert_eq!((t.text(), t.cursor()), ("def", 0));
        assert!(!t.handle(ctrl('u')));
        assert!(!t.handle(ctrl('w')));
    }

    #[test]
    fn cursor_keys_move_without_editing() {
        let mut t = TextInput::new("abc");
        assert!(!t.handle(ctrl('a')));
        assert_eq!(t.cursor(), 0);
        assert!(!t.handle(ctrl('e')));
        assert_eq!(t.cursor(), 3);
        t.handle(key(KeyCode::Home));
        assert_eq!(t.cursor(), 0);
        t.handle(key(KeyCode::Right));
        t.handle(key(KeyCode::Right));
        t.handle(key(KeyCode::Right));
        t.handle(key(KeyCode::Right));
        assert_eq!(t.cursor(), 3, "Right stops at the end");
        let alt_left = KeyEvent::new(KeyCode::Left, KeyModifiers::ALT);
        t.handle(alt_left);
        assert_eq!(t.cursor(), 0);
    }

    #[test]
    fn multibyte_and_combining_text_edits_by_char() {
        let mut t = TextInput::new("デスe\u{301}");
        for _ in 0..8 {
            t.handle(key(KeyCode::Left));
        }
        t.handle(key(KeyCode::Right));
        t.insert('x');
        assert_eq!(t.text(), "デxスe\u{301}");
        t.handle(key(KeyCode::Delete));
        t.handle(key(KeyCode::Backspace));
        assert_eq!(t.text(), "デe\u{301}");
        assert_eq!(t.cursor(), 1);
        let text = "a漢\u{1f600}e\u{301}";
        for at in 0..=text.chars().count() + 2 {
            let mut t = TextInput::at(text, at);
            t.insert('z');
            t.handle(key(KeyCode::Backspace));
            t.handle(key(KeyCode::Delete));
            t.handle(ctrl('w'));
            t.handle(ctrl('u'));
        }
    }
}
