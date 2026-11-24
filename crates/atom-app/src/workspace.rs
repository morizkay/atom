//! Root Atom GPUI Workspace view:
//! - Outer JetBrains-style Multi-Project Top Tab Bar (each project has its own sidebar, nested file tabs, and durable Pi session)
//! - Project-scoped Left Sidebar (Tree View)
//! - Center Nested File Tab Bar + Syntax-Highlighted Code Editor
//! - Right Built-in Durable Pi Coding Agent Dock (with crash-safe resume, /sessions, and /tree)
//! - Open Project Modal, Command Palette Overlay, and Status Bar

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use atom_editor::{CursorPos, TokenKind, highlight_line};
use atom_theme::{AtomTheme, ThemeKind};
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, actions, div,
    prelude::FluentBuilder, px, rgb, rgba,
};
use tokio::runtime::Runtime;

use crate::command_palette::{CommandPaletteState, PaletteCommandId};
use crate::pi_panel::ChatEntry;
use crate::project::{ProjectContext, discover_sibling_projects};
use crate::status_bar::{StatusBarInfo, render_status_bar};

actions!(
    atom,
    [
        ToggleCommandPalette,
        OpenProjectModal,
        NextProjectTab,
        PrevProjectTab,
        ToggleTreeView,
        TogglePiPanel,
        SaveFile,
        NewFile,
        CloseTab,
        Undo,
        Redo,
        DeleteLine,
        EscapeOverlay,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    Editor,
    PiInput,
    CommandPalette,
    OpenProject,
}

#[derive(Debug, Clone, Default)]
struct OpenProjectModalState {
    open: bool,
    path_input: String,
    suggestions: Vec<PathBuf>,
    error: Option<String>,
}

pub struct AtomWorkspace {
    focus_handle: FocusHandle,
    focus_area: FocusArea,
    runtime: Arc<Runtime>,
    theme_kind: ThemeKind,
    theme: AtomTheme,

    show_tree_view: bool,
    show_pi_panel: bool,

    projects: Vec<ProjectContext>,
    active_project: usize,

    palette: CommandPaletteState,
    open_project_modal: OpenProjectModalState,
}

impl AtomWorkspace {
    pub fn new(
        initial_projects: Vec<(PathBuf, Option<PathBuf>)>,
        runtime: Arc<Runtime>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let mut projects: Vec<ProjectContext> = initial_projects
            .into_iter()
            .map(|(root, file)| ProjectContext::new(root, file))
            .collect();

        if projects.is_empty() {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            projects.push(ProjectContext::new(cwd, None));
        }

        // Background UI pump draining durable Pi events across ALL open projects concurrently
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let keep_going = this
                    .update(cx, |ws, cx| {
                        let mut needs_notify = false;
                        for proj in &mut ws.projects {
                            let (had_events, modified) = proj.pi_panel.drain_events(&ws.runtime);
                            if !modified.is_empty() {
                                proj.tree_view.refresh();
                                for path in modified {
                                    proj.reload_if_open(&path);
                                }
                                needs_notify = true;
                            }
                            if had_events || proj.pi_panel.is_running {
                                needs_notify = true;
                            }
                        }
                        if needs_notify {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        })
        .detach();

        Self {
            focus_handle,
            focus_area: FocusArea::Editor,
            runtime,
            theme_kind: ThemeKind::OneDark,
            theme: AtomTheme::one_dark(),
            show_tree_view: true,
            show_pi_panel: true,
            projects,
            active_project: 0,
            palette: CommandPaletteState::default(),
            open_project_modal: OpenProjectModalState::default(),
        }
    }

    fn project(&self) -> &ProjectContext {
        &self.projects[self.active_project]
    }

    fn project_mut(&mut self) -> &mut ProjectContext {
        &mut self.projects[self.active_project]
    }

    fn open_project_tab(&mut self, root: PathBuf) -> bool {
        let canonical = std::fs::canonicalize(&root).unwrap_or(root);
        if !canonical.is_dir() {
            self.open_project_modal.error =
                Some(format!("Not a directory: {}", canonical.display()));
            return false;
        }
        if let Some(idx) = self.projects.iter().position(|p| p.root == canonical) {
            self.active_project = idx;
            self.open_project_modal.open = false;
            self.focus_area = FocusArea::Editor;
            return true;
        }
        self.projects.push(ProjectContext::new(canonical, None));
        self.active_project = self.projects.len() - 1;
        self.open_project_modal.open = false;
        self.open_project_modal.error = None;
        self.focus_area = FocusArea::Editor;
        true
    }

    fn close_project_tab(&mut self, idx: usize) {
        if self.projects.len() <= 1 || idx >= self.projects.len() {
            return;
        }
        self.projects.remove(idx);
        if self.active_project >= self.projects.len() {
            self.active_project = self.projects.len() - 1;
        }
    }

    fn open_project_dialog(&mut self) {
        let current_root = self.project().root.clone();
        self.palette.close();
        self.open_project_modal.open = true;
        self.open_project_modal.path_input = current_root
            .parent()
            .map(|p| format!("{}/", p.display()))
            .unwrap_or_default();
        self.open_project_modal.suggestions = discover_sibling_projects(&current_root);
        self.open_project_modal.error = None;
        self.focus_area = FocusArea::OpenProject;
    }

    fn toggle_theme(&mut self) {
        self.theme_kind = self.theme_kind.toggle();
        self.theme = AtomTheme::for_kind(self.theme_kind);
    }

    fn execute_palette_command(&mut self, cmd: PaletteCommandId) {
        self.palette.close();
        self.focus_area = FocusArea::Editor;
        match cmd {
            PaletteCommandId::OpenProject => self.open_project_dialog(),
            PaletteCommandId::NextProjectTab => {
                if !self.projects.is_empty() {
                    self.active_project = (self.active_project + 1) % self.projects.len();
                }
            }
            PaletteCommandId::PrevProjectTab => {
                if !self.projects.is_empty() {
                    self.active_project = if self.active_project == 0 {
                        self.projects.len() - 1
                    } else {
                        self.active_project - 1
                    };
                }
            }
            PaletteCommandId::ToggleTreeView => self.show_tree_view = !self.show_tree_view,
            PaletteCommandId::TogglePiPanel => {
                self.show_pi_panel = !self.show_pi_panel;
                if self.show_pi_panel {
                    self.focus_area = FocusArea::PiInput;
                }
            }
            PaletteCommandId::ToggleTheme => self.toggle_theme(),
            PaletteCommandId::NewFile => self.project_mut().new_file(),
            PaletteCommandId::SaveFile => self.project_mut().save_active_file(),
            PaletteCommandId::CloseTab => {
                let idx = self.project().active_tab;
                self.project_mut().close_tab(idx);
            }
            PaletteCommandId::ReloadActiveFile => {
                if let Some(buf) = self.project_mut().active_buffer_mut() {
                    let _ = buf.reload_from_disk();
                }
            }
            PaletteCommandId::PiResume => {
                self.show_pi_panel = true;
                let active_file = self
                    .project()
                    .active_buffer()
                    .and_then(|b| b.path().map(Path::to_path_buf));
                let rt = Arc::clone(&self.runtime);
                self.project_mut().pi_panel.resume(active_file, &rt);
            }
            PaletteCommandId::PiToggleSessions => {
                self.show_pi_panel = true;
                let pi = &mut self.project_mut().pi_panel;
                pi.show_sessions = !pi.show_sessions;
                if pi.show_sessions {
                    pi.show_tree = false;
                    pi.refresh_saved_sessions();
                }
            }
            PaletteCommandId::PiNewSession => {
                self.show_pi_panel = true;
                let rt = Arc::clone(&self.runtime);
                self.project_mut().pi_panel.new_session(&rt);
            }
            PaletteCommandId::PiToggleBackend => {
                self.show_pi_panel = true;
                self.project_mut().pi_panel.toggle_backend();
            }
            PaletteCommandId::PiCycleProvider => {
                self.show_pi_panel = true;
                self.project_mut().pi_panel.cycle_provider();
            }
            PaletteCommandId::PiToggleTree => {
                self.show_pi_panel = true;
                let rt = Arc::clone(&self.runtime);
                let pi = &mut self.project_mut().pi_panel;
                pi.show_tree = !pi.show_tree;
                if pi.show_tree {
                    pi.show_sessions = false;
                    pi.refresh_tree_snapshot(&rt);
                }
            }
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.platform {
            return;
        }

        match self.focus_area {
            FocusArea::OpenProject => {
                match ks.key.as_str() {
                    "escape" => {
                        self.open_project_modal.open = false;
                        self.focus_area = FocusArea::Editor;
                    }
                    "enter" => {
                        let path = PathBuf::from(self.open_project_modal.path_input.trim());
                        self.open_project_tab(path);
                    }
                    "backspace" => {
                        self.open_project_modal.path_input.pop();
                        self.open_project_modal.error = None;
                    }
                    _ => {
                        if let Some(ch) = &ks.key_char
                            && !ch.chars().any(char::is_control)
                        {
                            self.open_project_modal.path_input.push_str(ch);
                            self.open_project_modal.error = None;
                        }
                    }
                }
                cx.notify();
            }
            FocusArea::CommandPalette => {
                match ks.key.as_str() {
                    "escape" => {
                        self.palette.close();
                        self.focus_area = FocusArea::Editor;
                    }
                    "up" => self.palette.move_up(),
                    "down" => self.palette.move_down(),
                    "enter" => {
                        if let Some(cmd) = self.palette.current_command() {
                            self.execute_palette_command(cmd);
                        }
                    }
                    "backspace" => {
                        self.palette.query.pop();
                        self.palette.selected = 0;
                    }
                    _ => {
                        if let Some(ch) = &ks.key_char
                            && !ch.chars().any(char::is_control)
                        {
                            self.palette.query.push_str(ch);
                            self.palette.selected = 0;
                        }
                    }
                }
                cx.notify();
            }
            FocusArea::PiInput => {
                match ks.key.as_str() {
                    "escape" => {
                        self.focus_area = FocusArea::Editor;
                    }
                    "enter" => {
                        let active_file = self
                            .project()
                            .active_buffer()
                            .and_then(|b| b.path().map(Path::to_path_buf));
                        let rt = Arc::clone(&self.runtime);
                        self.project_mut().pi_panel.submit(active_file, &rt);
                    }
                    "backspace" => {
                        self.project_mut().pi_panel.input.pop();
                    }
                    _ => {
                        if let Some(ch) = &ks.key_char
                            && !ch.chars().any(char::is_control)
                        {
                            self.project_mut().pi_panel.input.push_str(ch);
                        }
                    }
                }
                cx.notify();
            }
            FocusArea::Editor => {
                if let Some(buf) = self.project_mut().active_buffer_mut() {
                    match ks.key.as_str() {
                        "enter" => buf.insert_newline(),
                        "tab" => buf.insert_tab(),
                        "backspace" => buf.backspace(),
                        "delete" => buf.delete_forward(),
                        "left" => buf.move_left(),
                        "right" => buf.move_right(),
                        "up" => buf.move_up(),
                        "down" => buf.move_down(),
                        "home" => buf.move_line_start(),
                        "end" => buf.move_line_end(),
                        _ => {
                            if let Some(ch) = &ks.key_char
                                && !ch.chars().any(char::is_control)
                            {
                                buf.insert_str(ch);
                            }
                        }
                    }
                }
                self.project_mut().ensure_cursor_visible();
                cx.notify();
            }
        }
    }

    fn token_color(&self, kind: TokenKind) -> gpui::Rgba {
        match kind {
            TokenKind::Plain => self.theme.syntax_fg,
            TokenKind::Comment => self.theme.syntax_comment,
            TokenKind::Keyword => self.theme.syntax_keyword,
            TokenKind::String => self.theme.syntax_string,
            TokenKind::Number => self.theme.syntax_number,
            TokenKind::Function => self.theme.syntax_function,
            TokenKind::Type => self.theme.syntax_type,
            TokenKind::Operator => self.theme.syntax_operator,
        }
    }

    /// Outer JetBrains-style multi-project top tab bar.
    fn render_project_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let mut proj_tabs = div().flex().items_center().flex_1().overflow_hidden();

        for (idx, proj) in self.projects.iter().enumerate() {
            let is_active = idx == self.active_project;
            let pi_indicator = if proj.pi_panel.is_running {
                ("π ●", theme.warning)
            } else if proj.pi_panel.durable_status.is_resumable() {
                ("π ↻", theme.error)
            } else {
                ("π", theme.text_subtle)
            };

            let mut tab = div()
                .id(SharedString::from(format!("project-tab-{idx}")))
                .h(px(34.0))
                .px(px(12.0))
                .bg(if is_active {
                    theme.panel_bg
                } else {
                    theme.input_bg
                })
                .border_r_1()
                .border_color(theme.border)
                .border_t_2()
                .border_color(if is_active {
                    theme.accent
                } else {
                    theme.input_bg
                })
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_xs()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    this.active_project = idx;
                    this.focus_area = FocusArea::Editor;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_color(if is_active {
                            theme.text_bright
                        } else {
                            theme.text_muted
                        })
                        .child(format!("⚛ {}", proj.name)),
                );

            if let Some(branch) = &proj.git_branch {
                tab = tab.child(
                    div()
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .bg(theme.surface_bg)
                        .text_color(theme.info)
                        .child(format!("⎇ {branch}")),
                );
            }

            tab = tab.child(div().text_color(pi_indicator.1).child(pi_indicator.0));

            if self.projects.len() > 1 {
                tab = tab.child(
                    div()
                        .id(SharedString::from(format!("project-tab-close-{idx}")))
                        .text_color(theme.text_subtle)
                        .hover(|s| s.text_color(theme.error))
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            this.close_project_tab(idx);
                            cx.notify();
                        }))
                        .child("×"),
                );
            }

            proj_tabs = proj_tabs.child(tab);
        }

        let right_actions = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .child(
                div()
                    .id("btn-open-project")
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(3.0))
                    .bg(theme.surface_bg)
                    .hover(|s| s.bg(theme.surface_hover).text_color(theme.text_bright))
                    .text_xs()
                    .text_color(theme.text)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.open_project_dialog();
                        cx.notify();
                    }))
                    .child("+ Project"),
            )
            .child(
                div()
                    .id("btn-toggle-theme")
                    .px(px(7.0))
                    .py(px(3.0))
                    .rounded(px(3.0))
                    .text_xs()
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.surface_hover).text_color(theme.text_bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.toggle_theme();
                        cx.notify();
                    }))
                    .child(self.theme_kind.name()),
            )
            .child(
                div()
                    .id("btn-toggle-pi")
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(3.0))
                    .bg(if self.show_pi_panel {
                        theme.accent
                    } else {
                        theme.surface_bg
                    })
                    .text_xs()
                    .text_color(if self.show_pi_panel {
                        rgb(0xffffff)
                    } else {
                        theme.text
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.show_pi_panel = !this.show_pi_panel;
                        cx.notify();
                    }))
                    .child("π Pi"),
            );

        div()
            .h(px(34.0))
            .w_full()
            .bg(theme.input_bg)
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_between()
            .child(proj_tabs)
            .child(right_actions)
    }

    /// Project-scoped Left Sidebar (Tree View).
    fn render_project_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let proj = self.project();
        let active_path = proj.active_buffer().and_then(|b| b.path());

        let mut list = div().flex().flex_col().flex_1().overflow_hidden().py(px(4.0));

        for (idx, row) in proj.tree_view.rows.iter().take(60).enumerate() {
            let path = row.path.clone();
            let is_dir = row.is_dir;
            let icon = if is_dir {
                if row.expanded { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let is_active = active_path.is_some_and(|p| p == path.as_path());

            let row_bg = if is_active {
                theme.surface_selected
            } else {
                theme.panel_bg
            };
            let text_color = if is_active {
                theme.text_bright
            } else if is_dir {
                theme.text
            } else {
                theme.text_muted
            };

            list = list.child(
                div()
                    .id(SharedString::from(format!("tree-row-{idx}")))
                    .h(px(24.0))
                    .pl(px(10.0 + (row.depth as f32) * 14.0))
                    .pr(px(8.0))
                    .bg(row_bg)
                    .hover(|s| s.bg(theme.surface_hover))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(text_color)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        if is_dir {
                            this.project_mut().tree_view.toggle_dir(&path);
                        } else {
                            this.project_mut().open_file(path.clone());
                            this.focus_area = FocusArea::Editor;
                        }
                        cx.notify();
                    }))
                    .child(format!("{icon}{}", row.name)),
            );
        }

        div()
            .w(px(235.0))
            .h_full()
            .bg(theme.panel_bg)
            .border_r_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(30.0))
                    .px(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_bright)
                            .child(format!("PROJECT · {}", proj.name.to_uppercase())),
                    )
                    .child(
                        div()
                            .id("tree-refresh")
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .hover(|s| s.text_color(theme.text_bright))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                this.project_mut().tree_view.refresh();
                                cx.notify();
                            }))
                            .child("↻"),
                    ),
            )
            .child(list)
    }

    /// Inner (nested) File Tab Bar scoped to the active project.
    fn render_nested_file_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let proj = self.project();
        let mut tabs_row = div().flex().items_center().flex_1().overflow_hidden();

        for (idx, tab) in proj.tabs.iter().enumerate() {
            let active = idx == proj.active_tab;
            let title = if tab.is_dirty() {
                format!("{} •", tab.title())
            } else {
                tab.title().to_string()
            };

            tabs_row = tabs_row.child(
                div()
                    .id(SharedString::from(format!("file-tab-{idx}")))
                    .h(px(30.0))
                    .px(px(12.0))
                    .bg(if active {
                        theme.tab_active_bg
                    } else {
                        theme.tab_bg
                    })
                    .border_r_1()
                    .border_color(theme.border)
                    .border_b_2()
                    .border_color(if active {
                        theme.tab_active_accent
                    } else {
                        theme.tab_bg
                    })
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_xs()
                    .text_color(if active {
                        theme.text_bright
                    } else {
                        theme.text_muted
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        let p = this.project_mut();
                        p.active_tab = idx;
                        p.editor_scroll_line = 0;
                        this.focus_area = FocusArea::Editor;
                        cx.notify();
                    }))
                    .child(title)
                    .child(
                        div()
                            .id(SharedString::from(format!("file-tab-close-{idx}")))
                            .text_color(theme.text_subtle)
                            .hover(|s| s.text_color(theme.error))
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                this.project_mut().close_tab(idx);
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            );
        }

        div()
            .h(px(30.0))
            .w_full()
            .bg(theme.tab_bar_bg)
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_between()
            .child(tabs_row)
            .child(
                div()
                    .id("btn-new-tab")
                    .px(px(10.0))
                    .text_xs()
                    .text_color(theme.text_muted)
                    .hover(|s| s.text_color(theme.text_bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.project_mut().new_file();
                        this.focus_area = FocusArea::Editor;
                        cx.notify();
                    }))
                    .child("+"),
            )
    }

    fn render_editor_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let proj = self.project();
        let is_focused = self.focus_area == FocusArea::Editor;

        let mut lines_col = div()
            .id("editor-body")
            .flex_1()
            .w_full()
            .bg(theme.editor_bg)
            .py(px(8.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.focus_area = FocusArea::Editor;
                cx.notify();
            }));

        if let Some(buf) = proj.active_buffer() {
            let total_lines = buf.line_count();
            let start = proj.editor_scroll_line.min(total_lines.saturating_sub(1));
            let end = (start + 44).min(total_lines);
            let cursor = buf.cursor();
            let lang = buf.language();

            for line_idx in start..end {
                let text = buf.line_text(line_idx);
                let is_cursor_line = line_idx == cursor.line;
                let spans = highlight_line(&text, lang);

                let mut code_row = div().flex().items_center().text_sm();
                if spans.is_empty() {
                    if is_cursor_line && is_focused {
                        code_row = code_row.child(
                            div()
                                .w(px(2.0))
                                .h(px(16.0))
                                .bg(theme.syntax_cursor),
                        );
                    } else {
                        code_row = code_row.child(" ");
                    }
                } else {
                    for span in spans {
                        code_row = code_row.child(
                            div()
                                .text_color(self.token_color(span.kind))
                                .child(span.text),
                        );
                    }
                    if is_cursor_line && is_focused {
                        code_row = code_row.child(
                            div()
                                .ml(px(1.0))
                                .w(px(2.0))
                                .h(px(16.0))
                                .bg(theme.syntax_cursor),
                        );
                    }
                }

                let row_bg = if is_cursor_line {
                    theme.syntax_line_highlight
                } else {
                    rgba(0x00000000)
                };

                lines_col = lines_col.child(
                    div()
                        .id(SharedString::from(format!("editor-line-{line_idx}")))
                        .h(px(21.0))
                        .w_full()
                        .px(px(8.0))
                        .bg(row_bg)
                        .flex()
                        .items_center()
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            this.focus_area = FocusArea::Editor;
                            if let Some(b) = this.project_mut().active_buffer_mut() {
                                let col = b.line_char_len(line_idx);
                                b.set_cursor(line_idx, col);
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(44.0))
                                .pr(px(12.0))
                                .text_xs()
                                .text_color(if is_cursor_line {
                                    theme.text_bright
                                } else {
                                    theme.syntax_gutter
                                })
                                .child(format!("{:>4}", line_idx + 1)),
                        )
                        .child(code_row),
                );
            }
        }

        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.render_nested_file_tab_bar(cx))
            .child(lines_col)
    }

    fn render_pi_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let pi = &self.project().pi_panel;
        let is_focused = self.focus_area == FocusArea::PiInput;
        let is_resumable = pi.durable_status.is_resumable() && !pi.is_running;

        let header = div()
            .h(px(30.0))
            .px(px(10.0))
            .bg(theme.panel_bg)
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.accent)
                            .child("π DURABLE PI"),
                    )
                    .child(
                        div()
                            .id("pi-backend-chip")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(theme.surface_bg)
                            .text_xs()
                            .text_color(theme.text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                this.project_mut().pi_panel.toggle_backend();
                                cx.notify();
                            }))
                            .child(pi.config.backend.label()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .id("pi-sessions-btn")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(if pi.show_sessions {
                                theme.surface_selected
                            } else {
                                theme.surface_bg
                            })
                            .text_xs()
                            .text_color(theme.text_muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                let pi = &mut this.project_mut().pi_panel;
                                pi.show_sessions = !pi.show_sessions;
                                if pi.show_sessions {
                                    pi.show_tree = false;
                                    pi.refresh_saved_sessions();
                                }
                                cx.notify();
                            }))
                            .child("Sessions"),
                    )
                    .child(
                        div()
                            .id("pi-tree-btn")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(if pi.show_tree {
                                theme.surface_selected
                            } else {
                                theme.surface_bg
                            })
                            .text_xs()
                            .text_color(theme.text_muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                let rt = Arc::clone(&this.runtime);
                                let pi = &mut this.project_mut().pi_panel;
                                pi.show_tree = !pi.show_tree;
                                if pi.show_tree {
                                    pi.show_sessions = false;
                                    pi.refresh_tree_snapshot(&rt);
                                }
                                cx.notify();
                            }))
                            .child("/tree"),
                    )
                    .child(
                        div()
                            .id("pi-new-btn")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(theme.surface_bg)
                            .text_xs()
                            .text_color(theme.text_muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                let rt = Arc::clone(&this.runtime);
                                this.project_mut().pi_panel.new_session(&rt);
                                cx.notify();
                            }))
                            .child("+"),
                    ),
            );

        let subbar = div()
            .h(px(26.0))
            .px(px(10.0))
            .bg(theme.surface_bg)
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .child(
                div()
                    .id("pi-provider-chip")
                    .text_color(theme.info)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.project_mut().pi_panel.cycle_provider();
                        cx.notify();
                    }))
                    .child(format!(
                        "{} · {}",
                        pi.config.provider.provider.name(),
                        pi.config.provider.model
                    )),
            )
            .child(
                div()
                    .text_color(if pi.is_running {
                        theme.warning
                    } else if is_resumable {
                        theme.error
                    } else {
                        theme.success
                    })
                    .child(pi.durable_status.short_label()),
            );

        let mut body = div()
            .flex_1()
            .w_full()
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .overflow_hidden();

        // Resumable durable checkpoint banner
        if is_resumable {
            body = body.child(
                div()
                    .p(px(8.0))
                    .rounded(px(4.0))
                    .bg(theme.input_bg)
                    .border_1()
                    .border_color(theme.warning)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.warning)
                            .child("Durable checkpoint saved — ready to resume"),
                    )
                    .child(
                        div()
                            .id("pi-resume-banner-btn")
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(3.0))
                            .bg(theme.warning)
                            .text_xs()
                            .text_color(rgb(0x1b1d23))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _ev, _window, cx| {
                                let active_file = this
                                    .project()
                                    .active_buffer()
                                    .and_then(|b| b.path().map(Path::to_path_buf));
                                let rt = Arc::clone(&this.runtime);
                                this.project_mut().pi_panel.resume(active_file, &rt);
                                cx.notify();
                            }))
                            .child("Resume"),
                    ),
            );
        }

        // Persisted /sessions drawer
        if pi.show_sessions {
            let mut sess_box = div()
                .p(px(8.0))
                .rounded(px(4.0))
                .bg(theme.input_bg)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.accent)
                        .child("Durable Sessions (.pi/sessions/):"),
                );

            if pi.saved_sessions.is_empty() {
                sess_box = sess_box.child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child("(no saved sessions yet)"),
                );
            } else {
                for (i, s) in pi.saved_sessions.iter().take(8).enumerate() {
                    let sid = s.session_id.clone();
                    let short_id = &s.session_id[..s.session_id.len().min(8)];
                    sess_box = sess_box.child(
                        div()
                            .id(SharedString::from(format!("pi-sess-{i}")))
                            .px(px(6.0))
                            .py(px(3.0))
                            .rounded(px(3.0))
                            .hover(|st| st.bg(theme.surface_hover))
                            .text_xs()
                            .text_color(theme.text)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                let rt = Arc::clone(&this.runtime);
                                this.project_mut()
                                    .pi_panel
                                    .load_saved_session(&sid, &rt);
                                cx.notify();
                            }))
                            .child(format!("[{short_id}] ({} nodes) {}", s.node_count, s.preview)),
                    );
                }
            }
            body = body.child(sess_box);
        }

        // Optional /tree session branching drawer
        if pi.show_tree {
            let mut tree_box = div()
                .p(px(8.0))
                .rounded(px(4.0))
                .bg(theme.input_bg)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.accent)
                        .child("Session Tree (click node to branch):"),
                );

            if pi.tree_snapshot.is_empty() {
                tree_box = tree_box.child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child("(no turns recorded yet)"),
                );
            } else {
                for (i, (id, label, is_active)) in pi.tree_snapshot.iter().enumerate() {
                    let node_id = id.clone();
                    tree_box = tree_box.child(
                        div()
                            .id(SharedString::from(format!("pi-tree-node-{i}")))
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(3.0))
                            .bg(if *is_active {
                                theme.surface_selected
                            } else {
                                theme.input_bg
                            })
                            .text_xs()
                            .text_color(if *is_active {
                                theme.text_bright
                            } else {
                                theme.text_muted
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                let rt = Arc::clone(&this.runtime);
                                this.project_mut().pi_panel.checkout_node(&node_id, &rt);
                                cx.notify();
                            }))
                            .child(format!("{} {label}", if *is_active { "●" } else { "○" })),
                    );
                }
            }
            body = body.child(tree_box);
        }

        // Most recent chat entries
        let start_idx = pi.entries.len().saturating_sub(14);
        for entry in &pi.entries[start_idx..] {
            match entry {
                ChatEntry::User(msg) => {
                    body = body.child(
                        div()
                            .p(px(8.0))
                            .rounded(px(4.0))
                            .bg(theme.surface_selected)
                            .text_xs()
                            .text_color(theme.text_bright)
                            .child(format!("You: {msg}")),
                    );
                }
                ChatEntry::Assistant(msg) => {
                    body = body.child(
                        div()
                            .p(px(8.0))
                            .rounded(px(4.0))
                            .bg(theme.surface_bg)
                            .text_xs()
                            .text_color(theme.text)
                            .child(msg.clone()),
                    );
                }
                ChatEntry::Tool {
                    name,
                    args_preview,
                    output,
                    is_error,
                    ..
                } => {
                    let status_color = if output.is_none() {
                        theme.warning
                    } else if *is_error {
                        theme.error
                    } else {
                        theme.success
                    };
                    let mut card = div()
                        .p(px(6.0))
                        .rounded(px(4.0))
                        .bg(theme.input_bg)
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(status_color)
                                .child(format!("⚙ {name} {args_preview}")),
                        );
                    if let Some(out) = output {
                        card = card.child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(out.clone()),
                        );
                    }
                    body = body.child(card);
                }
                ChatEntry::System(msg) => {
                    body = body.child(
                        div()
                            .px(px(6.0))
                            .py(px(4.0))
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(msg.clone()),
                    );
                }
                ChatEntry::Error(err) => {
                    body = body.child(
                        div()
                            .p(px(6.0))
                            .rounded(px(4.0))
                            .bg(theme.input_bg)
                            .border_1()
                            .border_color(theme.error)
                            .text_xs()
                            .text_color(theme.error)
                            .child(format!("Error: {err}")),
                    );
                }
            }
        }

        let input_display = if pi.input.is_empty() {
            if pi.is_running {
                "Type to steer running agent, or click Stop…".to_string()
            } else {
                "Ask Pi anything (durable in .pi/) or type /help…".to_string()
            }
        } else if is_focused {
            format!("{}▏", pi.input)
        } else {
            pi.input.clone()
        };

        let composer = div()
            .p(px(10.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.panel_bg)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .id("pi-input-box")
                    .min_h(px(42.0))
                    .p(px(8.0))
                    .rounded(px(4.0))
                    .bg(theme.input_bg)
                    .border_1()
                    .border_color(if is_focused {
                        theme.border_focus
                    } else {
                        theme.border
                    })
                    .text_xs()
                    .text_color(if pi.input.is_empty() {
                        theme.text_subtle
                    } else {
                        theme.text_bright
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.focus_area = FocusArea::PiInput;
                        cx.notify();
                    }))
                    .child(input_display),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child("Enter to send · Esc to editor"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .when(pi.is_running, |d| {
                                d.child(
                                    div()
                                        .id("pi-stop-btn")
                                        .px(px(8.0))
                                        .py(px(3.0))
                                        .rounded(px(3.0))
                                        .bg(theme.error)
                                        .text_xs()
                                        .text_color(rgb(0xffffff))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _ev, _window, cx| {
                                            let rt = Arc::clone(&this.runtime);
                                            this.project().pi_panel.abort(&rt);
                                            cx.notify();
                                        }))
                                        .child("Stop"),
                                )
                            })
                            .child(
                                div()
                                    .id("pi-send-btn")
                                    .px(px(10.0))
                                    .py(px(3.0))
                                    .rounded(px(3.0))
                                    .bg(theme.accent)
                                    .text_xs()
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        let active_file = this
                                            .project()
                                            .active_buffer()
                                            .and_then(|b| b.path().map(Path::to_path_buf));
                                        let rt = Arc::clone(&this.runtime);
                                        this.project_mut().pi_panel.submit(active_file, &rt);
                                        cx.notify();
                                    }))
                                    .child(if pi.is_running { "Steer" } else { "Send" }),
                            ),
                    ),
            );

        div()
            .w(px(385.0))
            .h_full()
            .bg(theme.panel_bg)
            .border_l_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .child(header)
            .child(subbar)
            .child(body)
            .child(composer)
    }

    fn render_open_project_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let input_display = if self.open_project_modal.path_input.is_empty() {
            "Enter project directory path…".to_string()
        } else {
            format!("{}▏", self.open_project_modal.path_input)
        };

        let mut suggestions_col = div().flex().flex_col().py(px(4.0));
        for (i, dir) in self.open_project_modal.suggestions.iter().take(10).enumerate() {
            let p = dir.clone();
            let label = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("dir")
                .to_string();
            let full = dir.display().to_string();
            suggestions_col = suggestions_col.child(
                div()
                    .id(SharedString::from(format!("proj-suggest-{i}")))
                    .px(px(12.0))
                    .py(px(6.0))
                    .hover(|s| s.bg(theme.surface_hover))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.open_project_tab(p.clone());
                        cx.notify();
                    }))
                    .child(div().text_color(theme.text_bright).child(format!("⚛ {label}")))
                    .child(div().text_color(theme.text_subtle).child(full)),
            );
        }

        let mut modal = div()
            .w(px(560.0))
            .bg(theme.panel_bg)
            .border_1()
            .border_color(theme.border_focus)
            .rounded(px(6.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.accent)
                    .child("OPEN PROJECT IN NEW TOP TAB (Enter to open · Esc to cancel)"),
            )
            .child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .bg(theme.input_bg)
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .text_color(theme.text_bright)
                    .child(input_display),
            );

        if let Some(err) = &self.open_project_modal.error {
            modal = modal.child(
                div()
                    .px(px(12.0))
                    .py(px(6.0))
                    .text_xs()
                    .text_color(theme.error)
                    .child(err.clone()),
            );
        }

        modal = modal.child(suggestions_col);

        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .w_full()
            .h_full()
            .bg(rgba(0x00000088))
            .flex()
            .justify_center()
            .pt(px(64.0))
            .child(modal)
    }

    fn render_command_palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let commands = self.palette.filtered();
        let query_display = if self.palette.query.is_empty() {
            "Type a command…".to_string()
        } else {
            format!("{}▏", self.palette.query)
        };

        let mut list = div().flex().flex_col().py(px(4.0));
        for (idx, cmd) in commands.iter().enumerate() {
            let selected = idx == self.palette.selected;
            let cmd_id = cmd.id;
            list = list.child(
                div()
                    .id(SharedString::from(format!("palette-cmd-{idx}")))
                    .px(px(12.0))
                    .py(px(6.0))
                    .bg(if selected {
                        theme.surface_selected
                    } else {
                        theme.panel_bg
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.execute_palette_command(cmd_id);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_color(if selected {
                                theme.text_bright
                            } else {
                                theme.text
                            })
                            .child(cmd.label),
                    )
                    .child(div().text_color(theme.text_subtle).child(cmd.shortcut)),
            );
        }

        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .w_full()
            .h_full()
            .bg(rgba(0x00000088))
            .flex()
            .justify_center()
            .pt(px(64.0))
            .child(
                div()
                    .w(px(520.0))
                    .bg(theme.panel_bg)
                    .border_1()
                    .border_color(theme.border_focus)
                    .rounded(px(6.0))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(10.0))
                            .bg(theme.input_bg)
                            .border_b_1()
                            .border_color(theme.border)
                            .text_sm()
                            .text_color(if self.palette.query.is_empty() {
                                theme.text_subtle
                            } else {
                                theme.text_bright
                            })
                            .child(query_display),
                    )
                    .child(list),
            )
    }
}

impl Focusable for AtomWorkspace {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AtomWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let proj = self.project();
        let (cursor, language, dirty) = proj
            .active_buffer()
            .map(|b| (b.cursor(), b.language().display_name(), b.is_dirty()))
            .unwrap_or((CursorPos::default(), "Plain Text", false));

        let status_info = StatusBarInfo {
            workspace_name: proj.name.clone(),
            git_branch: proj.git_branch.clone(),
            cursor,
            language,
            dirty,
            theme_name: self.theme_kind.name(),
            pi_status: proj.pi_panel.status_label(),
            pi_running: proj.pi_panel.is_running,
            status_message: proj.status_message.clone(),
        };

        let mut main_row = div().flex_1().w_full().flex().overflow_hidden();
        if self.show_tree_view {
            main_row = main_row.child(self.render_project_sidebar(cx));
        }
        main_row = main_row.child(self.render_editor_pane(cx));
        if self.show_pi_panel {
            main_row = main_row.child(self.render_pi_panel(cx));
        }

        let mut root = div()
            .relative()
            .size_full()
            .bg(self.theme.app_bg)
            .text_color(self.theme.text)
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, _window, cx| {
                this.open_project_modal.open = false;
                if this.palette.open {
                    this.palette.close();
                    this.focus_area = FocusArea::Editor;
                } else {
                    this.palette.open_fresh();
                    this.focus_area = FocusArea::CommandPalette;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenProjectModal, _window, cx| {
                this.open_project_dialog();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextProjectTab, _window, cx| {
                if !this.projects.is_empty() {
                    this.active_project = (this.active_project + 1) % this.projects.len();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PrevProjectTab, _window, cx| {
                if !this.projects.is_empty() {
                    this.active_project = if this.active_project == 0 {
                        this.projects.len() - 1
                    } else {
                        this.active_project - 1
                    };
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleTreeView, _window, cx| {
                this.show_tree_view = !this.show_tree_view;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePiPanel, _window, cx| {
                this.show_pi_panel = !this.show_pi_panel;
                if this.show_pi_panel {
                    this.focus_area = FocusArea::PiInput;
                } else {
                    this.focus_area = FocusArea::Editor;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SaveFile, _window, cx| {
                this.project_mut().save_active_file();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewFile, _window, cx| {
                this.project_mut().new_file();
                this.focus_area = FocusArea::Editor;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _window, cx| {
                let idx = this.project().active_tab;
                this.project_mut().close_tab(idx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Undo, _window, cx| {
                if let Some(buf) = this.project_mut().active_buffer_mut() {
                    buf.undo();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Redo, _window, cx| {
                if let Some(buf) = this.project_mut().active_buffer_mut() {
                    buf.redo();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DeleteLine, _window, cx| {
                if let Some(buf) = this.project_mut().active_buffer_mut() {
                    buf.delete_line();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EscapeOverlay, _window, cx| {
                this.palette.close();
                this.open_project_modal.open = false;
                this.focus_area = FocusArea::Editor;
                cx.notify();
            }))
            .child(self.render_project_tab_bar(cx))
            .child(main_row)
            .child(render_status_bar(&self.theme, &status_info));

        if self.palette.open {
            root = root.child(self.render_command_palette(cx));
        } else if self.open_project_modal.open {
            root = root.child(self.render_open_project_modal(cx));
        }

        root
    }
}
