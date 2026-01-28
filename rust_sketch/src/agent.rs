//! Agentic Chat Loop for LLM-driven tool orchestration
//!
//! Transforms the single-shot intent→tool→output flow into an agentic loop:
//! user_query → [LOOP: LLM decides action → execute tool → feed result back] → final answer

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::{call_llm, LlmRequest};
use crate::tools::ToolExecutor;

// ============================================================================
// Agent Configuration
// ============================================================================

/// Configuration for the agent loop
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Maximum number of turns before stopping
    pub max_turns: u32,
    /// Whether to print verbose debug output
    pub verbose: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 10,
            verbose: false,
        }
    }
}

// ============================================================================
// Conversation State
// ============================================================================

/// Role of a message in the conversation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool: String,
    pub success: bool,
    pub output: String,
}

/// A single message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            tool_result: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_result: None,
        }
    }

    pub fn tool(tool_name: impl Into<String>, success: bool, output: impl Into<String>) -> Self {
        let tool = tool_name.into();
        let output = output.into();
        Self {
            role: MessageRole::Tool,
            content: format!("Tool {} returned: {}", tool, output),
            tool_result: Some(ToolResult {
                tool,
                success,
                output,
            }),
        }
    }
}

/// Tracks conversation state across turns
#[derive(Debug)]
pub struct ConversationState {
    pub messages: Vec<Message>,
    pub response_id: Option<String>,
    pub turn_count: u32,
    pub max_turns: u32,
}

impl ConversationState {
    pub fn new(max_turns: u32) -> Self {
        Self {
            messages: Vec::new(),
            response_id: None,
            turn_count: 0,
            max_turns,
        }
    }

    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(Message::user(content));
    }

    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(Message::assistant(content));
    }

    pub fn add_tool_result(&mut self, tool: &str, success: bool, output: impl Into<String>) {
        self.messages.push(Message::tool(tool, success, output));
    }

    /// Format conversation history for the LLM
    pub fn format_for_llm(&self) -> String {
        let mut parts = Vec::new();

        for msg in &self.messages {
            let prefix = match msg.role {
                MessageRole::User => "USER",
                MessageRole::Assistant => "ASSISTANT",
                MessageRole::Tool => "TOOL_RESULT",
            };
            parts.push(format!("{}:\n{}", prefix, msg.content));
        }

        parts.join("\n\n")
    }
}

// ============================================================================
// Agent Actions
// ============================================================================

/// Actions the agent can take
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentAction {
    /// Call a tool with arguments
    CallTool { tool: String, args: Value },
    /// Provide final answer to user
    FinalAnswer { answer: String },
    /// Ask user for more information
    AskUser { question: String },
}

// ============================================================================
// System Prompt
// ============================================================================

/// Default system prompt file path
const AGENT_PROMPT_FILE: &str = "prompts/agent_system.md";

/// Fallback prompt if file cannot be loaded
const FALLBACK_AGENT_PROMPT: &str = r#"You are an intelligent assistant that helps users by calling tools.
Today's date is {{TODAY}}.

## Available Tools
- get_apple_weather: Weather for location/date
- search_notes: Search user's notes
- search_files: Search user's files
- draft_email: Draft an email
- create_todo: Create a reminder

## Response Format
Respond with JSON only:

1. Call a tool:
   {"action": "call_tool", "tool": "get_apple_weather", "args": {"location": "Seattle", "date": "2026-01-27"}}

2. Final answer:
   {"action": "final_answer", "answer": "Based on the weather data..."}

3. Need more info:
   {"action": "ask_user", "question": "Which city?"}

## Rules
- After tool results, synthesize into a helpful answer
- Be concise but informative
"#;

/// Load the agent system prompt
fn load_agent_prompt(tools: Option<&ToolExecutor>, verbose: bool) -> String {
    use chrono::Local;
    use std::fs;
    use std::path::Path;

    let today = Local::now().format("%Y-%m-%d").to_string();

    // Try to load from file
    let prompt = if Path::new(AGENT_PROMPT_FILE).exists() {
        match fs::read_to_string(AGENT_PROMPT_FILE) {
            Ok(content) => {
                if verbose {
                    eprintln!("DEBUG: Loaded agent prompt from {}", AGENT_PROMPT_FILE);
                }
                content
            }
            Err(e) => {
                eprintln!(
                    "WARNING: Failed to read {}: {}, using fallback",
                    AGENT_PROMPT_FILE, e
                );
                FALLBACK_AGENT_PROMPT.to_string()
            }
        }
    } else {
        if verbose {
            eprintln!("DEBUG: Agent prompt file not found, using fallback");
        }
        FALLBACK_AGENT_PROMPT.to_string()
    };

    // Replace {{TODAY}} placeholder
    let mut prompt = prompt.replace("{{TODAY}}", &today);

    // If tools executor provided, inject tool descriptions
    if let Some(executor) = tools {
        let tool_list: Vec<String> = executor
            .all_tools()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect();

        if !tool_list.is_empty() {
            let tools_section = format!("\n## Available Tools\n{}\n", tool_list.join("\n"));
            // Try to replace existing tools section or append
            if prompt.contains("## Available Tools") {
                // Find the section and replace up to next ## or end
                if let Some(start) = prompt.find("## Available Tools") {
                    let after_header = &prompt[start + 18..];
                    let end_offset = after_header
                        .find("\n## ")
                        .unwrap_or(after_header.len());
                    let end = start + 18 + end_offset;
                    prompt = format!("{}{}{}", &prompt[..start], tools_section, &prompt[end..]);
                }
            }
        }
    }

    prompt
}

// ============================================================================
// Agent Loop
// ============================================================================

/// Parse agent action from LLM response
/// Extracts the LAST valid JSON object (in case LLM "thinks out loud" with multiple JSONs)
fn parse_agent_action(text: &str) -> Result<AgentAction> {
    let s = text.trim();

    // Fast path: try parsing the whole thing
    if s.starts_with('{') && s.ends_with('}') {
        if let Ok(action) = serde_json::from_str(s) {
            return Ok(action);
        }
    }

    // Find ALL JSON objects and return the last valid one
    let mut last_valid: Option<AgentAction> = None;
    let mut search_start = 0;

    while let Some(start) = s[search_start..].find('{') {
        let start = search_start + start;

        // Brace balancing to find matching '}'
        let mut in_str = false;
        let mut escape = false;
        let mut depth = 0;
        let mut end = None;

        for (i, ch) in s[start..].char_indices() {
            if in_str {
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    in_str = false;
                }
                continue;
            }

            match ch {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(end_pos) = end {
            let json_str = &s[start..end_pos];
            if let Ok(action) = serde_json::from_str::<AgentAction>(json_str) {
                last_valid = Some(action);
            }
            search_start = end_pos;
        } else {
            // No matching brace found, move past this '{'
            search_start = start + 1;
        }
    }

    last_valid.ok_or_else(|| anyhow!("No valid agent action JSON found in LLM output"))
}

/// Call LLM and get agent action
fn call_llm_for_action(
    state: &ConversationState,
    system_prompt: &str,
    verbose: bool,
) -> Result<AgentAction> {
    let conversation = state.format_for_llm();
    let input = format!("{}\n\n{}", system_prompt, conversation);

    let request = LlmRequest {
        input,
        instructions: None,
        // Don't use previous_response_id - not all endpoints support it
        // and we're already passing the full conversation history
        previous_response_id: None,
        verbose,
    };

    let response = call_llm(&request)?;

    if verbose {
        eprintln!("DEBUG: LLM response: {}", response.output_text);
    }

    parse_agent_action(&response.output_text)
}

/// Execute a tool and return the result
fn execute_tool(
    tool: &str,
    args: &Value,
    executor: Option<&ToolExecutor>,
) -> (bool, String) {
    use crate::apple_notes;
    use crate::apple_weather;

    // Apple Notes tools
    let notes_tools = [
        "search_notes",
        "list_notes",
        "get_note",
        "notes_index",
        "notes_tags",
        "notes_search_by_tag",
    ];
    if notes_tools.contains(&tool) {
        if apple_notes::is_available() {
            let action = match tool {
                "search_notes" => "search",
                "list_notes" => "list",
                "get_note" => "get",
                "notes_index" => {
                    // Check 'action' arg: "build" or "check" (default: check)
                    match args.get("action").and_then(|v| v.as_str()).unwrap_or("check") {
                        "build" => "index_build",
                        _ => "index_check",
                    }
                }
                "notes_tags" => "tags",
                "notes_search_by_tag" => "search_by_tag",
                _ => unreachable!(),
            };
            match apple_notes::execute_apple_notes(action, args) {
                Ok(result) => return (true, result),
                Err(e) => {
                    return (false, format!("Apple Notes error: {}", e));
                }
            }
        }
        // Fall through to executor or stub if Apple Notes not available
    }

    // Special handling for Apple WeatherKit
    if tool == "get_apple_weather" {
        if apple_weather::is_configured() {
            let location = args["location"].as_str().unwrap_or("NYC");
            let date = args.get("date").and_then(|v| v.as_str());
            let date_end = args.get("date_end").and_then(|v| v.as_str());
            let query_type = args
                .get("weather_query")
                .and_then(|v| v.as_str())
                .map(apple_weather::QueryType::from_str)
                .unwrap_or_default();

            match apple_weather::execute_apple_weather(location, date, date_end, query_type) {
                Ok(result) => return (true, result),
                Err(e) => {
                    eprintln!("WARNING: Apple Weather failed: {}", e);
                    return (false, format!("Error: {}", e));
                }
            }
        }
    }

    // Try to execute via configured endpoint
    if let Some(exec) = executor {
        if exec.has_endpoint(tool) {
            match exec.execute(tool, args) {
                Ok(Some(result)) => return (true, result),
                Ok(None) => {
                    // No endpoint configured, fall through to stub
                }
                Err(e) => {
                    eprintln!("WARNING: Tool execution failed: {}", e);
                    return (false, format!("Error: {}", e));
                }
            }
        }
    }

    // Fall back to stub
    let result = match tool {
        "search_notes" => format!("[stub] searched notes for: {}", args),
        "search_files" => format!("[stub] searched files for: {}", args),
        "get_weather" => format!("[stub] weather result for: {}", args),
        "get_apple_weather" => format!("[stub] apple weather result for: {}", args),
        "draft_email" => format!("[stub] drafted email with: {}", args),
        "create_todo" => format!("[stub] created todo with: {}", args),
        _ => format!("[stub] unknown tool: {} args={}", tool, args),
    };

    (true, result)
}

/// Run the main agent loop
///
/// Transforms user query into a series of LLM → tool → LLM interactions
/// until the agent provides a final answer or reaches max turns.
pub fn run_agent_loop(
    query: &str,
    config: &AgentConfig,
    executor: Option<&ToolExecutor>,
) -> Result<String> {
    let mut state = ConversationState::new(config.max_turns);
    state.add_user_message(query);

    let system_prompt = load_agent_prompt(executor, config.verbose);

    if config.verbose {
        eprintln!("DEBUG: Starting agent loop with max_turns={}", config.max_turns);
    }

    loop {
        if state.turn_count >= state.max_turns {
            return Ok(format!(
                "Max turns ({}) reached. Last context: {}",
                state.max_turns,
                state.messages.last().map(|m| &m.content[..]).unwrap_or("")
            ));
        }

        let action = call_llm_for_action(&state, &system_prompt, config.verbose)?;
        state.turn_count += 1;

        if config.verbose {
            eprintln!("DEBUG: Turn {}: {:?}", state.turn_count, action);
        }

        match action {
            AgentAction::FinalAnswer { answer } => {
                return Ok(answer);
            }

            AgentAction::AskUser { question } => {
                return Ok(format!("Need more information: {}", question));
            }

            AgentAction::CallTool { tool, args } => {
                if config.verbose {
                    eprintln!("DEBUG: Calling tool '{}' with args: {}", tool, args);
                }

                let (success, output) = execute_tool(&tool, &args, executor);

                if config.verbose {
                    eprintln!(
                        "DEBUG: Tool result (success={}): {}",
                        success,
                        if output.len() > 200 {
                            format!("{}...", &output[..200])
                        } else {
                            output.clone()
                        }
                    );
                }

                // Add tool result to conversation and continue loop
                state.add_tool_result(&tool, success, output);
                // Record assistant's tool call decision
                state.add_assistant_message(format!(
                    "Called tool {} with args: {}",
                    tool,
                    serde_json::to_string(&args).unwrap_or_default()
                ));
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agent_action_call_tool() {
        let json = r#"{"action": "call_tool", "tool": "get_weather", "args": {"location": "NYC"}}"#;
        let action = parse_agent_action(json).unwrap();
        match action {
            AgentAction::CallTool { tool, args } => {
                assert_eq!(tool, "get_weather");
                assert_eq!(args["location"], "NYC");
            }
            _ => panic!("Expected CallTool action"),
        }
    }

    #[test]
    fn test_parse_agent_action_final_answer() {
        let json = r#"{"action": "final_answer", "answer": "The weather is sunny."}"#;
        let action = parse_agent_action(json).unwrap();
        match action {
            AgentAction::FinalAnswer { answer } => {
                assert_eq!(answer, "The weather is sunny.");
            }
            _ => panic!("Expected FinalAnswer action"),
        }
    }

    #[test]
    fn test_parse_agent_action_ask_user() {
        let json = r#"{"action": "ask_user", "question": "Which city?"}"#;
        let action = parse_agent_action(json).unwrap();
        match action {
            AgentAction::AskUser { question } => {
                assert_eq!(question, "Which city?");
            }
            _ => panic!("Expected AskUser action"),
        }
    }

    #[test]
    fn test_parse_agent_action_with_surrounding_text() {
        let text = r#"Let me check the weather.
        {"action": "call_tool", "tool": "get_weather", "args": {"location": "Seattle"}}
        "#;
        let action = parse_agent_action(text).unwrap();
        match action {
            AgentAction::CallTool { tool, .. } => {
                assert_eq!(tool, "get_weather");
            }
            _ => panic!("Expected CallTool action"),
        }
    }

    #[test]
    fn test_conversation_state_format() {
        let mut state = ConversationState::new(10);
        state.add_user_message("What's the weather in NYC?");
        state.add_tool_result("get_weather", true, "Sunny, 72F");
        state.add_assistant_message("The weather in NYC is sunny with 72F.");

        let formatted = state.format_for_llm();
        assert!(formatted.contains("USER:"));
        assert!(formatted.contains("TOOL_RESULT:"));
        assert!(formatted.contains("ASSISTANT:"));
        assert!(formatted.contains("What's the weather in NYC?"));
        assert!(formatted.contains("Sunny, 72F"));
    }

    #[test]
    fn test_message_constructors() {
        let user = Message::user("Hello");
        assert_eq!(user.role, MessageRole::User);
        assert_eq!(user.content, "Hello");
        assert!(user.tool_result.is_none());

        let tool = Message::tool("weather", true, "Sunny");
        assert_eq!(tool.role, MessageRole::Tool);
        assert!(tool.tool_result.is_some());
        let tr = tool.tool_result.unwrap();
        assert_eq!(tr.tool, "weather");
        assert!(tr.success);
        assert_eq!(tr.output, "Sunny");
    }
}
