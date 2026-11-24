//! Authentic Atom One Dark and One Light color palettes for GPUI.
//!
//! Derived directly from Atom's `one-dark-ui`, `one-dark-syntax`,
//! `one-light-ui`, and `one-light-syntax` Less stylesheets.

use gpui::{Rgba, rgb, rgba};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeKind {
    #[default]
    OneDark,
    OneLight,
}

impl ThemeKind {
    pub fn name(&self) -> &'static str {
        match self {
            ThemeKind::OneDark => "One Dark",
            ThemeKind::OneLight => "One Light",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            ThemeKind::OneDark => ThemeKind::OneLight,
            ThemeKind::OneLight => ThemeKind::OneDark,
        }
    }
}

/// Complete UI + Syntax theme palette for Atom GPUI.
#[derive(Debug, Clone)]
pub struct AtomTheme {
    pub kind: ThemeKind,

    // Chrome & Surfaces
    pub app_bg: Rgba,
    pub editor_bg: Rgba,
    pub panel_bg: Rgba,
    pub surface_bg: Rgba,
    pub surface_hover: Rgba,
    pub surface_selected: Rgba,
    pub input_bg: Rgba,
    pub border: Rgba,
    pub border_focus: Rgba,

    // Tabs
    pub tab_bar_bg: Rgba,
    pub tab_bg: Rgba,
    pub tab_active_bg: Rgba,
    pub tab_active_accent: Rgba,

    // Text
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_subtle: Rgba,
    pub text_bright: Rgba,

    // Semantic accents
    pub accent: Rgba,
    pub info: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub error: Rgba,

    // Syntax highlighting (One Dark / One Light)
    pub syntax_fg: Rgba,
    pub syntax_comment: Rgba,
    pub syntax_keyword: Rgba,
    pub syntax_string: Rgba,
    pub syntax_number: Rgba,
    pub syntax_function: Rgba,
    pub syntax_type: Rgba,
    pub syntax_variable: Rgba,
    pub syntax_operator: Rgba,
    pub syntax_gutter: Rgba,
    pub syntax_selection: Rgba,
    pub syntax_line_highlight: Rgba,
    pub syntax_cursor: Rgba,
}

impl AtomTheme {
    pub fn for_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::OneDark => Self::one_dark(),
            ThemeKind::OneLight => Self::one_light(),
        }
    }

    /// Classic Atom One Dark palette (`hsl(220, 13%, 18%)` = `#282c34`).
    pub fn one_dark() -> Self {
        Self {
            kind: ThemeKind::OneDark,

            app_bg: rgb(0x21252b),
            editor_bg: rgb(0x282c34),
            panel_bg: rgb(0x21252b),
            surface_bg: rgb(0x2c313a),
            surface_hover: rgb(0x323842),
            surface_selected: rgb(0x3a404d),
            input_bg: rgb(0x1b1d23),
            border: rgb(0x181a1f),
            border_focus: rgb(0x528bff),

            tab_bar_bg: rgb(0x21252b),
            tab_bg: rgb(0x21252b),
            tab_active_bg: rgb(0x282c34),
            tab_active_accent: rgb(0x528bff),

            text: rgb(0xabb2bf),
            text_muted: rgb(0x7f848e),
            text_subtle: rgb(0x5c6370),
            text_bright: rgb(0xd7dae0),

            accent: rgb(0x528bff),
            info: rgb(0x61afef),
            success: rgb(0x98c379),
            warning: rgb(0xe5c07b),
            error: rgb(0xe06c75),

            syntax_fg: rgb(0xabb2bf),
            syntax_comment: rgb(0x5c6370),
            syntax_keyword: rgb(0xc678dd),
            syntax_string: rgb(0x98c379),
            syntax_number: rgb(0xd19a66),
            syntax_function: rgb(0x61afef),
            syntax_type: rgb(0xe5c07b),
            syntax_variable: rgb(0xe06c75),
            syntax_operator: rgb(0x56b6c2),
            syntax_gutter: rgb(0x636d83),
            syntax_selection: rgba(0x3e4451cc),
            syntax_line_highlight: rgba(0x2c313caa),
            syntax_cursor: rgb(0x528bff),
        }
    }

    /// Classic Atom One Light palette (`#fafafa`).
    pub fn one_light() -> Self {
        Self {
            kind: ThemeKind::OneLight,

            app_bg: rgb(0xeaeaeb),
            editor_bg: rgb(0xfafafa),
            panel_bg: rgb(0xeaeaeb),
            surface_bg: rgb(0xf0f0f1),
            surface_hover: rgb(0xe2e2e4),
            surface_selected: rgb(0xd7d7da),
            input_bg: rgb(0xffffff),
            border: rgb(0xd4d4d6),
            border_focus: rgb(0x526fff),

            tab_bar_bg: rgb(0xeaeaeb),
            tab_bg: rgb(0xeaeaeb),
            tab_active_bg: rgb(0xfafafa),
            tab_active_accent: rgb(0x526fff),

            text: rgb(0x383a42),
            text_muted: rgb(0x696c77),
            text_subtle: rgb(0xa0a1a7),
            text_bright: rgb(0x1b1d23),

            accent: rgb(0x526fff),
            info: rgb(0x4078f2),
            success: rgb(0x50a14f),
            warning: rgb(0xc18401),
            error: rgb(0xe45649),

            syntax_fg: rgb(0x383a42),
            syntax_comment: rgb(0xa0a1a7),
            syntax_keyword: rgb(0xa626a4),
            syntax_string: rgb(0x50a14f),
            syntax_number: rgb(0x986801),
            syntax_function: rgb(0x4078f2),
            syntax_type: rgb(0xc18401),
            syntax_variable: rgb(0xe45649),
            syntax_operator: rgb(0x0184bc),
            syntax_gutter: rgb(0x9d9d9f),
            syntax_selection: rgba(0xe5e5e6cc),
            syntax_line_highlight: rgba(0xf0f0f1aa),
            syntax_cursor: rgb(0x526fff),
        }
    }
}

impl Default for AtomTheme {
    fn default() -> Self {
        Self::one_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_theme_kind() {
        assert_eq!(ThemeKind::OneDark.toggle(), ThemeKind::OneLight);
        assert_eq!(ThemeKind::OneLight.toggle(), ThemeKind::OneDark);
    }

    #[test]
    fn one_dark_has_authentic_bg() {
        let theme = AtomTheme::one_dark();
        assert_eq!(theme.editor_bg, rgb(0x282c34));
        assert_eq!(theme.app_bg, rgb(0x21252b));
    }
}
