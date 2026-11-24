//! Fast, zero-dependency syntax tokenizer for Atom GPUI's One Dark / One Light themes.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Json,
    Toml,
    Markdown,
    Shell,
    C,
    Go,
    PlainText,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Language::Rust,
            "ts" | "tsx" | "mts" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" | "pyi" => Language::Python,
            "json" | "jsonc" => Language::Json,
            "toml" => Language::Toml,
            "md" | "markdown" => Language::Markdown,
            "sh" | "bash" | "zsh" => Language::Shell,
            "c" | "h" | "cpp" | "cc" | "hpp" => Language::C,
            "go" => Language::Go,
            _ => Language::PlainText,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::TypeScript => "TypeScript",
            Language::JavaScript => "JavaScript",
            Language::Python => "Python",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Markdown => "Markdown",
            Language::Shell => "Shell",
            Language::C => "C/C++",
            Language::Go => "Go",
            Language::PlainText => "Plain Text",
        }
    }

    fn line_comment_prefix(&self) -> Option<&'static str> {
        match self {
            Language::Rust
            | Language::TypeScript
            | Language::JavaScript
            | Language::C
            | Language::Go => Some("//"),
            Language::Python | Language::Toml | Language::Shell => Some("#"),
            Language::Json | Language::Markdown | Language::PlainText => None,
        }
    }

    fn keywords(&self) -> &'static [&'static str] {
        match self {
            Language::Rust => &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
                "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
                "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
                "super", "trait", "true", "type", "unsafe", "use", "where", "while",
            ],
            Language::TypeScript | Language::JavaScript => &[
                "as", "async", "await", "break", "case", "catch", "class", "const", "continue",
                "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally",
                "for", "from", "function", "if", "implements", "import", "in", "instanceof",
                "interface", "let", "new", "null", "of", "private", "protected", "public",
                "readonly", "return", "static", "super", "switch", "this", "throw", "true", "try",
                "type", "typeof", "undefined", "var", "void", "while", "yield",
            ],
            Language::Python => &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if",
                "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise",
                "return", "True", "try", "while", "with", "yield",
            ],
            Language::Go => &[
                "break", "case", "chan", "const", "continue", "default", "defer", "else",
                "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map",
                "package", "range", "return", "select", "struct", "switch", "type", "var", "true",
                "false", "nil",
            ],
            Language::C => &[
                "auto", "break", "case", "char", "const", "continue", "default", "do", "double",
                "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long",
                "namespace", "new", "nullptr", "private", "public", "return", "short", "signed",
                "sizeof", "static", "struct", "switch", "template", "typedef", "typename", "union",
                "unsigned", "using", "virtual", "void", "volatile", "while", "true", "false",
            ],
            Language::Shell => &[
                "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "case",
                "esac", "function", "local", "export", "return", "exit", "true", "false",
            ],
            Language::Json | Language::Toml => &["true", "false", "null"],
            Language::Markdown | Language::PlainText => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Comment,
    Keyword,
    String,
    Number,
    Function,
    Type,
    Operator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub text: String,
    pub kind: TokenKind,
}

/// Tokenizes a single line of code into colored spans.
pub fn highlight_line(line: &str, language: Language) -> Vec<SyntaxSpan> {
    if line.is_empty() {
        return Vec::new();
    }

    if language == Language::PlainText {
        return vec![SyntaxSpan {
            text: line.to_string(),
            kind: TokenKind::Plain,
        }];
    }

    if language == Language::Markdown {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            return vec![SyntaxSpan {
                text: line.to_string(),
                kind: TokenKind::Keyword,
            }];
        }
        if trimmed.starts_with("```") {
            return vec![SyntaxSpan {
                text: line.to_string(),
                kind: TokenKind::Comment,
            }];
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            return vec![SyntaxSpan {
                text: line.to_string(),
                kind: TokenKind::Function,
            }];
        }
    }

    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let comment_prefix = language.line_comment_prefix();
    let keywords = language.keywords();

    while i < len {
        // Line comments
        if let Some(prefix) = comment_prefix
            && line[i..].starts_with(prefix)
        {
            push_span(&mut spans, &line[i..], TokenKind::Comment);
            break;
        }

        let b = bytes[i];

        // Strings ("..." or '...')
        if b == b'"' || (b == b'\'' && language != Language::Rust) {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            push_span(&mut spans, &line[start..i], TokenKind::String);
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            push_span(&mut spans, &line[start..i], TokenKind::Number);
            continue;
        }

        // Identifiers / Keywords / Types / Function calls
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &line[start..i];
            let kind = if keywords.contains(&ident) {
                TokenKind::Keyword
            } else if i < len && bytes[i] == b'(' {
                TokenKind::Function
            } else if ident.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                TokenKind::Type
            } else {
                TokenKind::Plain
            };
            push_span(&mut spans, ident, kind);
            continue;
        }

        // Operators
        if matches!(
            b,
            b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'!' | b'&' | b'|' | b'^' | b'<' | b'>' | b':'
        ) {
            let start = i;
            i += 1;
            push_span(&mut spans, &line[start..i], TokenKind::Operator);
            continue;
        }

        // Advance one UTF-8 char
        let ch = line[i..].chars().next().unwrap();
        let next = i + ch.len_utf8();
        push_span(&mut spans, &line[i..next], TokenKind::Plain);
        i = next;
    }

    spans
}

fn push_span(spans: &mut Vec<SyntaxSpan>, text: &str, kind: TokenKind) {
    if let Some(last) = spans.last_mut()
        && last.kind == kind
    {
        last.text.push_str(text);
        return;
    }
    spans.push(SyntaxSpan {
        text: text.to_string(),
        kind,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_line() {
        let spans = highlight_line("pub fn hello(x: usize) -> String { // greet", Language::Rust);
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword && s.text == "pub"));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Function && s.text == "hello"));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Type && s.text == "String"));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
    }
}
