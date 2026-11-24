//! Rope-backed text buffer with cursor navigation, editing, undo/redo, and file I/O.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ropey::Rope;

use crate::highlight::Language;

/// Zero-indexed line and column (in Unicode scalar values / chars).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorPos {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
struct EditSnapshot {
    text: String,
    cursor: CursorPos,
}

/// Core text buffer for an open Atom editor tab.
#[derive(Debug, Clone)]
pub struct TextBuffer {
    rope: Rope,
    path: Option<PathBuf>,
    title: String,
    language: Language,
    cursor: CursorPos,
    dirty: bool,
    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,
}

impl TextBuffer {
    pub fn new_untitled(title: impl Into<String>, initial_text: &str) -> Self {
        Self {
            rope: Rope::from_str(initial_text),
            path: None,
            title: title.into(),
            language: Language::PlainText,
            cursor: CursorPos::default(),
            dirty: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read file {}", path.display()))?;
        let title = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        let language = Language::from_path(&path);
        Ok(Self {
            rope: Rope::from_str(&content),
            path: Some(path),
            title,
            language,
            cursor: CursorPos::default(),
            dirty: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn reload_from_disk(&mut self) -> Result<()> {
        if let Some(path) = self.path.clone() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to reload file {}", path.display()))?;
            self.rope = Rope::from_str(&content);
            self.dirty = false;
            self.clamp_cursor();
        }
        Ok(())
    }

    pub fn save(&mut self) -> Result<()> {
        let path = self
            .path
            .as_ref()
            .context("cannot save untitled buffer without a path")?;
        std::fs::write(path, self.rope.to_string())
            .with_context(|| format!("failed to write file {}", path.display()))?;
        self.dirty = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, self.rope.to_string())
            .with_context(|| format!("failed to write file {}", path.display()))?;
        self.title = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        self.language = Language::from_path(&path);
        self.path = Some(path);
        self.dirty = false;
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn cursor(&self) -> CursorPos {
        self.cursor
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines().max(1)
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Returns a line without its trailing `\n` or `\r\n`.
    pub fn line_text(&self, line_idx: usize) -> String {
        if line_idx >= self.rope.len_lines() {
            return String::new();
        }
        let mut s = self.rope.line(line_idx).to_string();
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        s
    }

    pub fn line_char_len(&self, line_idx: usize) -> usize {
        self.line_text(line_idx).chars().count()
    }

    fn cursor_char_idx(&self) -> usize {
        let line = self.cursor.line.min(self.line_count().saturating_sub(1));
        let line_start = self.rope.line_to_char(line);
        let col = self.cursor.col.min(self.line_char_len(line));
        line_start + col
    }

    fn set_cursor_from_char_idx(&mut self, char_idx: usize) {
        let clamped = char_idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(clamped);
        let line_start = self.rope.line_to_char(line);
        self.cursor = CursorPos {
            line,
            col: clamped - line_start,
        };
    }

    fn clamp_cursor(&mut self) {
        let max_line = self.line_count().saturating_sub(1);
        self.cursor.line = self.cursor.line.min(max_line);
        let max_col = self.line_char_len(self.cursor.line);
        self.cursor.col = self.cursor.col.min(max_col);
    }

    fn push_undo(&mut self) {
        if self.undo_stack.len() >= 200 {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(EditSnapshot {
            text: self.rope.to_string(),
            cursor: self.cursor,
        });
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(EditSnapshot {
                text: self.rope.to_string(),
                cursor: self.cursor,
            });
            self.rope = Rope::from_str(&prev.text);
            self.cursor = prev.cursor;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(EditSnapshot {
                text: self.rope.to_string(),
                cursor: self.cursor,
            });
            self.rope = Rope::from_str(&next.text);
            self.cursor = next.cursor;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.push_undo();
        let idx = self.cursor_char_idx();
        self.rope.insert(idx, s);
        self.set_cursor_from_char_idx(idx + s.chars().count());
        self.dirty = true;
    }

    pub fn insert_newline(&mut self) {
        // Preserve leading indentation of the current line
        let current = self.line_text(self.cursor.line);
        let indent: String = current
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let mut to_insert = String::with_capacity(1 + indent.len());
        to_insert.push('\n');
        to_insert.push_str(&indent);
        self.insert_str(&to_insert);
    }

    pub fn insert_tab(&mut self) {
        self.insert_str("    ");
    }

    pub fn backspace(&mut self) {
        let idx = self.cursor_char_idx();
        if idx == 0 {
            return;
        }
        self.push_undo();
        self.rope.remove((idx - 1)..idx);
        self.set_cursor_from_char_idx(idx - 1);
        self.dirty = true;
    }

    pub fn delete_forward(&mut self) {
        let idx = self.cursor_char_idx();
        if idx >= self.rope.len_chars() {
            return;
        }
        self.push_undo();
        self.rope.remove(idx..(idx + 1));
        self.clamp_cursor();
        self.dirty = true;
    }

    pub fn delete_line(&mut self) {
        if self.rope.len_chars() == 0 {
            return;
        }
        self.push_undo();
        let line = self.cursor.line.min(self.line_count().saturating_sub(1));
        let start = self.rope.line_to_char(line);
        let end = if line + 1 < self.rope.len_lines() {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        if start < end {
            self.rope.remove(start..end);
        }
        self.clamp_cursor();
        self.dirty = true;
    }

    pub fn move_left(&mut self) {
        let idx = self.cursor_char_idx();
        if idx > 0 {
            self.set_cursor_from_char_idx(idx - 1);
        }
    }

    pub fn move_right(&mut self) {
        let idx = self.cursor_char_idx();
        if idx < self.rope.len_chars() {
            self.set_cursor_from_char_idx(idx + 1);
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.cursor.col.min(self.line_char_len(self.cursor.line));
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor.line + 1 < self.line_count() {
            self.cursor.line += 1;
            self.cursor.col = self.cursor.col.min(self.line_char_len(self.cursor.line));
        }
    }

    pub fn move_line_start(&mut self) {
        self.cursor.col = 0;
    }

    pub fn move_line_end(&mut self) {
        self.cursor.col = self.line_char_len(self.cursor.line);
    }

    pub fn set_cursor(&mut self, line: usize, col: usize) {
        self.cursor = CursorPos { line, col };
        self.clamp_cursor();
    }

    pub fn goto_line(&mut self, one_based_line: usize) {
        let line = one_based_line.saturating_sub(1);
        self.set_cursor(line, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_backspace_and_undo_redo() {
        let mut buf = TextBuffer::new_untitled("test.rs", "fn main() {\n}");
        buf.set_cursor(0, 11);
        buf.insert_newline();
        buf.insert_str("    println!(\"hi\");");
        assert!(buf.is_dirty());
        assert_eq!(buf.line_count(), 3);

        buf.backspace();
        assert!(buf.line_text(1).ends_with(')'));

        assert!(buf.undo());
        assert!(buf.line_text(1).ends_with(';'));

        assert!(buf.redo());
        assert!(buf.line_text(1).ends_with(')'));
    }

    #[test]
    fn delete_line_and_navigation() {
        let mut buf = TextBuffer::new_untitled("a.txt", "one\ntwo\nthree");
        buf.move_down();
        assert_eq!(buf.cursor().line, 1);
        buf.delete_line();
        assert_eq!(buf.text(), "one\nthree");
    }
}
