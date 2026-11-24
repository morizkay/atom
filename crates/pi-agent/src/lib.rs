//! Built-in Pi Coding Agent runtime for Atom GPUI.
//!
//! Implements Pi's minimal, extensible harness philosophy:
//! - 4 core tools: `read`, `write`, `edit`, `bash`
//! - Under-1k-token system prompt
//! - Tree-structured JSONL sessions (branch & navigate with `/tree`)
//! - Multi-provider streaming LLM client (Anthropic, OpenAI, OpenRouter, Ollama)
//! - Optional `pi --mode rpc` JSONL subprocess bridge for full compatibility
//!   with `@earendil-works/pi-coding-agent` extensions & skills.

pub mod durable;
pub mod native;
pub mod provider;
pub mod rpc;
pub mod session;
pub mod tools;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use durable::{
    DurableProjectState, DurableRunStatus, DurableStore, RestoredDurableSession, SessionSummary,
};
pub use native::NativePiAgent;
pub use provider::{LlmProvider, ProviderConfig};
pub use rpc::PiRpcClient;
pub use session::{MessageRole, SessionNode, SessionTree, ToolCallRecord, ToolResultRecord};
pub use tools::{ToolCall, ToolExecutionResult, execute_tool};

/// Selects whether the built-in Pi panel runs the native Rust agent loop
/// or bridges to an external `pi --mode rpc` CLI process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AgentBackendKind {
    #[default]
    NativeRust,
    ExternalPiRpc,
}

impl AgentBackendKind {
    pub fn label(&self) -> &'static str {
        match self {
            AgentBackendKind::NativeRust => "Native Rust",
            AgentBackendKind::ExternalPiRpc => "Pi CLI (RPC)",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            AgentBackendKind::NativeRust => AgentBackendKind::ExternalPiRpc,
            AgentBackendKind::ExternalPiRpc => AgentBackendKind::NativeRust,
        }
    }
}

/// Configuration for the built-in Pi Coding Agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiAgentConfig {
    pub workspace_root: PathBuf,
    pub backend: AgentBackendKind,
    pub provider: ProviderConfig,
    pub max_turns: usize,
    pub pi_cli_binary: String,
}

impl PiAgentConfig {
    pub fn default_for_workspace(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            backend: AgentBackendKind::NativeRust,
            provider: ProviderConfig::detect_from_env(),
            max_turns: 16,
            pi_cli_binary: "pi".to_string(),
        }
    }
}

/// Unified streaming event emitted by both Native Rust and External Pi RPC backends.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    AgentStart,
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolEnd {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    FileModified(PathBuf),
    DurableStatus(DurableRunStatus),
    Status(String),
    Error(String),
    AgentEnd,
}
