//! Durable Native Rust Pi Coding Agent runtime loop.
//!
//! Checkpoints every step atomically to `<workspace_root>/.pi/` via `DurableStore`,
//! retries transient LLM stream errors with exponential backoff, and supports
//! resuming interrupted or failed runs from the exact persisted session tree.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::Client;
use tokio::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use crate::durable::{DurableRunStatus, DurableStore};
use crate::provider::stream_completion;
use crate::session::{SessionTree, ToolCallRecord, ToolResultRecord};
use crate::tools::execute_tool;
use crate::{AgentEvent, PiAgentConfig};

pub struct NativePiAgent {
    client: Client,
    cancel_flag: Arc<AtomicBool>,
    steer_queue: Arc<Mutex<Vec<String>>>,
}

impl NativePiAgent {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            steer_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    pub fn steer_handle(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.steer_queue)
    }

    /// Appends a new user prompt and runs the durable agent loop.
    pub async fn run_prompt(
        &self,
        config: PiAgentConfig,
        session: Arc<Mutex<SessionTree>>,
        prompt: String,
        active_file: Option<PathBuf>,
        events: UnboundedSender<AgentEvent>,
    ) {
        {
            let mut guard = session.lock().await;
            guard.append_user(prompt);
        }
        self.run_loop(config, session, active_file, events).await;
    }

    /// Resumes an interrupted or failed durable run directly from the persisted `SessionTree`.
    pub async fn resume_session(
        &self,
        config: PiAgentConfig,
        session: Arc<Mutex<SessionTree>>,
        active_file: Option<PathBuf>,
        events: UnboundedSender<AgentEvent>,
    ) {
        let _ = events.send(AgentEvent::Status(
            "Resuming durable Pi session from last checkpoint…".into(),
        ));
        self.run_loop(config, session, active_file, events).await;
    }

    async fn run_loop(
        &self,
        config: PiAgentConfig,
        session: Arc<Mutex<SessionTree>>,
        active_file: Option<PathBuf>,
        events: UnboundedSender<AgentEvent>,
    ) {
        self.cancel_flag.store(false, Ordering::SeqCst);
        let store = DurableStore::new(config.workspace_root.clone());
        let _ = events.send(AgentEvent::AgentStart);

        let system_prompt = build_pi_system_prompt(&config.workspace_root, active_file.as_deref());
        let mut final_status = DurableRunStatus::Completed { turns: 0 };

        for turn in 0..config.max_turns {
            if self.cancel_flag.load(Ordering::SeqCst) {
                let status = DurableRunStatus::Interrupted {
                    turn,
                    reason: "Aborted by user".into(),
                };
                final_status = status.clone();
                let _ = events.send(AgentEvent::Status("Aborted by user (checkpoint saved)".into()));
                break;
            }

            // Inject any queued steering messages before the next LLM call
            {
                let mut q = self.steer_queue.lock().await;
                if !q.is_empty() {
                    let mut guard = session.lock().await;
                    for msg in q.drain(..) {
                        guard.append_user(format!("[Steer] {msg}"));
                    }
                }
            }

            // Checkpoint before calling the model
            let running_status = DurableRunStatus::Running {
                turn: turn + 1,
                step: "model".into(),
            };
            {
                let guard = session.lock().await;
                let _ = store.checkpoint(
                    &guard,
                    running_status.clone(),
                    config.backend,
                    &config.provider,
                );
            }
            let _ = events.send(AgentEvent::DurableStatus(running_status));

            // Durable retry loop for transient provider errors (up to 3 attempts)
            let mut turn_resp = None;
            let mut last_err = None;
            for attempt in 0..3 {
                if self.cancel_flag.load(Ordering::SeqCst) {
                    break;
                }
                let res = {
                    let guard = session.lock().await;
                    let branch = guard.active_branch();
                    stream_completion(
                        &self.client,
                        &config.provider,
                        &system_prompt,
                        &branch,
                        &events,
                    )
                    .await
                };
                match res {
                    Ok(r) => {
                        turn_resp = Some(r);
                        break;
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let retryable = msg.contains("500")
                            || msg.contains("502")
                            || msg.contains("503")
                            || msg.contains("429")
                            || msg.contains("timed out");
                        last_err = Some(msg);
                        if retryable && attempt < 2 {
                            let delay = Duration::from_secs(1 << attempt);
                            let _ = events.send(AgentEvent::Status(format!(
                                "Transient error, retrying in {}s (attempt {}/3)…",
                                delay.as_secs(),
                                attempt + 2
                            )));
                            tokio::time::sleep(delay).await;
                        } else {
                            break;
                        }
                    }
                }
            }

            let Some(turn_resp) = turn_resp else {
                let err_msg = last_err.unwrap_or_else(|| "Model call aborted".into());
                let _ = events.send(AgentEvent::Error(err_msg.clone()));
                final_status = DurableRunStatus::Failed {
                    turn: turn + 1,
                    error: err_msg,
                };
                break;
            };

            let tool_call_records: Vec<ToolCallRecord> = turn_resp
                .tool_calls
                .iter()
                .map(|tc| ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect();

            {
                let mut guard = session.lock().await;
                guard.append_assistant(turn_resp.text.clone(), tool_call_records);
                let _ = store.checkpoint(
                    &guard,
                    DurableRunStatus::Running {
                        turn: turn + 1,
                        step: "assistant_recorded".into(),
                    },
                    config.backend,
                    &config.provider,
                );
            }

            if turn_resp.tool_calls.is_empty() {
                final_status = DurableRunStatus::Completed { turns: turn + 1 };
                break;
            }

            let mut aborted_in_tools = false;
            for tc in &turn_resp.tool_calls {
                if self.cancel_flag.load(Ordering::SeqCst) {
                    let _ = events.send(AgentEvent::Status(
                        "Aborted during tool execution (checkpoint saved)".into(),
                    ));
                    final_status = DurableRunStatus::Interrupted {
                        turn: turn + 1,
                        reason: format!("Aborted before tool `{}`", tc.name),
                    };
                    aborted_in_tools = true;
                    break;
                }

                let tool_status = DurableRunStatus::Running {
                    turn: turn + 1,
                    step: format!("tool:{}", tc.name),
                };
                {
                    let guard = session.lock().await;
                    let _ = store.checkpoint(
                        &guard,
                        tool_status.clone(),
                        config.backend,
                        &config.provider,
                    );
                }
                let _ = events.send(AgentEvent::DurableStatus(tool_status));

                let _ = events.send(AgentEvent::ToolStart {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });

                let exec_res = execute_tool(&config.workspace_root, tc).await;

                if let Some(modified) = exec_res.modified_file.clone() {
                    let _ = events.send(AgentEvent::FileModified(modified));
                }

                let _ = events.send(AgentEvent::ToolEnd {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: exec_res.output.clone(),
                    is_error: exec_res.is_error,
                });

                let mut guard = session.lock().await;
                guard.append_tool_result(ToolResultRecord {
                    tool_call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: exec_res.output,
                    is_error: exec_res.is_error,
                });
                let _ = store.checkpoint(
                    &guard,
                    DurableRunStatus::Running {
                        turn: turn + 1,
                        step: format!("tool_done:{}", tc.name),
                    },
                    config.backend,
                    &config.provider,
                );
            }

            if aborted_in_tools {
                break;
            }

            if turn + 1 == config.max_turns {
                let _ = events.send(AgentEvent::Status(format!(
                    "Reached max turns ({}); use /resume to continue",
                    config.max_turns
                )));
                final_status = DurableRunStatus::Interrupted {
                    turn: config.max_turns,
                    reason: "Max turns reached".into(),
                };
            }
        }

        // Final durable checkpoint
        {
            let guard = session.lock().await;
            let _ = store.checkpoint(
                &guard,
                final_status.clone(),
                config.backend,
                &config.provider,
            );
        }
        let _ = events.send(AgentEvent::DurableStatus(final_status));
        let _ = events.send(AgentEvent::AgentEnd);
    }
}

impl Default for NativePiAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimal Pi-style system prompt (< 250 tokens).
fn build_pi_system_prompt(
    workspace_root: &std::path::Path,
    active_file: Option<&std::path::Path>,
) -> String {
    let mut prompt = format!(
        "You are Pi, the minimal durable coding agent built into the Atom GPUI editor.\n\
         Workspace root: {}\n\
         Tools available: read, write, edit, bash.\n\
         Guidelines:\n\
         - Read files before editing them.\n\
         - Make minimal, surgical edits with `edit` or create files with `write`.\n\
         - Use `bash` to run tests, builds, or inspect git status.\n\
         - Keep responses concise and direct.",
        workspace_root.display()
    );
    if let Some(file) = active_file {
        prompt.push_str(&format!("\nCurrently open editor file: {}", file.display()));
    }
    prompt
}
