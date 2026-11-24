//! Atom-style Command Palette state and fuzzy matcher.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommandId {
    OpenProject,
    NextProjectTab,
    PrevProjectTab,
    ToggleTreeView,
    TogglePiPanel,
    ToggleTheme,
    NewFile,
    SaveFile,
    CloseTab,
    ReloadActiveFile,
    PiResume,
    PiToggleSessions,
    PiNewSession,
    PiToggleBackend,
    PiCycleProvider,
    PiToggleTree,
}

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: PaletteCommandId,
    pub label: &'static str,
    pub shortcut: &'static str,
}

pub fn all_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            id: PaletteCommandId::OpenProject,
            label: "Project: Open Project Tab…",
            shortcut: "Ctrl+Shift+O",
        },
        PaletteCommand {
            id: PaletteCommandId::NextProjectTab,
            label: "Project: Next Project Tab",
            shortcut: "Ctrl+Alt+Right",
        },
        PaletteCommand {
            id: PaletteCommandId::PrevProjectTab,
            label: "Project: Previous Project Tab",
            shortcut: "Ctrl+Alt+Left",
        },
        PaletteCommand {
            id: PaletteCommandId::TogglePiPanel,
            label: "Pi Agent: Toggle Panel",
            shortcut: "Ctrl+Shift+A",
        },
        PaletteCommand {
            id: PaletteCommandId::PiResume,
            label: "Pi Agent: Resume Durable Run",
            shortcut: "/resume",
        },
        PaletteCommand {
            id: PaletteCommandId::PiToggleSessions,
            label: "Pi Agent: Browse Saved Sessions",
            shortcut: "/sessions",
        },
        PaletteCommand {
            id: PaletteCommandId::PiNewSession,
            label: "Pi Agent: New Session",
            shortcut: "/new",
        },
        PaletteCommand {
            id: PaletteCommandId::PiToggleBackend,
            label: "Pi Agent: Switch Backend (Native Rust / Pi RPC)",
            shortcut: "/backend",
        },
        PaletteCommand {
            id: PaletteCommandId::PiCycleProvider,
            label: "Pi Agent: Cycle LLM Provider",
            shortcut: "/provider",
        },
        PaletteCommand {
            id: PaletteCommandId::PiToggleTree,
            label: "Pi Agent: Toggle Session Tree (/tree)",
            shortcut: "/tree",
        },
        PaletteCommand {
            id: PaletteCommandId::ToggleTreeView,
            label: "Project Sidebar: Toggle Tree View",
            shortcut: "Ctrl+\\",
        },
        PaletteCommand {
            id: PaletteCommandId::ToggleTheme,
            label: "Theme: Toggle One Dark / One Light",
            shortcut: "",
        },
        PaletteCommand {
            id: PaletteCommandId::NewFile,
            label: "Application: New File",
            shortcut: "Ctrl+N",
        },
        PaletteCommand {
            id: PaletteCommandId::SaveFile,
            label: "Core: Save",
            shortcut: "Ctrl+S",
        },
        PaletteCommand {
            id: PaletteCommandId::CloseTab,
            label: "Pane: Close File Tab",
            shortcut: "Ctrl+W",
        },
        PaletteCommand {
            id: PaletteCommandId::ReloadActiveFile,
            label: "Editor: Reload From Disk",
            shortcut: "",
        },
    ]
}

#[derive(Debug, Clone, Default)]
pub struct CommandPaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

impl CommandPaletteState {
    pub fn open_fresh(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
    }

    pub fn filtered(&self) -> Vec<PaletteCommand> {
        let q = self.query.trim().to_ascii_lowercase();
        all_commands()
            .into_iter()
            .filter(|cmd| q.is_empty() || fuzzy_match(&cmd.label.to_ascii_lowercase(), &q))
            .collect()
    }

    pub fn move_up(&mut self) {
        let len = self.filtered().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if self.selected == 0 {
            len - 1
        } else {
            self.selected - 1
        };
    }

    pub fn move_down(&mut self) {
        let len = self.filtered().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + 1) % len;
    }

    pub fn current_command(&self) -> Option<PaletteCommandId> {
        let list = self.filtered();
        list.get(self.selected)
            .or_else(|| list.first())
            .map(|c| c.id)
    }
}

fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let mut h_chars = haystack.chars();
    for n in needle.chars() {
        if !h_chars.any(|h| h == n) {
            return false;
        }
    }
    true
}
