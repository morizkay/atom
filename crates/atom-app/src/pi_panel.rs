//! Built-in Durable Pi Coding Agent panel state, slash commands, and async backend driver.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use pi_agent::{
    AgentBackendKind, AgentEvent, DurableRunStatus, DurableStore, MessageRole, NativePiAgent,
    PiAgentConfig, PiRpcClient, ProviderConfig, SessionSummary, SessionTree,
};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
    Tool {
        id: String,
        name: String,
        args_preview: String,
        output: Option<String>,
        is_error: bool,
    },
    System(String),
    Error(String),
}

pub struct PiPanelState {
    pub config: PiAgentConfig,
    pub store: DurableStore,
    pub durable_status: DurableRunStatus,
    pub session: Arc<Mutex<SessionTree>>,
    pub native_agent: Arc<NativePiAgent>,
    pub rpc_client: Arc<Mutex<Option<Arc<PiRpcClient>>>>,
    pub entries: Vec<ChatEntry>,
    pub input: String,
    pub is_running: bool,
    pub show_tree: bool,
    pub tree_snapshot: Vec<(String, String, bool)>,
    pub show_sessions: bool,
    pub saved_sessions: Vec<SessionSummary>,
    pub event_tx: UnboundedSender<AgentEvent>,
    pub event_rx: UnboundedReceiver<AgentEvent>,
}

impl PiPanelState {
    pub fn new(workspace_root: PathBuf) -> Self {
        let mut config = PiAgentConfig::default_for_workspace(workspace_root.clone());
        let store = DurableStore::new(workspace_root);
        let restored = store.load_or_init(&config.provider);

        config.backend = restored.state.backend;
        config.provider = restored.state.provider.into_runtime_config();
        let durable_status = restored.state.run_status;

        let mut entries = entries_from_session(&restored.tree);
        if entries.is_empty() {
            entries.push(ChatEntry::System(format!(
                "Durable Pi ready ({} · {} / {}).\n\
                 Tools: read, write, edit, bash. State persisted in .pi/\n\
                 Commands: /resume, /sessions, /tree, /new, /model <id>, /provider, /backend, /help",
                config.backend.label(),
                config.provider.provider.name(),
                config.provider.model
            )));
        } else if restored.recovered_interrupted {
            entries.push(ChatEntry::System(
                "Recovered interrupted durable Pi run from .pi/state.json. Click Resume or run /resume to continue."
                    .to_string(),
            ));
        } else {
            entries.push(ChatEntry::System(format!(
                "Restored durable Pi session ({} turns).",
                restored.tree.active_branch().len()
            )));
        }

        let saved_sessions = store.list_sessions();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            config,
            store,
            durable_status,
            session: Arc::new(Mutex::new(restored.tree)),
            native_agent: Arc::new(NativePiAgent::new()),
            rpc_client: Arc::new(Mutex::new(None)),
            entries,
            input: String::new(),
            is_running: false,
            show_tree: false,
            tree_snapshot: Vec::new(),
            show_sessions: false,
            saved_sessions,
            event_tx,
            event_rx,
        }
    }

    pub fn status_label(&self) -> String {
        if self.is_running {
            format!("Pi: running ({})", self.config.provider.model)
        } else if self.durable_status.is_resumable() {
            "Pi: interrupted (resumable)".to_string()
        } else {
            format!(
                "Pi: {} · {}",
                self.config.backend.label(),
                self.durable_status.short_label()
            )
        }
    }

    pub fn toggle_backend(&mut self) {
        self.config.backend = self.config.backend.toggle();
        self.entries.push(ChatEntry::System(format!(
            "Switched Pi backend to {}",
            self.config.backend.label()
        )));
    }

    pub fn cycle_provider(&mut self) {
        let next = self.config.provider.provider.cycle();
        self.config.provider = ProviderConfig::for_provider(next);
        self.entries.push(ChatEntry::System(format!(
            "Switched provider to {} ({})",
            self.config.provider.provider.name(),
            self.config.provider.model
        )));
    }

    pub fn new_session(&mut self, runtime: &Runtime) {
        let session = Arc::clone(&self.session);
        let store = self.store.clone();
        let backend = self.config.backend;
        let provider = self.config.provider.clone();

        runtime.block_on(async {
            let mut guard = session.lock().await;
            *guard = SessionTree::new();
            let _ = store.checkpoint(&guard, DurableRunStatus::Idle, backend, &provider);
        });
        self.durable_status = DurableRunStatus::Idle;
        self.entries.clear();
        self.tree_snapshot.clear();
        self.saved_sessions = self.store.list_sessions();
        self.entries.push(ChatEntry::System(
            "Started new durable Pi session tree.".to_string(),
        ));
    }

    pub fn refresh_saved_sessions(&mut self) {
        self.saved_sessions = self.store.list_sessions();
    }

    pub fn load_saved_session(&mut self, session_id: &str, runtime: &Runtime) {
        match self.store.load_session(session_id) {
            Ok(tree) => {
                let rebuilt = entries_from_session(&tree);
                let session = Arc::clone(&self.session);
                let store = self.store.clone();
                let backend = self.config.backend;
                let provider = self.config.provider.clone();
                runtime.block_on(async {
                    let mut guard = session.lock().await;
                    *guard = tree;
                    let _ = store.checkpoint(&guard, DurableRunStatus::Idle, backend, &provider);
                });
                self.durable_status = DurableRunStatus::Idle;
                self.entries = rebuilt;
                self.entries.push(ChatEntry::System(format!(
                    "Loaded durable session {}",
                    &session_id[..session_id.len().min(8)]
                )));
                self.show_sessions = false;
                if self.show_tree {
                    self.refresh_tree_snapshot(runtime);
                }
            }
            Err(err) => {
                self.entries
                    .push(ChatEntry::Error(format!("Failed to load session: {err}")));
            }
        }
    }

    pub fn refresh_tree_snapshot(&mut self, runtime: &Runtime) {
        let session = Arc::clone(&self.session);
        let snapshot = runtime.block_on(async {
            let guard = session.lock().await;
            let active = guard.active_leaf_id().map(ToString::to_string);
            guard
                .all_nodes()
                .iter()
                .map(|n| {
                    let is_active = active.as_deref() == Some(n.id.as_str());
                    (
                        n.id.clone(),
                        format!("{:?}: {}", n.role, n.preview()),
                        is_active,
                    )
                })
                .collect::<Vec<_>>()
        });
        self.tree_snapshot = snapshot;
    }

    pub fn checkout_node(&mut self, node_id: &str, runtime: &Runtime) {
        let session = Arc::clone(&self.session);
        let store = self.store.clone();
        let backend = self.config.backend;
        let provider = self.config.provider.clone();
        let status = self.durable_status.clone();

        let rebuilt = runtime.block_on(async {
            let mut guard = session.lock().await;
            if guard.checkout(node_id) {
                let _ = store.checkpoint(&guard, status, backend, &provider);
                Some(entries_from_session(&guard))
            } else {
                None
            }
        });
        if let Some(rebuilt) = rebuilt {
            self.entries = rebuilt;
            self.entries.push(ChatEntry::System(format!(
                "Branched session tree to node {}",
                &node_id[..node_id.len().min(8)]
            )));
            self.refresh_tree_snapshot(runtime);
        }
    }

    pub fn resume(&mut self, active_file: Option<PathBuf>, runtime: &Runtime) {
        if self.is_running {
            return;
        }
        if self.config.backend != AgentBackendKind::NativeRust {
            self.entries.push(ChatEntry::System(
                "Durable /resume is handled by the Native Rust backend.".into(),
            ));
            return;
        }
        self.is_running = true;
        let agent = Arc::clone(&self.native_agent);
        let config = self.config.clone();
        let session = Arc::clone(&self.session);
        let tx = self.event_tx.clone();
        runtime.spawn(async move {
            agent.resume_session(config, session, active_file, tx).await;
        });
    }

    pub fn abort(&self, runtime: &Runtime) {
        self.native_agent
            .cancel_handle()
            .store(true, Ordering::SeqCst);
        let rpc_client = Arc::clone(&self.rpc_client);
        runtime.spawn(async move {
            if let Some(client) = rpc_client.lock().await.as_ref() {
                let _ = client.abort().await;
            }
        });
    }

    /// Submits the current input box (handling slash commands, steering, or a new prompt).
    pub fn submit(&mut self, active_file: Option<PathBuf>, runtime: &Runtime) {
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.input.clear();

        if prompt.starts_with('/') {
            self.handle_slash_command(&prompt, active_file, runtime);
            return;
        }

        // Mid-run steering (Pi's steer behavior)
        if self.is_running {
            self.entries
                .push(ChatEntry::User(format!("[Steer] {prompt}")));
            match self.config.backend {
                AgentBackendKind::NativeRust => {
                    let steer = self.native_agent.steer_handle();
                    runtime.spawn(async move {
                        steer.lock().await.push(prompt);
                    });
                }
                AgentBackendKind::ExternalPiRpc => {
                    let rpc_client = Arc::clone(&self.rpc_client);
                    runtime.spawn(async move {
                        if let Some(client) = rpc_client.lock().await.as_ref() {
                            let _ = client.steer(&prompt).await;
                        }
                    });
                }
            }
            return;
        }

        self.entries.push(ChatEntry::User(prompt.clone()));
        self.is_running = true;

        match self.config.backend {
            AgentBackendKind::NativeRust => {
                let agent = Arc::clone(&self.native_agent);
                let config = self.config.clone();
                let session = Arc::clone(&self.session);
                let tx = self.event_tx.clone();
                runtime.spawn(async move {
                    agent.run_prompt(config, session, prompt, active_file, tx).await;
                });
            }
            AgentBackendKind::ExternalPiRpc => {
                let rpc_slot = Arc::clone(&self.rpc_client);
                let binary = self.config.pi_cli_binary.clone();
                let root = self.config.workspace_root.clone();
                let tx = self.event_tx.clone();
                runtime.spawn(async move {
                    let mut guard = rpc_slot.lock().await;
                    if guard.is_none() {
                        match PiRpcClient::spawn(&binary, &root, tx.clone()).await {
                            Ok(client) => *guard = Some(Arc::new(client)),
                            Err(err) => {
                                let _ = tx.send(AgentEvent::Error(err.to_string()));
                                let _ = tx.send(AgentEvent::AgentEnd);
                                return;
                            }
                        }
                    }
                    if let Some(client) = guard.as_ref()
                        && let Err(err) = client.prompt(&prompt).await
                    {
                        let _ = tx.send(AgentEvent::Error(err.to_string()));
                        let _ = tx.send(AgentEvent::AgentEnd);
                    }
                });
            }
        }
    }

    fn handle_slash_command(
        &mut self,
        cmd: &str,
        active_file: Option<PathBuf>,
        runtime: &Runtime,
    ) {
        let mut parts = cmd.splitn(2, ' ');
        let name = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");

        match name {
            "/new" => self.new_session(runtime),
            "/resume" => self.resume(active_file, runtime),
            "/sessions" => {
                self.show_sessions = !self.show_sessions;
                if self.show_sessions {
                    self.show_tree = false;
                    self.refresh_saved_sessions();
                }
            }
            "/clear" => self.entries.clear(),
            "/tree" => {
                self.show_tree = !self.show_tree;
                if self.show_tree {
                    self.show_sessions = false;
                    self.refresh_tree_snapshot(runtime);
                }
            }
            "/backend" => self.toggle_backend(),
            "/provider" => self.cycle_provider(),
            "/model" => {
                if arg.is_empty() {
                    self.entries.push(ChatEntry::System(format!(
                        "Current model: {} ({}). Usage: /model <model-id>",
                        self.config.provider.model,
                        self.config.provider.provider.name()
                    )));
                } else {
                    self.config.provider.model = arg.to_string();
                    self.entries.push(ChatEntry::System(format!(
                        "Model set to {}",
                        self.config.provider.model
                    )));
                }
            }
            "/help" => {
                self.entries.push(ChatEntry::System(
                    "Durable Pi Slash Commands:\n\
                     /resume         Resume interrupted or failed durable run\n\
                     /sessions       Browse & load persisted .pi/sessions/ history\n\
                     /tree           Toggle session tree drawer (branch from any turn)\n\
                     /new            Start a new durable session tree\n\
                     /model <id>     Set active model ID\n\
                     /provider       Cycle provider (Anthropic / OpenAI / OpenRouter / Ollama)\n\
                     /backend        Toggle Native Rust durable core vs `pi --mode rpc`\n\
                     /clear          Clear chat view"
                        .to_string(),
                ));
            }
            other => {
                self.entries.push(ChatEntry::Error(format!(
                    "Unknown command `{other}`. Type /help for available commands."
                )));
            }
        }
    }

    /// Drains all pending `AgentEvent`s from the channel, returning `(had_events, modified_files)`.
    pub fn drain_events(&mut self, runtime: &Runtime) -> (bool, Vec<PathBuf>) {
        let mut had_events = false;
        let mut modified_files = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            had_events = true;
            match event {
                AgentEvent::AgentStart => {
                    self.is_running = true;
                }
                AgentEvent::TextDelta(delta) => {
                    if let Some(ChatEntry::Assistant(existing)) = self.entries.last_mut() {
                        existing.push_str(&delta);
                    } else {
                        self.entries.push(ChatEntry::Assistant(delta));
                    }
                }
                AgentEvent::ThinkingDelta(_) => {}
                AgentEvent::ToolStart {
                    id,
                    name,
                    arguments,
                } => {
                    let args_preview = arguments.to_string();
                    let args_preview = if args_preview.len() > 100 {
                        format!("{}…", &args_preview[..100])
                    } else {
                        args_preview
                    };
                    self.entries.push(ChatEntry::Tool {
                        id,
                        name,
                        args_preview,
                        output: None,
                        is_error: false,
                    });
                }
                AgentEvent::ToolEnd {
                    id,
                    name,
                    output,
                    is_error,
                } => {
                    let preview = if output.len() > 400 {
                        format!("{}…", &output[..400])
                    } else {
                        output
                    };
                    if let Some(ChatEntry::Tool {
                        output: slot,
                        is_error: err_slot,
                        ..
                    }) = self
                        .entries
                        .iter_mut()
                        .rev()
                        .find(|e| matches!(e, ChatEntry::Tool { id: tid, .. } if tid == &id))
                    {
                        *slot = Some(preview);
                        *err_slot = is_error;
                    } else {
                        self.entries.push(ChatEntry::Tool {
                            id,
                            name,
                            args_preview: String::new(),
                            output: Some(preview),
                            is_error,
                        });
                    }
                }
                AgentEvent::FileModified(path) => {
                    modified_files.push(path);
                }
                AgentEvent::DurableStatus(status) => {
                    self.durable_status = status;
                }
                AgentEvent::Status(msg) => {
                    self.entries.push(ChatEntry::System(msg));
                }
                AgentEvent::Error(err) => {
                    self.entries.push(ChatEntry::Error(err));
                }
                AgentEvent::AgentEnd => {
                    self.is_running = false;
                    self.refresh_saved_sessions();
                    if self.show_tree {
                        self.refresh_tree_snapshot(runtime);
                    }
                }
            }
        }
        (had_events, modified_files)
    }
}

fn entries_from_session(tree: &SessionTree) -> Vec<ChatEntry> {
    let mut out = Vec::new();
    for node in tree.active_branch() {
        match node.role {
            MessageRole::User => out.push(ChatEntry::User(node.text.clone())),
            MessageRole::Assistant => {
                if !node.text.is_empty() {
                    out.push(ChatEntry::Assistant(node.text.clone()));
                }
                for tc in &node.tool_calls {
                    let args_preview = tc.arguments.to_string();
                    let args_preview = if args_preview.len() > 100 {
                        format!("{}…", &args_preview[..100])
                    } else {
                        args_preview
                    };
                    out.push(ChatEntry::Tool {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        args_preview,
                        output: None,
                        is_error: false,
                    });
                }
            }
            MessageRole::ToolResult => {
                if let Some(tr) = &node.tool_result {
                    let preview = if tr.output.len() > 400 {
                        format!("{}…", &tr.output[..400])
                    } else {
                        tr.output.clone()
                    };
                    if let Some(ChatEntry::Tool {
                        output: slot,
                        is_error: err_slot,
                        ..
                    }) = out
                        .iter_mut()
                        .rev()
                        .find(|e| matches!(e, ChatEntry::Tool { id, .. } if id == &tr.tool_call_id))
                    {
                        *slot = Some(preview);
                        *err_slot = tr.is_error;
                    }
                }
            }
        }
    }
    out
}
