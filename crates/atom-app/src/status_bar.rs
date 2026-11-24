//! Atom One Dark / One Light bottom status bar rendering helper.

use atom_editor::CursorPos;
use atom_theme::AtomTheme;
use gpui::{IntoElement, ParentElement, Styled, div, px};

pub struct StatusBarInfo {
    pub workspace_name: String,
    pub git_branch: Option<String>,
    pub cursor: CursorPos,
    pub language: &'static str,
    pub dirty: bool,
    pub theme_name: &'static str,
    pub pi_status: String,
    pub pi_running: bool,
    pub status_message: Option<String>,
}

pub fn render_status_bar(theme: &AtomTheme, info: &StatusBarInfo) -> impl IntoElement {
    let mut left = div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .text_color(theme.text_bright)
                .child(format!("⚛ {}", info.workspace_name)),
        );

    if let Some(branch) = &info.git_branch {
        left = left.child(
            div()
                .text_color(theme.info)
                .child(format!("⎇ {branch}")),
        );
    }

    if let Some(msg) = &info.status_message {
        left = left.child(
            div()
                .text_color(theme.warning)
                .child(msg.clone()),
        );
    }

    let pi_color = if info.pi_running {
        theme.warning
    } else {
        theme.success
    };

    let right = div()
        .flex()
        .items_center()
        .gap(px(14.0))
        .child(
            div()
                .text_color(pi_color)
                .child(format!("π {}", info.pi_status)),
        )
        .child(div().text_color(theme.text_muted).child(format!(
            "Ln {}, Col {}{}",
            info.cursor.line + 1,
            info.cursor.col + 1,
            if info.dirty { " •" } else { "" }
        )))
        .child(div().text_color(theme.text).child(info.language))
        .child(div().text_color(theme.text_muted).child("UTF-8"))
        .child(div().text_color(theme.text_subtle).child(info.theme_name));

    div()
        .h(px(26.0))
        .w_full()
        .px(px(12.0))
        .bg(theme.panel_bg)
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .justify_between()
        .text_xs()
        .child(left)
        .child(right)
}

/// Reads the current git branch from `.git/HEAD` if available.
pub fn detect_git_branch(workspace_root: &std::path::Path) -> Option<String> {
    let head = std::fs::read_to_string(workspace_root.join(".git/HEAD")).ok()?;
    let trimmed = head.trim();
    if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
        Some(rest.to_string())
    } else {
        Some(trimmed.chars().take(7).collect())
    }
}
