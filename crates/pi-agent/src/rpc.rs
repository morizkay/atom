//! Optional `pi --mode rpc` subprocess bridge.
//!
//! Speaks Pi's strict LF-only JSONL protocol over stdin/stdout so users with
//! `@earendil-works/pi-coding-agent` installed can use their full Pi extensions,
//! skills, MCP adapters, and model catalog directly inside Atom GPUI.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

use crate::AgentEvent;

pub struct PiRpcClient {
    stdin: Mutex<ChildStdin>,
    _child: Mutex<Child>,
    next_id: AtomicU64,
}

impl PiRpcClient {
    /// Spawns `pi --mode rpc --no-session` in `workspace_root` and streams stdout events
    /// into `events`.
    pub async fn spawn(
        binary: &str,
        workspace_root: &Path,
        events: UnboundedSender<AgentEvent>,
    ) -> Result<Self> {
        let mut child = Command::new(binary)
            .args(["--mode", "rpc", "--no-session"])
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to spawn `{binary} --mode rpc`. Install with `npm install -g --ignore-scripts @earendil-works/pi-coding-agent` or switch to Native Rust backend."
                )
            })?;

        let stdin = child.stdin.take().context("missing stdin on pi process")?;
        let stdout = child.stdout.take().context("missing stdout on pi process")?;

        // Strict LF-only framing per Pi RPC docs (U+2028/U+2029 are valid inside JSON strings)
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line_buf = Vec::new();
            loop {
                line_buf.clear();
                match reader.read_until(b'\n', &mut line_buf).await {
                    Ok(0) => {
                        let _ = events.send(AgentEvent::Status("Pi RPC process exited".into()));
                        let _ = events.send(AgentEvent::AgentEnd);
                        break;
                    }
                    Ok(_) => {
                        while line_buf.ends_with(b"\n") || line_buf.ends_with(b"\r") {
                            line_buf.pop();
                        }
                        if line_buf.is_empty() {
                            continue;
                        }
                        if let Ok(text) = std::str::from_utf8(&line_buf)
                            && let Ok(val) = serde_json::from_str::<Value>(text)
                        {
                            dispatch_rpc_record(&val, &events);
                        }
                    }
                    Err(err) => {
                        let _ = events.send(AgentEvent::Error(format!("Pi RPC read error: {err}")));
                        let _ = events.send(AgentEvent::AgentEnd);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            stdin: Mutex::new(stdin),
            _child: Mutex::new(child),
            next_id: AtomicU64::new(1),
        })
    }

    async fn send_command(&self, mut cmd: Value) -> Result<()> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        if let Some(obj) = cmd.as_object_mut() {
            obj.insert("id".into(), Value::String(format!("atom-{id}")));
        }
        let mut line = serde_json::to_string(&cmd)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn prompt(&self, message: &str) -> Result<()> {
        self.send_command(json!({
            "type": "prompt",
            "message": message,
        }))
        .await
    }

    pub async fn steer(&self, message: &str) -> Result<()> {
        self.send_command(json!({
            "type": "prompt",
            "message": message,
            "streamingBehavior": "steer",
        }))
        .await
    }

    pub async fn abort(&self) -> Result<()> {
        self.send_command(json!({ "type": "abort" })).await
    }
}

fn dispatch_rpc_record(val: &Value, events: &UnboundedSender<AgentEvent>) {
    let record_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match record_type {
        "agent_start" => {
            let _ = events.send(AgentEvent::AgentStart);
        }
        "agent_end" => {
            let _ = events.send(AgentEvent::AgentEnd);
        }
        "message_update" => {
            if let Some(delta) = val
                .get("assistantMessageEvent")
                .and_then(|e| e.get("delta"))
                .and_then(|v| v.as_str())
            {
                let _ = events.send(AgentEvent::TextDelta(delta.to_string()));
            } else if let Some(delta) = val.get("delta").and_then(|v| v.as_str()) {
                let _ = events.send(AgentEvent::TextDelta(delta.to_string()));
            }
        }
        "tool_execution_start" => {
            let id = val
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = val
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let arguments = val.get("args").cloned().unwrap_or_else(|| json!({}));
            let _ = events.send(AgentEvent::ToolStart {
                id,
                name,
                arguments,
            });
        }
        "tool_execution_end" => {
            let id = val
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = val
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let output = val
                .get("result")
                .map(|r| r.to_string())
                .unwrap_or_default();
            let is_error = val
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let _ = events.send(AgentEvent::ToolEnd {
                id,
                name,
                output,
                is_error,
            });
        }
        "response" => {
            if val.get("success").and_then(|v| v.as_bool()) == Some(false) {
                let err = val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Pi RPC command failed");
                let _ = events.send(AgentEvent::Error(err.to_string()));
            }
        }
        _ => {}
    }
}
