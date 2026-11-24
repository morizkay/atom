//! Atom GPUI + Built-in Durable Pi Coding Agent entry point.

mod command_palette;
mod pi_panel;
mod project;
mod status_bar;
mod tree_view;
mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, AppContext, Bounds, Focusable, KeyBinding, QuitMode, WindowBounds, WindowOptions, px,
    size,
};
use workspace::{
    AtomWorkspace, CloseTab, DeleteLine, EscapeOverlay, NewFile, NextProjectTab, OpenProjectModal,
    PrevProjectTab, Redo, SaveFile, ToggleCommandPalette, TogglePiPanel, ToggleTreeView, Undo,
};

fn main() {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to initialize tokio runtime"),
    );

    let initial_projects = parse_cli_args();

    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            cx.bind_keys([
                KeyBinding::new("ctrl-shift-p", ToggleCommandPalette, None),
                KeyBinding::new("cmd-shift-p", ToggleCommandPalette, None),
                KeyBinding::new("ctrl-shift-o", OpenProjectModal, None),
                KeyBinding::new("cmd-shift-o", OpenProjectModal, None),
                KeyBinding::new("ctrl-alt-right", NextProjectTab, None),
                KeyBinding::new("ctrl-alt-left", PrevProjectTab, None),
                KeyBinding::new("ctrl-\\", ToggleTreeView, None),
                KeyBinding::new("cmd-\\", ToggleTreeView, None),
                KeyBinding::new("ctrl-shift-a", TogglePiPanel, None),
                KeyBinding::new("cmd-shift-a", TogglePiPanel, None),
                KeyBinding::new("ctrl-s", SaveFile, None),
                KeyBinding::new("cmd-s", SaveFile, None),
                KeyBinding::new("ctrl-n", NewFile, None),
                KeyBinding::new("cmd-n", NewFile, None),
                KeyBinding::new("ctrl-w", CloseTab, None),
                KeyBinding::new("cmd-w", CloseTab, None),
                KeyBinding::new("ctrl-z", Undo, None),
                KeyBinding::new("cmd-z", Undo, None),
                KeyBinding::new("ctrl-shift-z", Redo, None),
                KeyBinding::new("cmd-shift-z", Redo, None),
                KeyBinding::new("ctrl-y", Redo, None),
                KeyBinding::new("ctrl-shift-k", DeleteLine, None),
                KeyBinding::new("escape", EscapeOverlay, None),
            ]);

            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let projects = initial_projects.clone();
            let rt = Arc::clone(&runtime);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |window, cx| {
                    let view = cx.new(|cx| AtomWorkspace::new(projects, rt, cx));
                    view.focus_handle(cx).focus(window, cx);
                    view
                },
            )
            .expect("failed to open Atom GPUI window");
        });
}

/// Parses one or more project directories or files from CLI args so multiple
/// projects can be opened directly as nested top tabs: `atom /proj/a /proj/b`.
fn parse_cli_args() -> Vec<(PathBuf, Option<PathBuf>)> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return vec![(cwd, None)];
    }

    let mut projects = Vec::new();
    for arg in args {
        let path = PathBuf::from(arg);
        let resolved = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        if resolved.is_dir() {
            projects.push((resolved, None));
        } else if resolved.is_file() {
            let root = resolved
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| cwd.clone());
            projects.push((root, Some(resolved)));
        }
    }

    if projects.is_empty() {
        projects.push((cwd, None));
    }
    projects
}
