//! Per-project workspace context for JetBrains-style multi-project nested tabs.
//!
//! Each open project owns its own:
//! - Project-scoped left sidebar (`FileTreeState`)
//! - Nested editor file tabs (`Vec<TextBuffer>`)
//! - Durable Pi Coding Agent session (`PiPanelState` backed by `<root>/.pi/`)

use std::path::{Path, PathBuf};

use atom_editor::TextBuffer;

use crate::pi_panel::PiPanelState;
use crate::status_bar::detect_git_branch;
use crate::tree_view::FileTreeState;

pub struct ProjectContext {
    pub name: String,
    pub root: PathBuf,
    pub git_branch: Option<String>,
    pub tree_view: FileTreeState,
    pub tabs: Vec<TextBuffer>,
    pub active_tab: usize,
    pub editor_scroll_line: usize,
    pub pi_panel: PiPanelState,
    pub status_message: Option<String>,
}

impl ProjectContext {
    pub fn new(root: PathBuf, initial_file: Option<PathBuf>) -> Self {
        let canonical = std::fs::canonicalize(&root).unwrap_or(root);
        let name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();
        let git_branch = detect_git_branch(&canonical);
        let tree_view = FileTreeState::new(canonical.clone());
        let pi_panel = PiPanelState::new(canonical.clone());

        let mut tabs = Vec::new();
        if let Some(file) = initial_file
            && let Ok(buf) = TextBuffer::from_file(&file)
        {
            tabs.push(buf);
        }
        if tabs.is_empty() {
            let readme = canonical.join("README.md");
            if readme.exists()
                && let Ok(buf) = TextBuffer::from_file(&readme)
            {
                tabs.push(buf);
            } else {
                tabs.push(TextBuffer::new_untitled(
                    "welcome.md",
                    &format!(
                        "# {name} — Atom GPUI + Durable Pi\n\n\
                         - Left sidebar is scoped to `{}`\n\
                         - Top outer tabs switch between open projects (JetBrains-style, `Ctrl+Shift+O` to open another)\n\
                         - Nested inner tabs switch between files in this project\n\
                         - Right dock runs a durable Pi Coding Agent persisted under `.pi/`\n",
                        canonical.display()
                    ),
                ));
            }
        }

        Self {
            name,
            root: canonical,
            git_branch,
            tree_view,
            tabs,
            active_tab: 0,
            editor_scroll_line: 0,
            pi_panel,
            status_message: None,
        }
    }

    pub fn active_buffer(&self) -> Option<&TextBuffer> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut TextBuffer> {
        self.tabs.get_mut(self.active_tab)
    }

    pub fn open_file(&mut self, path: PathBuf) {
        if let Some(idx) = self.tabs.iter().position(|t| t.path() == Some(path.as_path())) {
            self.active_tab = idx;
            self.editor_scroll_line = 0;
            return;
        }
        match TextBuffer::from_file(&path) {
            Ok(buf) => {
                self.tabs.push(buf);
                self.active_tab = self.tabs.len() - 1;
                self.editor_scroll_line = 0;
                self.status_message = None;
            }
            Err(err) => {
                self.status_message = Some(format!("Open error: {err}"));
            }
        }
    }

    pub fn new_file(&mut self) {
        let num = self.tabs.len() + 1;
        self.tabs
            .push(TextBuffer::new_untitled(format!("untitled-{num}"), ""));
        self.active_tab = self.tabs.len() - 1;
        self.editor_scroll_line = 0;
    }

    pub fn save_active_file(&mut self) {
        if let Some(buf) = self.active_buffer_mut() {
            match buf.save() {
                Ok(()) => {
                    self.status_message = Some(format!("Saved {}", buf.title()));
                }
                Err(err) => {
                    self.status_message = Some(format!("Save failed: {err}"));
                }
            }
        }
    }

    pub fn close_tab(&mut self, idx: usize) {
        if self.tabs.is_empty() || idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.new_file();
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
    }

    pub fn reload_if_open(&mut self, path: &Path) {
        for tab in &mut self.tabs {
            if tab.path() == Some(path) {
                let _ = tab.reload_from_disk();
                self.status_message = Some(format!("Pi updated {}", tab.title()));
            }
        }
    }

    pub fn ensure_cursor_visible(&mut self) {
        if let Some(buf) = self.active_buffer() {
            let line = buf.cursor().line;
            if line < self.editor_scroll_line {
                self.editor_scroll_line = line;
            } else if line >= self.editor_scroll_line + 38 {
                self.editor_scroll_line = line.saturating_sub(37);
            }
        }
    }
}

/// Discovers sibling directories next to `current_root` for quick project switching.
pub fn discover_sibling_projects(current_root: &Path) -> Vec<PathBuf> {
    let Some(parent) = current_root.parent() else {
        return Vec::new();
    };
    let Ok(read_dir) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = read_dir
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            if p.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    dirs.sort();
    dirs
}
