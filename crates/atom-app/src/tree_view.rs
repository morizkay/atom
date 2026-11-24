//! Atom-style File Tree sidebar state and directory scanner.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub struct FileTreeState {
    pub root: PathBuf,
    pub expanded_dirs: HashSet<PathBuf>,
    pub rows: Vec<TreeRow>,
}

const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", ".pi"];

impl FileTreeState {
    pub fn new(root: PathBuf) -> Self {
        let mut expanded_dirs = HashSet::new();
        expanded_dirs.insert(root.clone());
        let mut state = Self {
            root,
            expanded_dirs,
            rows: Vec::new(),
        };
        state.refresh();
        state
    }

    pub fn refresh(&mut self) {
        self.rows.clear();
        let root = self.root.clone();
        self.scan_dir(&root, 0);
    }

    pub fn toggle_dir(&mut self, path: &Path) {
        if self.expanded_dirs.contains(path) {
            self.expanded_dirs.remove(path);
        } else {
            self.expanded_dirs.insert(path.to_path_buf());
        }
        self.refresh();
    }

    fn scan_dir(&mut self, dir: &Path, depth: usize) {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return;
        };

        let mut entries: Vec<(PathBuf, String, bool)> = read_dir
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let name = entry.file_name().to_str()?.to_string();
                let is_dir = entry.file_type().ok()?.is_dir();
                if is_dir && IGNORED_DIRS.contains(&name.as_str()) {
                    return None;
                }
                Some((path, name, is_dir))
            })
            .collect();

        // Directories first, then alphabetical
        entries.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));

        for (path, name, is_dir) in entries {
            let expanded = is_dir && self.expanded_dirs.contains(&path);
            self.rows.push(TreeRow {
                path: path.clone(),
                name,
                depth,
                is_dir,
                expanded,
            });
            if expanded && depth < 6 {
                self.scan_dir(&path, depth + 1);
            }
        }
    }
}
