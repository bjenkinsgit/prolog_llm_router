//! LLM-based intent extraction using OpenAI-compatible API

use anyhow::{anyhow, Result};
use chrono::Local;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::{Constraints, Entities, IntentPayload, IntentType, SourcePreference, WeatherQueryType};

const LLM_BASE_URL: &str = "http://alien.local:8000/v1";
const LLM_MODEL: &str = "openai/gpt-oss-20b";

/// Default prompt file path (relative to working directory)
const PROMPT_FILE: &str = "prompts/intent_extractor.md";

/// Fallback prompt if file cannot be loaded
const FALLBACK_PROMPT: &str = r#"You are an intent extractor for a tool-routing system.
Extract the user's intent and relevant entities from their message.
Output ONLY a single JSON object. No markdown. No explanation.

Schema:
{
  "intent": "summarize|find|draft|remind|weather|unknown",
  "entities": {
    "topic": string or null,
    "query": string or null,
    "location": string or null,
    "date": "YYYY-MM-DD or null",
    "date_end": "YYYY-MM-DD or null",
    "recipient": string or null,
    "priority": string or null,
    "weather_query": "current|forecast|assessment" or null
  },
  "constraints": {
    "source_preference": "notes|files|either",
    "safety": "normal"
  }
}

Rules:
- Choose intent from: summarize, find, draft, remind, weather, unknown
- Convert ALL dates to YYYY-MM-DD format
- For date ranges, set both date (start) and date_end
- weather_query: "current" (default), "forecast" (multi-day), "assessment" (bad weather check)
"#;

/// Load the system prompt from file, with {{TODAY}} substitution
fn load_system_prompt(verbose: bool) -> String {
    let today = Local::now().format("%Y-%m-%d").to_string();

    // Try to load from file
    let prompt = if Path::new(PROMPT_FILE).exists() {
        match fs::read_to_string(PROMPT_FILE) {
            Ok(content) => {
                if verbose {
                    eprintln!("DEBUG: Loaded prompt from {}", PROMPT_FILE);
                }
                content
            }
            Err(e) => {
                eprintln!("WARNING: Failed to read {}: {}, using fallback", PROMPT_FILE, e);
                FALLBACK_PROMPT.to_string()
            }
        }
    } else {
        if verbose {
            eprintln!("DEBUG: Prompt file not found, using fallback");
        }
        FALLBACK_PROMPT.to_string()
    };

    // Replace {{TODAY}} placeholder with actual date
    prompt.replace("{{TODAY}}", &today)
}

/// Request body for the Responses API (internal)
#[derive(Serialize)]
struct ResponsesApiRequest {
    model: String,
    input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
}

/// Public LLM request for generic calls
#[derive(Debug, Clone, Default)]
pub struct LlmRequest {
    /// The input text/prompt
    pub input: String,
    /// Optional system instructions
    pub instructions: Option<String>,
    /// Previous response ID for conversation continuation
    pub previous_response_id: Option<String>,
    /// Enable verbose debug output
    pub verbose: bool,
}

/// Public LLM response
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LlmResponse {
    /// Response ID for continuation
    pub id: Option<String>,
    /// The text output from the model
    pub output_text: String,
}

/// Response from the Responses API
#[derive(Deserialize, Debug)]
struct ResponsesApiResponse {
    /// Response ID for conversation continuation
    #[serde(default)]
    id: Option<String>,
    output: Vec<OutputItem>,
    #[serde(default)]
    output_text: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OutputItem {
    #[serde(default)]
    content: Vec<ContentItem>,
}

#[derive(Deserialize, Debug)]
struct ContentItem {
    #[serde(default)]
    text: Option<String>,
}

/// Raw intent structure from LLM (more permissive than our strict types)
#[derive(Deserialize, Debug)]
struct RawIntentPayload {
    intent: Option<String>,
    entities: Option<RawEntities>,
    constraints: Option<RawConstraints>,
}

#[derive(Deserialize, Debug, Default)]
struct RawEntities {
    topic: Option<String>,
    query: Option<String>,
    location: Option<String>,
    date: Option<String>,
    date_end: Option<String>,
    recipient: Option<String>,
    priority: Option<String>,
    weather_query: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct RawConstraints {
    source_preference: Option<String>,
    safety: Option<String>,
}

/// Extract intent from user text using LLM
pub fn extract_intent_llm(user_text: &str, verbose: bool) -> Result<IntentPayload> {
    let system_prompt = load_system_prompt(verbose);

    let request = LlmRequest {
        input: format!("{}\n\nUSER:\n{}", system_prompt, user_text),
        instructions: None,
        previous_response_id: None,
        verbose,
    };

    let response = call_llm(&request)?;

    if verbose {
        eprintln!("DEBUG: LLM raw response: {}", response.output_text);
    }

    // Parse JSON from text
    let raw = parse_json_from_text(&response.output_text)?;

    // Normalize to our strict types
    Ok(normalize_intent_payload(raw))
}

/// Generic LLM call function for agent and other uses
pub fn call_llm(request: &LlmRequest) -> Result<LlmResponse> {
    let client = Client::new();

    let api_request = ResponsesApiRequest {
        model: LLM_MODEL.to_string(),
        input: request.input.clone(),
        instructions: request.instructions.clone(),
        previous_response_id: request.previous_response_id.clone(),
    };

    if request.verbose {
        eprintln!("DEBUG: Calling LLM at {}/responses", LLM_BASE_URL);
    }

    let response = client
        .post(format!("{}/responses", LLM_BASE_URL))
        .json(&api_request)
        .send()
        .map_err(|e| anyhow!("LLM request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("LLM error {}: {}", status, body));
    }

    let resp: ResponsesApiResponse = response
        .json()
        .map_err(|e| anyhow!("Failed to parse LLM response: {}", e))?;

    // Extract text from response
    let output_text = extract_text_from_response(&resp)?;

    Ok(LlmResponse {
        id: resp.id,
        output_text,
    })
}

/// Extract text content from Responses API response
fn extract_text_from_response(resp: &ResponsesApiResponse) -> Result<String> {
    let mut chunks = Vec::new();

    for item in &resp.output {
        for content in &item.content {
            if let Some(ref text) = content.text {
                chunks.push(text.clone());
            }
        }
    }

    // Fallback to output_text if no content found
    if chunks.is_empty() {
        if let Some(ref text) = resp.output_text {
            chunks.push(text.clone());
        }
    }

    if chunks.is_empty() {
        return Err(anyhow!("No text found in LLM response"));
    }

    Ok(chunks.join("\n").trim().to_string())
}

/// Extract the first JSON object from text using brace balancing
fn parse_json_from_text(text: &str) -> Result<RawIntentPayload> {
    let s = text.trim();

    // Fast path: try parsing the whole thing
    if s.starts_with('{') {
        if let Ok(parsed) = serde_json::from_str(s) {
            return Ok(parsed);
        }
    }

    // Find first '{'
    let start = s.find('{').ok_or_else(|| anyhow!("No '{{' found in LLM output"))?;

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

    let end = end.ok_or_else(|| anyhow!("No matching '}}' found in LLM output"))?;
    let json_str = &s[start..end];

    serde_json::from_str(json_str).map_err(|e| anyhow!("Failed to parse JSON: {}", e))
}

/// Normalize raw LLM output to our strict types
fn normalize_intent_payload(raw: RawIntentPayload) -> IntentPayload {
    // Map intent string to enum
    let intent = match raw.intent.as_deref().map(|s| s.to_lowercase()) {
        Some(s) => match s.as_str() {
            "summarize" | "summary" | "summarise" => IntentType::Summarize,
            "find" | "search" | "lookup" | "locate" | "query" => IntentType::Find,
            "draft" | "email" | "compose" | "write_email" => IntentType::Draft,
            "remind" | "reminder" | "todo" | "task" => IntentType::Remind,
            "weather" | "forecast" => IntentType::Weather,
            _ => IntentType::Unknown,
        },
        None => IntentType::Unknown,
    };

    // Map entities
    let raw_entities = raw.entities.unwrap_or_default();

    // Map weather_query string to enum
    let weather_query = raw_entities.weather_query.as_deref().and_then(|s| {
        match s.to_lowercase().as_str() {
            "current" => Some(WeatherQueryType::Current),
            "forecast" => Some(WeatherQueryType::Forecast),
            "assessment" => Some(WeatherQueryType::Assessment),
            _ => None,
        }
    });

    let entities = Entities {
        topic: raw_entities.topic,
        query: raw_entities.query,
        location: raw_entities.location,
        date: raw_entities.date,
        date_end: raw_entities.date_end,
        recipient: raw_entities.recipient,
        priority: raw_entities.priority,
        weather_query,
    };

    // Map constraints
    let raw_constraints = raw.constraints.unwrap_or_default();
    let source_preference = match raw_constraints.source_preference.as_deref() {
        Some("notes") => SourcePreference::Notes,
        Some("files") => SourcePreference::Files,
        _ => SourcePreference::Either,
    };

    let constraints = Constraints {
        source_preference,
        safety: raw_constraints.safety.unwrap_or_else(|| "normal".to_string()),
    };

    IntentPayload {
        intent,
        entities,
        constraints,
    }
}
