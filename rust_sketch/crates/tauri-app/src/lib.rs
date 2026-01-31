//! Prolog Router Tauri Application Library
//!
//! This module provides the Tauri commands and state management for the desktop app.

use chrono::Utc;
use prolog_router_core::{agent, tools};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

// ============================================================================
// App State
// ============================================================================

/// Persistent conversation storage
#[derive(Debug, Default)]
pub struct ConversationStore {
    conversations: HashMap<String, StoredConversation>,
}

impl ConversationStore {
    /// Load conversations from disk
    pub fn load(data_dir: &PathBuf) -> Self {
        let path = data_dir.join("conversations.json");
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(convos) = serde_json::from_str(&data) {
                    return Self { conversations: convos };
                }
            }
        }
        Self::default()
    }

    /// Save conversations to disk
    pub fn save(&self, data_dir: &PathBuf) -> Result<(), String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let path = data_dir.join("conversations.json");
        let data = serde_json::to_string_pretty(&self.conversations)
            .map_err(|e| e.to_string())?;
        fs::write(&path, data).map_err(|e| e.to_string())
    }

    /// Add or update a conversation
    pub fn upsert(&mut self, id: String, conv: StoredConversation) {
        self.conversations.insert(id, conv);
    }

    /// Get a conversation by ID
    pub fn get(&self, id: &str) -> Option<&StoredConversation> {
        self.conversations.get(id)
    }

    /// Delete a conversation
    pub fn delete(&mut self, id: &str) -> bool {
        self.conversations.remove(id).is_some()
    }

    /// List all conversations, sorted by last updated (newest first)
    pub fn list(&self) -> Vec<ConversationSummary> {
        let mut summaries: Vec<_> = self.conversations
            .iter()
            .map(|(id, conv)| ConversationSummary {
                id: id.clone(),
                title: conv.title.clone(),
                last_message: conv.messages.last().map(|m| m.content.chars().take(100).collect()),
                updated_at: conv.updated_at.clone(),
                message_count: conv.messages.len(),
            })
            .collect();
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        summaries
    }
}

/// A stored conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConversation {
    pub title: String,
    pub messages: Vec<StoredMessage>,
    pub created_at: String,
    pub updated_at: String,
}

/// A message in a stored conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub role: String,  // "user", "assistant", "tool"
    pub content: String,
    pub tool_result: Option<StoredToolResult>,
    pub timestamp: String,
}

/// Tool result in stored format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToolResult {
    pub tool: String,
    pub success: bool,
    pub output: String,
    pub args: Option<serde_json::Value>,
}

/// Summary for conversation list
#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub last_message: Option<String>,
    pub updated_at: String,
    pub message_count: usize,
}

// ============================================================================
// Agent Events
// ============================================================================

/// Event payload sent to the frontend during agent execution
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEventPayload {
    TurnStarted { turn: u32, max_turns: u32 },
    ToolCalling { tool: String, args: serde_json::Value },
    ToolResult { tool: String, success: bool, output: String },
    FinalAnswer { answer: String },
    Error { message: String },
}

impl From<agent::AgentEvent> for AgentEventPayload {
    fn from(event: agent::AgentEvent) -> Self {
        match event {
            agent::AgentEvent::TurnStarted { turn, max_turns } => {
                AgentEventPayload::TurnStarted { turn, max_turns }
            }
            agent::AgentEvent::ToolCalling { tool, args } => {
                AgentEventPayload::ToolCalling { tool, args }
            }
            agent::AgentEvent::ToolResult { tool, success, output } => {
                AgentEventPayload::ToolResult { tool, success, output }
            }
            agent::AgentEvent::FinalAnswer { answer } => {
                AgentEventPayload::FinalAnswer { answer }
            }
            agent::AgentEvent::Error { message } => {
                AgentEventPayload::Error { message }
            }
        }
    }
}

// ============================================================================
// Tauri Commands - Conversations
// ============================================================================

/// Send a message to the agent with streaming events
#[tauri::command]
async fn send_message(
    app: AppHandle,
    conversation_id: Option<String>,
    message: String,
) -> Result<SendMessageResult, String> {
    let (tx, rx) = mpsc::channel::<agent::AgentEvent>();
    let app_for_events = app.clone();

    // Generate conversation ID if not provided
    let conv_id = conversation_id.unwrap_or_else(|| {
        format!("conv_{}", chrono::Utc::now().timestamp_millis())
    });
    let conv_id_for_save = conv_id.clone();

    // Collect messages for storage
    let message_clone = message.clone();
    let collected_messages = std::sync::Arc::new(Mutex::new(Vec::<StoredMessage>::new()));
    let collected_for_callback = collected_messages.clone();

    // Spawn event forwarding task
    let event_handle = tauri::async_runtime::spawn(async move {
        while let Ok(event) = rx.recv() {
            let payload: AgentEventPayload = event.into();
            let event_name = match &payload {
                AgentEventPayload::TurnStarted { .. } => "agent:turn_started",
                AgentEventPayload::ToolCalling { .. } => "agent:tool_calling",
                AgentEventPayload::ToolResult { .. } => "agent:tool_result",
                AgentEventPayload::FinalAnswer { .. } => "agent:final_answer",
                AgentEventPayload::Error { .. } => "agent:error",
            };

            // Collect tool results and final answer for storage
            if let Ok(mut msgs) = collected_for_callback.lock() {
                match &payload {
                    AgentEventPayload::ToolResult { tool, success, output } => {
                        msgs.push(StoredMessage {
                            role: "tool".to_string(),
                            content: format!("Tool: {}", tool),
                            tool_result: Some(StoredToolResult {
                                tool: tool.clone(),
                                success: *success,
                                output: output.clone(),
                                args: None,
                            }),
                            timestamp: Utc::now().to_rfc3339(),
                        });
                    }
                    AgentEventPayload::FinalAnswer { answer } => {
                        msgs.push(StoredMessage {
                            role: "assistant".to_string(),
                            content: answer.clone(),
                            tool_result: None,
                            timestamp: Utc::now().to_rfc3339(),
                        });
                    }
                    _ => {}
                }
            }

            if let Err(e) = app_for_events.emit(event_name, &payload) {
                eprintln!("Failed to emit event: {}", e);
            }
        }
    });

    // Run blocking agent code
    let result = tauri::async_runtime::spawn_blocking(move || {
        dotenvy::dotenv().ok();

        let tools_paths = [
            std::path::PathBuf::from("tools.json"),
            std::path::PathBuf::from("../../tools.json"),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools.json"),
        ];

        let tool_executor = tools_paths
            .iter()
            .find(|p| p.exists())
            .and_then(|p| {
                eprintln!("DEBUG: Loading tools from {:?}", p);
                tools::ToolExecutor::load(p).ok()
            });

        if tool_executor.is_none() {
            eprintln!("WARNING: Could not load tools.json from any path: {:?}", tools_paths);
        }

        let config = agent::AgentConfig {
            max_turns: 10,
            verbose: true,
            use_memory: true,
        };

        agent::run_agent_loop_with_events(
            &message,
            &config,
            tool_executor.as_ref(),
            move |event| {
                let _ = tx.send(event);
            },
        )
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;

    let _ = event_handle.await;

    // Save conversation to store
    if let Ok(_answer) = &result {
        if let Ok(data_dir) = app.path().app_data_dir() {
            let state = app.state::<Mutex<ConversationStore>>();
            let store_guard = state.lock();
            if let Ok(mut store) = store_guard {
                // Get existing or create new conversation
                let now = Utc::now().to_rfc3339();
                let mut conv = store.get(&conv_id_for_save).cloned().unwrap_or_else(|| {
                    StoredConversation {
                        title: message_clone.chars().take(50).collect(),
                        messages: Vec::new(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    }
                });

                // Add user message
                conv.messages.push(StoredMessage {
                    role: "user".to_string(),
                    content: message_clone,
                    tool_result: None,
                    timestamp: now.clone(),
                });

                // Add collected messages (tool results and final answer)
                if let Ok(msgs) = collected_messages.lock() {
                    conv.messages.extend(msgs.clone());
                }

                conv.updated_at = now;
                store.upsert(conv_id_for_save.clone(), conv);
                let _ = store.save(&data_dir);
            }
        }
    }

    result
        .map(|answer| SendMessageResult {
            conversation_id: conv_id_for_save,
            answer,
        })
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
struct SendMessageResult {
    conversation_id: String,
    answer: String,
}

/// List all conversations
#[tauri::command]
fn list_conversations(app: AppHandle) -> Result<Vec<ConversationSummary>, String> {
    let store = app.state::<Mutex<ConversationStore>>();
    let store = store.lock().map_err(|e| e.to_string())?;
    Ok(store.list())
}

/// Get a specific conversation
#[tauri::command]
fn get_conversation(app: AppHandle, id: String) -> Result<Option<StoredConversation>, String> {
    let store = app.state::<Mutex<ConversationStore>>();
    let store = store.lock().map_err(|e| e.to_string())?;
    Ok(store.get(&id).cloned())
}

/// Delete a conversation
#[tauri::command]
fn delete_conversation(app: AppHandle, id: String) -> Result<bool, String> {
    let store = app.state::<Mutex<ConversationStore>>();
    let mut store = store.lock().map_err(|e| e.to_string())?;
    let deleted = store.delete(&id);
    if deleted {
        if let Some(data_dir) = app.path().app_data_dir().ok() {
            let _ = store.save(&data_dir);
        }
    }
    Ok(deleted)
}

/// Clear current conversation (start fresh)
#[tauri::command]
fn clear_conversation() -> Result<(), String> {
    // This is handled client-side by clearing the chat state
    Ok(())
}

// ============================================================================
// Tauri Commands - Tools
// ============================================================================

/// Tool information with parameters for the UI
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ToolParameter>,
}

/// Tool parameter information
#[derive(Debug, Clone, Serialize)]
pub struct ToolParameter {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub param_type: String,
}

/// Get list of available tools with their schemas
#[tauri::command]
fn get_tools() -> Result<Vec<ToolInfo>, String> {
    let tools_paths = [
        std::path::PathBuf::from("tools.json"),
        std::path::PathBuf::from("../../tools.json"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools.json"),
    ];

    let tools_path = tools_paths
        .iter()
        .find(|p| p.exists())
        .ok_or_else(|| format!("tools.json not found in paths: {:?}", tools_paths))?;

    let executor = tools::ToolExecutor::load(tools_path)
        .map_err(|e| format!("Failed to load tools: {}", e))?;

    let tools: Vec<ToolInfo> = executor
        .all_tools()
        .map(|t| {
            // Extract parameters from the tool's parameters Value
            let mut parameters = Vec::new();

            if let Some(props) = t.parameters.get("properties").and_then(|p| p.as_object()) {
                let required: Vec<&str> = t.parameters.get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                for (name, schema) in props {
                    parameters.push(ToolParameter {
                        name: name.clone(),
                        description: schema.get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("")
                            .to_string(),
                        required: required.contains(&name.as_str()),
                        param_type: schema.get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("string")
                            .to_string(),
                    });
                }
            }

            ToolInfo {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters,
            }
        })
        .collect();

    Ok(tools)
}

// ============================================================================
// Tauri Commands - Utility
// ============================================================================

/// Get a greeting message (for testing)
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to Prolog Router.", name)
}

/// Open a note in Apple Notes app by its x-coredata ID
#[tauri::command]
fn open_note(note_id: String) -> Result<String, String> {
    prolog_router_core::apple_notes::open_note(&note_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// App Entry Point
// ============================================================================

/// Run the Tauri application
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Initialize conversation store from disk
            let data_dir = app.path().app_data_dir()
                .expect("Failed to get app data dir");
            let store = ConversationStore::load(&data_dir);
            app.manage(Mutex::new(store));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            greet,
            get_tools,
            list_conversations,
            get_conversation,
            delete_conversation,
            clear_conversation,
            open_note,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
