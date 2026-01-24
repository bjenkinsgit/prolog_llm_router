//! Prolog-based LLM Intent Router in Rust
//!
//! This demonstrates the DRY principle: one set of structs for both
//! JSON serialization (serde) AND Prolog term conversion (swipl).

use anyhow::Result;
use serde::{Deserialize, Serialize};

// Note: In a real implementation, you'd uncomment this:
// use swipl::prelude::*;

mod derive_sketch;

// ============================================================================
// APPROACH 1: Dual Derive (Ideal but requires swipl to support it well)
// ============================================================================
//
// The dream: derive BOTH serde and Prolog traits on the same struct.
// This works if swipl's Unifiable maps cleanly to your Prolog representation.

/// Intent types supported by the router
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentType {
    Summarize,
    Find,
    Draft,
    Remind,
    Weather,
    Unknown,
}

impl IntentType {
    /// Convert to Prolog atom string
    fn as_atom(&self) -> &'static str {
        match self {
            IntentType::Summarize => "summarize",
            IntentType::Find => "find",
            IntentType::Draft => "draft",
            IntentType::Remind => "remind",
            IntentType::Weather => "weather",
            IntentType::Unknown => "unknown",
        }
    }
}

/// Source preference for search operations
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePreference {
    Notes,
    Files,
    #[default]
    Either,
}

/// Extracted entities from user input
///
/// This struct is used for:
/// 1. Parsing JSON from LLM structured output (serde)
/// 2. Passing to Prolog as a dict term (manual or derive)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// Routing constraints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraints {
    #[serde(default)]
    pub source_preference: SourcePreference,

    #[serde(default = "default_safety")]
    pub safety: String,
}

fn default_safety() -> String {
    "normal".to_string()
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            source_preference: SourcePreference::Either,
            safety: "normal".to_string(),
        }
    }
}

/// The main intent payload - used for both JSON and Prolog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentPayload {
    pub intent: IntentType,

    #[serde(default)]
    pub entities: Entities,

    #[serde(default)]
    pub constraints: Constraints,
}

// ============================================================================
// APPROACH 2: Trait-based conversion (more explicit, always works)
// ============================================================================
//
// Define a trait that converts our structs to Prolog dict syntax.
// This is similar to what we do in Python but type-safe.

/// Trait for converting Rust types to SWI-Prolog dict term strings
pub trait ToPrologDict {
    fn to_prolog_dict(&self) -> String;
}

impl ToPrologDict for Entities {
    fn to_prolog_dict(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref v) = self.topic {
            parts.push(format!("topic:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.query {
            parts.push(format!("query:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.location {
            parts.push(format!("location:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.date {
            parts.push(format!("date:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.recipient {
            parts.push(format!("recipient:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.priority {
            parts.push(format!("priority:\"{}\"", escape_prolog_string(v)));
        }

        format!("_{{{}}}", parts.join(", "))
    }
}

impl ToPrologDict for Constraints {
    fn to_prolog_dict(&self) -> String {
        let source_pref = match self.source_preference {
            SourcePreference::Notes => "notes",
            SourcePreference::Files => "files",
            SourcePreference::Either => "either",
        };

        format!(
            "_{{source_preference:\"{}\", safety:\"{}\"}}",
            source_pref,
            escape_prolog_string(&self.safety)
        )
    }
}

fn escape_prolog_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ============================================================================
// APPROACH 3: Use serde_json::Value as intermediate (most flexible)
// ============================================================================
//
// Both serde and Prolog can work with a generic JSON-like Value type.
// This avoids needing special derives - just convert through JSON.

pub trait ToPrologDictViaJson: Serialize {
    fn to_prolog_dict_via_json(&self) -> Result<String> {
        let value = serde_json::to_value(self)?;
        Ok(json_value_to_prolog_dict(&value))
    }
}

// Blanket implementation for anything that implements Serialize
impl<T: Serialize> ToPrologDictViaJson for T {}

fn json_value_to_prolog_dict(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("\"{}\"", escape_prolog_string(s)),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_value_to_prolog_dict).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let pairs: Vec<String> = map
                .iter()
                .filter(|(_, v)| !v.is_null()) // Skip null values like Python version
                .map(|(k, v)| format!("{}:{}", k, json_value_to_prolog_dict(v)))
                .collect();
            format!("_{{{}}}", pairs.join(", "))
        }
    }
}

// ============================================================================
// Router implementation using swipl
// ============================================================================

#[derive(Debug)]
pub enum Decision {
    Route { tool: String, args: serde_json::Value },
    NeedInfo { question: String },
    Reject { reason: String },
}

/// Query the Prolog router
pub fn prolog_decide(payload: &IntentPayload) -> Result<Decision> {
    // In a real implementation, you'd initialize the Prolog engine once
    // and reuse it. This is simplified for illustration.

    let intent_atom = payload.intent.as_atom();
    let entities_dict = payload.entities.to_prolog_dict();
    let constraints_dict = payload.constraints.to_prolog_dict();

    // Build the query string
    let query = format!(
        "route({}, {}, {}, Tool, Args)",
        intent_atom, entities_dict, constraints_dict
    );

    println!("Prolog query: {}", query);

    // Here you would actually call swipl:
    //
    // let engine = Engine::new();
    // engine.call("consult('router.pl')")?;
    // let results = engine.query(&query)?;
    //
    // For now, return a stub:
    Ok(Decision::Route {
        tool: "stub_tool".to_string(),
        args: serde_json::json!({"query": "stub"}),
    })
}

// ============================================================================
// Main - demonstrating the DRY principle
// ============================================================================

fn main() -> Result<()> {
    // Example 1: Parse JSON from LLM (serde)
    let json_input = r#"{
        "intent": "summarize",
        "entities": {
            "topic": "machine learning",
            "date": null
        },
        "constraints": {
            "source_preference": "notes"
        }
    }"#;

    let payload: IntentPayload = serde_json::from_str(json_input)?;
    println!("Parsed from JSON:");
    println!("  Intent: {:?}", payload.intent);
    println!("  Topic: {:?}", payload.entities.topic);
    println!();

    // Example 2: Convert SAME struct to Prolog dict (manual trait)
    println!("Prolog dict (manual trait):");
    println!("  Entities: {}", payload.entities.to_prolog_dict());
    println!("  Constraints: {}", payload.constraints.to_prolog_dict());
    println!();

    // Example 3: Convert via JSON intermediate (most flexible)
    println!("Prolog dict (via JSON):");
    println!("  Entities: {}", payload.entities.to_prolog_dict_via_json()?);
    println!("  Full payload: {}", payload.to_prolog_dict_via_json()?);
    println!();

    // Example 4: Query Prolog
    let decision = prolog_decide(&payload)?;
    println!("Decision: {:?}", decision);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_json() {
        let payload = IntentPayload {
            intent: IntentType::Weather,
            entities: Entities {
                location: Some("NYC".to_string()),
                date: Some("tomorrow".to_string()),
                ..Default::default()
            },
            constraints: Constraints::default(),
        };

        // Serialize to JSON
        let json = serde_json::to_string(&payload).unwrap();

        // Deserialize back
        let recovered: IntentPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(payload.intent, recovered.intent);
        assert_eq!(payload.entities.location, recovered.entities.location);
    }

    #[test]
    fn test_prolog_dict_generation() {
        let entities = Entities {
            topic: Some("AI notes".to_string()),
            location: None,
            ..Default::default()
        };

        let dict = entities.to_prolog_dict();
        assert!(dict.contains("topic:\"AI notes\""));
        assert!(!dict.contains("location")); // None fields should be skipped
    }

    #[test]
    fn test_json_to_prolog_consistency() {
        let entities = Entities {
            topic: Some("test".to_string()),
            query: Some("search term".to_string()),
            ..Default::default()
        };

        let manual = entities.to_prolog_dict();
        let via_json = entities.to_prolog_dict_via_json().unwrap();

        // Both should produce valid Prolog dict syntax
        assert!(manual.starts_with("_{"));
        assert!(via_json.starts_with("_{"));
    }
}
