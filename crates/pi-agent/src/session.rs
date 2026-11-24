//! Tree-structured JSONL session store inspired by Pi's `/tree` session format.
//!
//! Every message node references its `parent_id`, allowing non-destructive branching,
//! navigating back to any prior turn, and persisting sessions as newline-delimited JSON.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
    ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRecord {
    pub tool_call_id: String,
    pub name: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: MessageRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultRecord>,
    pub timestamp: DateTime<Utc>,
}

impl SessionNode {
    pub fn preview(&self) -> String {
        let base = if !self.text.is_empty() {
            self.text.as_str()
        } else if let Some(first) = self.tool_calls.first() {
            first.name.as_str()
        } else if let Some(res) = &self.tool_result {
            res.name.as_str()
        } else {
            "(empty)"
        };
        let single_line = base.lines().next().unwrap_or("");
        if single_line.chars().count() > 56 {
            let truncated: String = single_line.chars().take(55).collect();
            format!("{truncated}…")
        } else {
            single_line.to_string()
        }
    }
}

/// Tree-structured Pi session.
#[derive(Debug, Clone, Default)]
pub struct SessionTree {
    pub session_id: String,
    nodes: Vec<SessionNode>,
    index_by_id: HashMap<String, usize>,
    active_leaf_id: Option<String>,
}

impl SessionTree {
    pub fn new() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            nodes: Vec::new(),
            index_by_id: HashMap::new(),
            active_leaf_id: None,
        }
    }

    pub fn active_leaf_id(&self) -> Option<&str> {
        self.active_leaf_id.as_deref()
    }

    pub fn all_nodes(&self) -> &[SessionNode] {
        &self.nodes
    }

    pub fn append_user(&mut self, text: impl Into<String>) -> String {
        self.append_node(MessageRole::User, text.into(), Vec::new(), None)
    }

    pub fn append_assistant(
        &mut self,
        text: impl Into<String>,
        tool_calls: Vec<ToolCallRecord>,
    ) -> String {
        self.append_node(MessageRole::Assistant, text.into(), tool_calls, None)
    }

    pub fn append_tool_result(&mut self, result: ToolResultRecord) -> String {
        self.append_node(MessageRole::ToolResult, String::new(), Vec::new(), Some(result))
    }

    fn append_node(
        &mut self,
        role: MessageRole,
        text: String,
        tool_calls: Vec<ToolCallRecord>,
        tool_result: Option<ToolResultRecord>,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let node = SessionNode {
            id: id.clone(),
            parent_id: self.active_leaf_id.clone(),
            role,
            text,
            tool_calls,
            tool_result,
            timestamp: Utc::now(),
        };
        let idx = self.nodes.len();
        self.nodes.push(node);
        self.index_by_id.insert(id.clone(), idx);
        self.active_leaf_id = Some(id.clone());
        id
    }

    /// Checkout any existing node to branch from that point (Pi `/tree` behavior).
    pub fn checkout(&mut self, node_id: &str) -> bool {
        if self.index_by_id.contains_key(node_id) {
            self.active_leaf_id = Some(node_id.to_string());
            true
        } else {
            false
        }
    }

    /// Returns the linear path from root to `active_leaf_id`.
    pub fn active_branch(&self) -> Vec<&SessionNode> {
        let mut path = Vec::new();
        let mut current = self.active_leaf_id.as_deref();
        while let Some(id) = current {
            if let Some(&idx) = self.index_by_id.get(id) {
                let node = &self.nodes[idx];
                path.push(node);
                current = node.parent_id.as_deref();
            } else {
                break;
            }
        }
        path.reverse();
        path
    }

    /// Saves the full session tree as strict JSONL (one JSON record per line, LF terminated).
    pub fn save_jsonl(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::new();
        for node in &self.nodes {
            let line = serde_json::to_string(node)?;
            out.push_str(&line);
            out.push('\n');
        }
        std::fs::write(path, out)
            .with_context(|| format!("failed to save session {}", path.display()))?;
        Ok(())
    }

    /// Loads a JSONL session file.
    pub fn load_jsonl(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to load session {}", path.display()))?;
        let mut tree = Self::new();
        for line in content.split('\n') {
            let line = line.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            let node: SessionNode = serde_json::from_str(line)?;
            let idx = tree.nodes.len();
            tree.active_leaf_id = Some(node.id.clone());
            tree.index_by_id.insert(node.id.clone(), idx);
            tree.nodes.push(node);
        }
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_branching_and_checkout() {
        let mut tree = SessionTree::new();
        let u1 = tree.append_user("hello");
        let _a1 = tree.append_assistant("hi there", vec![]);
        assert_eq!(tree.active_branch().len(), 2);

        // Branch from u1
        assert!(tree.checkout(&u1));
        let _a2 = tree.append_assistant("alternative reply", vec![]);
        let branch = tree.active_branch();
        assert_eq!(branch.len(), 2);
        assert_eq!(branch[1].text, "alternative reply");
        assert_eq!(tree.all_nodes().len(), 3);
    }
}
