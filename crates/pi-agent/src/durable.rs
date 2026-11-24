//! Durable execution checkpointing & crash-safe session persistence for Pi Coding Agent.
//!
//! Stores per-project state under `<workspace_root>/.pi/`:
//! - `.pi/state.json`: atomic checkpoint of active session, leaf node, durable run status, and model/backend config.
//! - `.pi/sessions/<session_id>.jsonl`: append/atomic JSONL session tree.
//!
//! If Atom exits or crashes mid-run, the next launch detects `DurableRunStatus::Running`,
//! transitions it to `Interrupted`, and allows one-click `/resume` without losing any prior turns or tool outputs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::provider::{LlmProvider, ProviderConfig};
use crate::session::SessionTree;
use crate::AgentBackendKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DurableRunStatus {
    #[default]
    Idle,
    Running {
        turn: usize,
        step: String,
    },
    Interrupted {
        turn: usize,
        reason: String,
    },
    Failed {
        turn: usize,
        error: String,
    },
    Completed {
        turns: usize,
    },
}

impl DurableRunStatus {
    pub fn is_resumable(&self) -> bool {
        matches!(
            self,
            DurableRunStatus::Interrupted { .. } | DurableRunStatus::Failed { .. }
        )
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            DurableRunStatus::Idle => "durable: idle",
            DurableRunStatus::Running { .. } => "durable: running",
            DurableRunStatus::Interrupted { .. } => "durable: interrupted",
            DurableRunStatus::Failed { .. } => "durable: failed",
            DurableRunStatus::Completed { .. } => "durable: synced",
        }
    }
}

/// Persisted provider settings (never writes API keys to disk).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedProviderRef {
    pub provider: LlmProvider,
    pub model: String,
    pub base_url: String,
}

impl From<&ProviderConfig> for SavedProviderRef {
    fn from(cfg: &ProviderConfig) -> Self {
        Self {
            provider: cfg.provider,
            model: cfg.model.clone(),
            base_url: cfg.base_url.clone(),
        }
    }
}

impl SavedProviderRef {
    pub fn into_runtime_config(self) -> ProviderConfig {
        let mut cfg = ProviderConfig::for_provider(self.provider);
        cfg.model = self.model;
        cfg.base_url = self.base_url;
        cfg
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableProjectState {
    pub active_session_id: String,
    pub active_leaf_id: Option<String>,
    pub run_status: DurableRunStatus,
    pub backend: AgentBackendKind,
    pub provider: SavedProviderRef,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub node_count: usize,
    pub preview: String,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct RestoredDurableSession {
    pub state: DurableProjectState,
    pub tree: SessionTree,
    pub recovered_interrupted: bool,
}

/// Manages atomic durable checkpoints under `<workspace_root>/.pi/`.
#[derive(Debug, Clone)]
pub struct DurableStore {
    workspace_root: PathBuf,
}

impl DurableStore {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn pi_dir(&self) -> PathBuf {
        self.workspace_root.join(".pi")
    }

    pub fn state_path(&self) -> PathBuf {
        self.pi_dir().join("state.json")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.pi_dir().join("sessions")
    }

    pub fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{session_id}.jsonl"))
    }

    /// Loads the last durable state & session tree for this project, or initializes a fresh one.
    /// If the previous process died while `Running`, marks it `Interrupted` for safe resumption.
    pub fn load_or_init(&self, default_provider: &ProviderConfig) -> RestoredDurableSession {
        if let Ok(raw) = std::fs::read_to_string(self.state_path())
            && let Ok(mut state) = serde_json::from_str::<DurableProjectState>(&raw)
        {
            let mut recovered_interrupted = false;
            if let DurableRunStatus::Running { turn, step } = &state.run_status {
                state.run_status = DurableRunStatus::Interrupted {
                    turn: *turn,
                    reason: format!("Interrupted during {step}; ready to resume"),
                };
                recovered_interrupted = true;
            }

            let session_file = self.session_path(&state.active_session_id);
            let mut tree = SessionTree::load_jsonl(&session_file).unwrap_or_else(|_| {
                let mut fresh = SessionTree::new();
                fresh.session_id = state.active_session_id.clone();
                fresh
            });
            tree.session_id = state.active_session_id.clone();
            if let Some(leaf) = &state.active_leaf_id {
                tree.checkout(leaf);
            }

            if recovered_interrupted {
                let _ = self.save_state(&state);
            }

            return RestoredDurableSession {
                state,
                tree,
                recovered_interrupted,
            };
        }

        let tree = SessionTree::new();
        let state = DurableProjectState {
            active_session_id: tree.session_id.clone(),
            active_leaf_id: None,
            run_status: DurableRunStatus::Idle,
            backend: AgentBackendKind::NativeRust,
            provider: SavedProviderRef::from(default_provider),
            updated_at: Utc::now(),
        };
        RestoredDurableSession {
            state,
            tree,
            recovered_interrupted: false,
        }
    }

    /// Atomically persists both the session tree JSONL and the durable run checkpoint.
    pub fn checkpoint(
        &self,
        tree: &SessionTree,
        run_status: DurableRunStatus,
        backend: AgentBackendKind,
        provider: &ProviderConfig,
    ) -> Result<DurableProjectState> {
        let session_file = self.session_path(&tree.session_id);
        tree.save_jsonl(&session_file)?;

        let state = DurableProjectState {
            active_session_id: tree.session_id.clone(),
            active_leaf_id: tree.active_leaf_id().map(ToString::to_string),
            run_status,
            backend,
            provider: SavedProviderRef::from(provider),
            updated_at: Utc::now(),
        };
        self.save_state(&state)?;
        Ok(state)
    }

    /// Writes `.pi/state.json` atomically via a temporary file + rename.
    pub fn save_state(&self, state: &DurableProjectState) -> Result<()> {
        std::fs::create_dir_all(self.pi_dir())?;
        let target = self.state_path();
        let tmp = self.pi_dir().join("state.json.tmp");
        let payload = serde_json::to_string_pretty(state)?;
        std::fs::write(&tmp, payload)
            .with_context(|| format!("failed to write durable checkpoint {}", tmp.display()))?;
        std::fs::rename(&tmp, &target)
            .with_context(|| format!("failed to commit durable checkpoint {}", target.display()))?;
        Ok(())
    }

    /// Loads any saved session by ID.
    pub fn load_session(&self, session_id: &str) -> Result<SessionTree> {
        let path = self.session_path(session_id);
        let mut tree = SessionTree::load_jsonl(&path)?;
        tree.session_id = session_id.to_string();
        Ok(tree)
    }

    /// Lists all persisted sessions under `.pi/sessions/` sorted newest-first.
    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        let Ok(entries) = std::fs::read_dir(self.sessions_dir()) else {
            return Vec::new();
        };

        let mut list = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Ok(tree) = SessionTree::load_jsonl(&path) {
                let nodes = tree.all_nodes();
                let preview = nodes
                    .iter()
                    .find(|n| !n.text.is_empty())
                    .map(|n| n.preview())
                    .unwrap_or_else(|| "(empty session)".to_string());
                let updated_at = nodes.last().map(|n| n.timestamp);
                list.push(SessionSummary {
                    session_id: stem.to_string(),
                    node_count: nodes.len(),
                    preview,
                    updated_at,
                });
            }
        }

        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        list
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_checkpoint_and_crash_recovery() {
        let dir = std::env::temp_dir().join(format!("pi_durable_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let store = DurableStore::new(dir.clone());
        let provider = ProviderConfig::for_provider(LlmProvider::Ollama);
        let mut tree = SessionTree::new();
        tree.append_user("refactor editor");
        tree.append_assistant("working on it", vec![]);

        // Simulate crash while Running
        store
            .checkpoint(
                &tree,
                DurableRunStatus::Running {
                    turn: 1,
                    step: "tool:bash".into(),
                },
                AgentBackendKind::NativeRust,
                &provider,
            )
            .unwrap();

        // Reload as if process restarted
        let restored = store.load_or_init(&provider);
        assert!(restored.recovered_interrupted);
        assert!(restored.state.run_status.is_resumable());
        assert_eq!(restored.tree.active_branch().len(), 2);
        assert_eq!(store.list_sessions().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
