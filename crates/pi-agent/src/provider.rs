//! Multi-provider streaming LLM client for the built-in Pi Coding Agent.
//!
//! Supports:
//! - Anthropic Messages API (`/v1/messages`, SSE streaming + tool_use)
//! - OpenAI Chat Completions (`/v1/chat/completions`, SSE streaming + tool_calls)
//! - OpenRouter (OpenAI-compatible)
//! - Ollama / local vLLM / LM Studio (`http://localhost:11434/v1/chat/completions`)

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::AgentEvent;
use crate::session::{MessageRole, SessionNode};
use crate::tools::{ToolCall, anthropic_tool_definitions, openai_tool_definitions};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmProvider {
    Anthropic,
    OpenAi,
    OpenRouter,
    Ollama,
}

impl LlmProvider {
    pub fn name(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "Anthropic",
            LlmProvider::OpenAi => "OpenAI",
            LlmProvider::OpenRouter => "OpenRouter",
            LlmProvider::Ollama => "Ollama (Local)",
        }
    }

    pub fn cycle(&self) -> Self {
        match self {
            LlmProvider::Anthropic => LlmProvider::OpenAi,
            LlmProvider::OpenAi => LlmProvider::OpenRouter,
            LlmProvider::OpenRouter => LlmProvider::Ollama,
            LlmProvider::Ollama => LlmProvider::Anthropic,
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "claude-sonnet-4-20250514",
            LlmProvider::OpenAi => "gpt-4.1",
            LlmProvider::OpenRouter => "anthropic/claude-sonnet-4",
            LlmProvider::Ollama => "qwen2.5-coder:14b",
        }
    }

    pub fn default_base_url(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "https://api.anthropic.com",
            LlmProvider::OpenAi => "https://api.openai.com/v1",
            LlmProvider::OpenRouter => "https://openrouter.ai/api/v1",
            LlmProvider::Ollama => "http://localhost:11434/v1",
        }
    }

    pub fn env_key_name(&self) -> Option<&'static str> {
        match self {
            LlmProvider::Anthropic => Some("ANTHROPIC_API_KEY"),
            LlmProvider::OpenAi => Some("OPENAI_API_KEY"),
            LlmProvider::OpenRouter => Some("OPENROUTER_API_KEY"),
            LlmProvider::Ollama => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: LlmProvider,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ProviderConfig {
    pub fn for_provider(provider: LlmProvider) -> Self {
        let api_key = provider
            .env_key_name()
            .and_then(|k| std::env::var(k).ok())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| load_pi_auth_key(provider));

        Self {
            provider,
            model: provider.default_model().to_string(),
            base_url: provider.default_base_url().to_string(),
            api_key,
        }
    }

    /// Detects the best configured provider from environment variables or `~/.pi/agent/auth.json`.
    pub fn detect_from_env() -> Self {
        for provider in [
            LlmProvider::Anthropic,
            LlmProvider::OpenAi,
            LlmProvider::OpenRouter,
        ] {
            let cfg = Self::for_provider(provider);
            if cfg.api_key.is_some() {
                return cfg;
            }
        }
        Self::for_provider(LlmProvider::Ollama)
    }

    pub fn has_credentials(&self) -> bool {
        self.provider == LlmProvider::Ollama
            || self
                .api_key
                .as_ref()
                .is_some_and(|k| !k.trim().is_empty())
    }
}

/// Attempts to read credentials from Pi's standard `~/.pi/agent/auth.json`.
fn load_pi_auth_key(provider: LlmProvider) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home).join(".pi/agent/auth.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let key = match provider {
        LlmProvider::Anthropic => "anthropic",
        LlmProvider::OpenAi => "openai",
        LlmProvider::OpenRouter => "openrouter",
        LlmProvider::Ollama => return None,
    };
    json.get(key)
        .and_then(|entry| {
            entry
                .as_str()
                .or_else(|| entry.get("key").and_then(|v| v.as_str()))
                .or_else(|| entry.get("apiKey").and_then(|v| v.as_str()))
        })
        .map(ToString::to_string)
}

#[derive(Debug, Clone)]
pub struct ModelTurnResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Streams a single model turn and emits `AgentEvent::TextDelta` as tokens arrive.
pub async fn stream_completion(
    client: &Client,
    config: &ProviderConfig,
    system_prompt: &str,
    history: &[&SessionNode],
    events: &UnboundedSender<AgentEvent>,
) -> Result<ModelTurnResponse> {
    match config.provider {
        LlmProvider::Anthropic => {
            stream_anthropic(client, config, system_prompt, history, events).await
        }
        LlmProvider::OpenAi | LlmProvider::OpenRouter | LlmProvider::Ollama => {
            stream_openai_compatible(client, config, system_prompt, history, events).await
        }
    }
}

async fn stream_anthropic(
    client: &Client,
    config: &ProviderConfig,
    system_prompt: &str,
    history: &[&SessionNode],
    events: &UnboundedSender<AgentEvent>,
) -> Result<ModelTurnResponse> {
    let api_key = config
        .api_key
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!("ANTHROPIC_API_KEY not set. Export it or configure ~/.pi/agent/auth.json.")
        })?;

    let mut messages = Vec::new();
    for node in history {
        match node.role {
            MessageRole::User => {
                messages.push(json!({
                    "role": "user",
                    "content": node.text,
                }));
            }
            MessageRole::Assistant => {
                let mut blocks = Vec::new();
                if !node.text.is_empty() {
                    blocks.push(json!({ "type": "text", "text": node.text }));
                }
                for tc in &node.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                if !blocks.is_empty() {
                    messages.push(json!({
                        "role": "assistant",
                        "content": blocks,
                    }));
                }
            }
            MessageRole::ToolResult => {
                if let Some(tr) = &node.tool_result {
                    messages.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tr.tool_call_id,
                            "content": tr.output,
                            "is_error": tr.is_error,
                        }]
                    }));
                }
            }
        }
    }

    let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));
    let body = json!({
        "model": config.model,
        "max_tokens": 4096,
        "stream": true,
        "system": system_prompt,
        "tools": anthropic_tool_definitions(),
        "messages": messages,
    });

    let resp = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("failed to reach Anthropic API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Anthropic API error {status}: {text}"));
    }

    let mut text_out = String::new();
    let mut active_tools: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    let mut byte_stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim_end_matches('\r').to_string();
            buf.drain(..=pos);

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data == "[DONE]" {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(data) else {
                continue;
            };

            match val.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "content_block_start" => {
                    let index = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    if let Some(cb) = val.get("content_block")
                        && cb.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                    {
                        let id = cb
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = cb
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        active_tools.insert(index, (id, name, String::new()));
                    }
                }
                "content_block_delta" => {
                    let index = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    if let Some(delta) = val.get("delta") {
                        match delta.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                            "text_delta" => {
                                if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                    text_out.push_str(t);
                                    let _ = events.send(AgentEvent::TextDelta(t.to_string()));
                                }
                            }
                            "input_json_delta" => {
                                if let Some(partial) =
                                    delta.get("partial_json").and_then(|v| v.as_str())
                                    && let Some((_, _, json_buf)) = active_tools.get_mut(&index)
                                {
                                    json_buf.push_str(partial);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let tool_calls = active_tools
        .into_values()
        .map(|(id, name, args_str)| {
            let arguments = serde_json::from_str(&args_str).unwrap_or_else(|_| json!({}));
            ToolCall {
                id,
                name,
                arguments,
            }
        })
        .collect();

    Ok(ModelTurnResponse {
        text: text_out,
        tool_calls,
    })
}

async fn stream_openai_compatible(
    client: &Client,
    config: &ProviderConfig,
    system_prompt: &str,
    history: &[&SessionNode],
    events: &UnboundedSender<AgentEvent>,
) -> Result<ModelTurnResponse> {
    if config.provider != LlmProvider::Ollama && !config.has_credentials() {
        return Err(anyhow!(
            "{} API key not configured. Export {} or switch provider.",
            config.provider.name(),
            config.provider.env_key_name().unwrap_or("API_KEY")
        ));
    }

    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];

    for node in history {
        match node.role {
            MessageRole::User => {
                messages.push(json!({
                    "role": "user",
                    "content": node.text,
                }));
            }
            MessageRole::Assistant => {
                let mut msg = json!({
                    "role": "assistant",
                    "content": if node.text.is_empty() { Value::Null } else { Value::String(node.text.clone()) },
                });
                if !node.tool_calls.is_empty() {
                    let tcs: Vec<Value> = node
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    msg["tool_calls"] = Value::Array(tcs);
                }
                messages.push(msg);
            }
            MessageRole::ToolResult => {
                if let Some(tr) = &node.tool_result {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tr.tool_call_id,
                        "content": tr.output,
                    }));
                }
            }
        }
    }

    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = json!({
        "model": config.model,
        "stream": true,
        "messages": messages,
        "tools": openai_tool_definitions(),
    });

    let mut req = client
        .post(&url)
        .header("content-type", "application/json")
        .json(&body);

    if let Some(key) = config.api_key.as_deref().filter(|s| !s.is_empty()) {
        req = req.header("authorization", format!("Bearer {key}"));
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("failed to reach {} at {url}", config.provider.name()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{} API error {status}: {text}", config.provider.name()));
    }

    let mut text_out = String::new();
    let mut tool_acc: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    let mut byte_stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim_end_matches('\r').to_string();
            buf.drain(..=pos);

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data.trim() == "[DONE]" {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            let Some(delta) = val
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("delta"))
            else {
                continue;
            };

            if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                text_out.push_str(content);
                let _ = events.send(AgentEvent::TextDelta(content.to_string()));
            }

            if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let entry = tool_acc.entry(idx).or_insert_with(|| {
                        (
                            format!("call_{idx}"),
                            String::new(),
                            String::new(),
                        )
                    });
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str())
                        && !id.is_empty()
                    {
                        entry.0 = id.to_string();
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                            entry.1.push_str(name);
                        }
                        if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                            entry.2.push_str(args);
                        }
                    }
                }
            }
        }
    }

    let tool_calls = tool_acc
        .into_values()
        .map(|(id, name, args_str)| {
            let arguments = serde_json::from_str(&args_str).unwrap_or_else(|_| json!({}));
            ToolCall {
                id,
                name,
                arguments,
            }
        })
        .collect();

    Ok(ModelTurnResponse {
        text: text_out,
        tool_calls,
    })
}
