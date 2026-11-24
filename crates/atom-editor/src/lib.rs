pub mod buffer;
pub mod highlight;

pub use buffer::{CursorPos, TextBuffer};
pub use highlight::{Language, SyntaxSpan, TokenKind, highlight_line};
