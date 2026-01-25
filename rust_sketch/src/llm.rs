//! LLM-based intent extraction using OpenAI-compatible API

use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{Constraints, Entities, IntentPayload, IntentType, SourcePreference};

const LLM_BASE_URL: &str = "http://alien.local:8000/v1";
const LLM_MODEL: &str = "openai/gpt-oss-20b";

const SYSTEM_PROMPT: &str = r#"You are an intent extractor for a tool-routing system.
Extract the user's intent and relevant entities from their message.
Output ONLY a single JSON object. No markdown. No explanation.

Schema:
{
  "intent": "summarize|find|draft|remind|weather|unknown",
  "entities": {
    "topic": string or null,
    "query": string or null,
    "location": string or null,
    "date": string or null,
    "recipient": string or null,
    "priority": string or null
  },
  "constraints": {
    "source_preference": "notes|files|either",
    "safety": "normal"
  }
}

Rules:
- Choose intent from: summarize, find, draft, remind, weather, unknown
- Put the main subject into entities.topic when relevant
- If user explicitly mentions notes/files, set source_preference accordingly; otherwise "either"
- If missing critical info (e.g. weather without location/date), leave those entities as null
- Extract dates in natural language form (e.g., "tomorrow", "next Friday", "2026-02-10")
- Do not output anything after the final closing brace '}'
"#;

/// Request body for the Responses API
#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
}

/// Response from the Responses API
#[derive(Deserialize, Debug)]
struct ResponsesResponse {
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
    recipient: Option<String>,
    priority: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct RawConstraints {
    source_preference: Option<String>,
    safety: Option<String>,
}

/// Extract intent from user text using LLM
pub fn extract_intent_llm(user_text: &str) -> Result<IntentPayload> {
    let client = Client::new();

    let request = ResponsesRequest {
        model: LLM_MODEL.to_string(),
        input: format!("{}\n\nUSER:\n{}", SYSTEM_PROMPT, user_text),
        instructions: None,
    };

    eprintln!("DEBUG: Calling LLM at {}/responses", LLM_BASE_URL);

    let response = client
        .post(format!("{}/responses", LLM_BASE_URL))
        .json(&request)
        .send()
        .map_err(|e| anyhow!("LLM request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("LLM error {}: {}", status, body));
    }

    let resp: ResponsesResponse = response
        .json()
        .map_err(|e| anyhow!("Failed to parse LLM response: {}", e))?;

    // Extract text from response
    let text = extract_text_from_response(&resp)?;
    eprintln!("DEBUG: LLM raw response: {}", text);

    // Parse JSON from text
    let raw = parse_json_from_text(&text)?;

    // Normalize to our strict types
    Ok(normalize_intent_payload(raw))
}

/// Extract text content from Responses API response
fn extract_text_from_response(resp: &ResponsesResponse) -> Result<String> {
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
    let entities = Entities {
        topic: raw_entities.topic,
        query: raw_entities.query,
        location: raw_entities.location,
        date: raw_entities.date,
        recipient: raw_entities.recipient,
        priority: raw_entities.priority,
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
