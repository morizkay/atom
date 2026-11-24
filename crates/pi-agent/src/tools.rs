//! Pi's iconic 4-tool core: `read`, `write`, `edit`, and `bash`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub output: String,
    pub is_error: bool,
    pub modified_file: Option<PathBuf>,
}

/// Returns OpenAI-style tool definitions for Pi's 4 core tools.
pub fn openai_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read",
                "description": "Read a file from the workspace with optional 1-based line offset and line limit.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path relative to workspace root or absolute." },
                        "offset": { "type": "integer", "description": "1-based starting line number (optional)." },
                        "limit": { "type": "integer", "description": "Maximum number of lines to return (optional, default 200)." }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write",
                "description": "Create or overwrite a file in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to write." },
                        "content": { "type": "string", "description": "Complete file contents." }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "edit",
                "description": "Edit an existing file by replacing an exact unique substring `old_text` with `new_text`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to edit." },
                        "old_text": { "type": "string", "description": "Exact existing text to replace." },
                        "new_text": { "type": "string", "description": "Replacement text." }
                    },
                    "required": ["path", "old_text", "new_text"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Execute a bash command in the workspace root directory.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute." }
                    },
                    "required": ["command"]
                }
            }
        }),
    ]
}

/// Returns Anthropic-style tool definitions for Pi's 4 core tools.
pub fn anthropic_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "read",
            "description": "Read a file from the workspace with optional 1-based line offset and line limit.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to workspace root or absolute." },
                    "offset": { "type": "integer", "description": "1-based starting line number." },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return (default 200)." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "write",
            "description": "Create or overwrite a file in the workspace.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write." },
                    "content": { "type": "string", "description": "Complete file contents." }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "edit",
            "description": "Edit an existing file by replacing an exact unique substring `old_text` with `new_text`.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to edit." },
                    "old_text": { "type": "string", "description": "Exact existing text to replace." },
                    "new_text": { "type": "string", "description": "Replacement text." }
                },
                "required": ["path", "old_text", "new_text"]
            }
        }),
        json!({
            "name": "bash",
            "description": "Execute a bash command in the workspace root directory.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute." }
                },
                "required": ["command"]
            }
        }),
    ]
}

fn resolve_path(workspace_root: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    }
}

/// Executes one of Pi's 4 core tools (`read`, `write`, `edit`, `bash`).
pub async fn execute_tool(workspace_root: &Path, call: &ToolCall) -> ToolExecutionResult {
    match call.name.as_str() {
        "read" => run_read(workspace_root, &call.arguments),
        "write" => run_write(workspace_root, &call.arguments),
        "edit" => run_edit(workspace_root, &call.arguments),
        "bash" => run_bash(workspace_root, &call.arguments).await,
        other => ToolExecutionResult {
            output: format!("Unknown tool `{other}`. Available tools: read, write, edit, bash."),
            is_error: true,
            modified_file: None,
        },
    }
}

fn run_read(workspace_root: &Path, args: &Value) -> ToolExecutionResult {
    let Some(path_str) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `path`".into(),
            is_error: true,
            modified_file: None,
        };
    };
    let path = resolve_path(workspace_root, path_str);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            return ToolExecutionResult {
                output: format!("Failed to read {}: {err}", path.display()),
                is_error: true,
                modified_file: None,
            };
        }
    };

    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .min(1000) as usize;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start_idx = offset.saturating_sub(1).min(total);
    let end_idx = (start_idx + limit).min(total);

    let mut out = format!("File: {} ({total} lines)\n", path.display());
    for (idx, line) in lines[start_idx..end_idx].iter().enumerate() {
        out.push_str(&format!("{:>4} | {line}\n", start_idx + idx + 1));
    }
    if end_idx < total {
        out.push_str(&format!("... ({} more lines)\n", total - end_idx));
    }

    ToolExecutionResult {
        output: out,
        is_error: false,
        modified_file: None,
    }
}

fn run_write(workspace_root: &Path, args: &Value) -> ToolExecutionResult {
    let Some(path_str) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `path`".into(),
            is_error: true,
            modified_file: None,
        };
    };
    let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `content`".into(),
            is_error: true,
            modified_file: None,
        };
    };
    let path = resolve_path(workspace_root, path_str);
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return ToolExecutionResult {
            output: format!("Failed to create parent directories: {err}"),
            is_error: true,
            modified_file: None,
        };
    }
    match std::fs::write(&path, content) {
        Ok(()) => ToolExecutionResult {
            output: format!("Wrote {} bytes to {}", content.len(), path.display()),
            is_error: false,
            modified_file: Some(path),
        },
        Err(err) => ToolExecutionResult {
            output: format!("Failed to write {}: {err}", path.display()),
            is_error: true,
            modified_file: None,
        },
    }
}

fn run_edit(workspace_root: &Path, args: &Value) -> ToolExecutionResult {
    let Some(path_str) = args.get("path").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `path`".into(),
            is_error: true,
            modified_file: None,
        };
    };
    let Some(old_text) = args.get("old_text").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `old_text`".into(),
            is_error: true,
            modified_file: None,
        };
    };
    let Some(new_text) = args.get("new_text").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `new_text`".into(),
            is_error: true,
            modified_file: None,
        };
    };

    let path = resolve_path(workspace_root, path_str);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            return ToolExecutionResult {
                output: format!("Failed to read {}: {err}", path.display()),
                is_error: true,
                modified_file: None,
            };
        }
    };

    let matches = content.matches(old_text).count();
    if matches == 0 {
        return ToolExecutionResult {
            output: format!("`old_text` not found in {}", path.display()),
            is_error: true,
            modified_file: None,
        };
    }
    if matches > 1 {
        return ToolExecutionResult {
            output: format!(
                "`old_text` matched {matches} times in {}; provide more surrounding context.",
                path.display()
            ),
            is_error: true,
            modified_file: None,
        };
    }

    let updated = content.replacen(old_text, new_text, 1);
    match std::fs::write(&path, updated) {
        Ok(()) => ToolExecutionResult {
            output: format!("Edited {}", path.display()),
            is_error: false,
            modified_file: Some(path),
        },
        Err(err) => ToolExecutionResult {
            output: format!("Failed to save edit to {}: {err}", path.display()),
            is_error: true,
            modified_file: None,
        },
    }
}

async fn run_bash(workspace_root: &Path, args: &Value) -> ToolExecutionResult {
    let Some(command_str) = args.get("command").and_then(|v| v.as_str()) else {
        return ToolExecutionResult {
            output: "Missing required argument `command`".into(),
            is_error: true,
            modified_file: None,
        };
    };

    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg(command_str)
        .current_dir(workspace_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(err)) => {
            return ToolExecutionResult {
                output: format!("Failed to spawn bash: {err}"),
                is_error: true,
                modified_file: None,
            };
        }
        Err(_) => {
            return ToolExecutionResult {
                output: "Command timed out after 60s".into(),
                is_error: true,
                modified_file: None,
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("[stderr]\n");
        combined.push_str(&stderr);
    }
    if combined.is_empty() {
        combined = format!("(exited with status {})", output.status);
    }
    // Cap output size to keep context clean
    if combined.len() > 16_000 {
        combined.truncate(16_000);
        combined.push_str("\n... [output truncated]");
    }

    ToolExecutionResult {
        output: combined,
        is_error: !output.status.success(),
        modified_file: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_write_edit_bash_cycle() {
        let dir = std::env::temp_dir().join(format!("pi_tools_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let write_res = execute_tool(
            &dir,
            &ToolCall {
                id: "1".into(),
                name: "write".into(),
                arguments: json!({"path": "hello.txt", "content": "hello world\nsecond line\n"}),
            },
        )
        .await;
        assert!(!write_res.is_error);
        assert!(write_res.modified_file.is_some());

        let edit_res = execute_tool(
            &dir,
            &ToolCall {
                id: "2".into(),
                name: "edit".into(),
                arguments: json!({"path": "hello.txt", "old_text": "world", "new_text": "atom"}),
            },
        )
        .await;
        assert!(!edit_res.is_error);

        let read_res = execute_tool(
            &dir,
            &ToolCall {
                id: "3".into(),
                name: "read".into(),
                arguments: json!({"path": "hello.txt", "offset": 1, "limit": 10}),
            },
        )
        .await;
        assert!(!read_res.is_error);
        assert!(read_res.output.contains("hello atom"));

        let bash_res = execute_tool(
            &dir,
            &ToolCall {
                id: "4".into(),
                name: "bash".into(),
                arguments: json!({"command": "cat hello.txt"}),
            },
        )
        .await;
        assert!(!bash_res.is_error);
        assert!(bash_res.output.contains("hello atom"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
