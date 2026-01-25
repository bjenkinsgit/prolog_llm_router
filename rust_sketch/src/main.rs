//! Prolog-based LLM Intent Router in Rust
//!
//! This is a port of the Python router, demonstrating the DRY principle:
//! one set of structs for both JSON serialization (serde) AND Prolog term conversion.
//!
//! Supports two Prolog backends (via feature flags):
//! - `swipl-backend`: Uses SWI-Prolog via FFI (requires SWI-Prolog installed)
//! - `scryer-backend`: Uses Scryer Prolog (pure Rust, no external dependencies)

use anyhow::Result;
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod derive_sketch;
mod llm;

#[cfg(feature = "swipl-backend")]
mod prolog;

#[cfg(feature = "scryer-backend")]
mod scryer;

// ============================================================================
// CLI Arguments
// ============================================================================

#[derive(Parser, Debug)]
#[command(name = "prolog-router")]
#[command(about = "Route user intents to tools via Prolog rules")]
struct Args {
    /// User request in natural language
    user_text: String,

    /// Optional date (e.g., today, tomorrow, 2026-02-10)
    #[arg(long)]
    date: Option<String>,

    /// Optional location for weather
    #[arg(long)]
    location: Option<String>,

    /// Optional recipient for draft email
    #[arg(long)]
    recipient: Option<String>,

    /// Optional source preference
    #[arg(long, value_enum)]
    source: Option<SourcePreferenceArg>,

    /// Use the LLM intent extractor (calls http://alien.local:8000/v1)
    #[arg(long = "use-llm")]
    use_llm: bool,

    /// Use stub Prolog router instead of real Prolog
    #[arg(long = "stub")]
    use_stub: bool,

    /// Path to router.pl file (use router_standard.pl for Scryer backend)
    #[arg(long = "router")]
    router_path: Option<PathBuf>,
}

#[derive(Debug, Clone, ValueEnum)]
enum SourcePreferenceArg {
    Notes,
    Files,
    Either,
}

impl From<SourcePreferenceArg> for SourcePreference {
    fn from(arg: SourcePreferenceArg) -> Self {
        match arg {
            SourcePreferenceArg::Notes => SourcePreference::Notes,
            SourcePreferenceArg::Files => SourcePreference::Files,
            SourcePreferenceArg::Either => SourcePreference::Either,
        }
    }
}

// ============================================================================
// Domain Models (shared between serde JSON and Prolog)
// ============================================================================

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
// Prolog Dict Conversion (DRY approach via JSON intermediate)
// ============================================================================

/// Trait for converting Rust types to SWI-Prolog dict syntax
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
// Prolog List Conversion (for Scryer and standard Prolog)
// ============================================================================

/// Trait for converting Rust types to standard Prolog list syntax [key-value, ...]
pub trait ToPrologList {
    fn to_prolog_list(&self) -> String;
}

impl ToPrologList for Entities {
    fn to_prolog_list(&self) -> String {
        let mut parts = Vec::new();

        // Use single quotes for atoms in Scryer Prolog
        if let Some(ref v) = self.topic {
            parts.push(format!("topic-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.query {
            parts.push(format!("query-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.location {
            parts.push(format!("location-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.date {
            parts.push(format!("date-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.recipient {
            parts.push(format!("recipient-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.priority {
            parts.push(format!("priority-'{}'", escape_prolog_atom(v)));
        }

        format!("[{}]", parts.join(", "))
    }
}

impl ToPrologList for Constraints {
    fn to_prolog_list(&self) -> String {
        let source_pref = match self.source_preference {
            SourcePreference::Notes => "notes",
            SourcePreference::Files => "files",
            SourcePreference::Either => "either",
        };

        format!(
            "[source_preference-{}, safety-'{}']",
            source_pref,
            escape_prolog_atom(&self.safety)
        )
    }
}

fn escape_prolog_atom(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

// ============================================================================
// Date Resolution (converts relative dates to absolute YYYY-MM-DD)
// ============================================================================

/// Resolve relative date strings to absolute YYYY-MM-DD format
pub fn resolve_relative_date(date_str: &str) -> String {
    let today = Local::now().date_naive();
    let lower = date_str.to_lowercase();
    let trimmed = lower.trim();

    match trimmed {
        "today" => format_date(today),
        "tomorrow" => format_date(today + Days::new(1)),
        "yesterday" => format_date(today - Days::new(1)),
        _ => {
            // Try to parse "next <weekday>" patterns
            if let Some(weekday_str) = trimmed.strip_prefix("next ") {
                if let Some(target_weekday) = parse_weekday(weekday_str.trim()) {
                    return format_date(next_weekday(today, target_weekday));
                }
            }

            // Try to parse standalone weekday (means "this coming <weekday>")
            if let Some(target_weekday) = parse_weekday(trimmed) {
                return format_date(next_weekday(today, target_weekday));
            }

            // Already absolute or unrecognized - pass through
            date_str.to_string()
        }
    }
}

/// Format a NaiveDate as YYYY-MM-DD
fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Parse weekday name to chrono::Weekday
fn parse_weekday(s: &str) -> Option<Weekday> {
    match s.to_lowercase().as_str() {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" | "thurs" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Find the next occurrence of a weekday (1-7 days from today)
fn next_weekday(from: NaiveDate, target: Weekday) -> NaiveDate {
    let current = from.weekday();
    let days_ahead = (target.num_days_from_monday() as i64
        - current.num_days_from_monday() as i64
        + 7) % 7;
    // If it's the same day, go to next week
    let days_ahead = if days_ahead == 0 { 7 } else { days_ahead as u64 };
    from + Days::new(days_ahead)
}

/// Get today's date in YYYY-MM-DD format
pub fn today_date() -> String {
    format_date(Local::now().date_naive())
}

// ============================================================================
// Stub Intent Extractor (heuristic-based, like Python version)
// ============================================================================

/// Simple heuristic intent extractor for testing routing
fn extract_intent_stub(user_text: &str) -> IntentPayload {
    let t = user_text.to_lowercase();

    // Detect source preference
    let source_pref = if t.contains("notes") {
        SourcePreference::Notes
    } else if t.contains("files") {
        SourcePreference::Files
    } else {
        SourcePreference::Either
    };

    // Naive intent detection
    let intent = if t.starts_with("summarize") || t.contains("summarize") {
        IntentType::Summarize
    } else if t.starts_with("find") || t.starts_with("search") || t.contains("look for") {
        IntentType::Find
    } else if t.contains("weather") {
        IntentType::Weather
    } else if t.contains("email") || t.starts_with("draft") {
        IntentType::Draft
    } else if t.contains("remind") || t.contains("todo") {
        IntentType::Remind
    } else {
        IntentType::Unknown
    };

    // Naive topic extraction: everything after "about"
    let topic = if let Some(idx) = t.find("about") {
        let after_about = &user_text[idx + 5..];
        Some(after_about.trim().to_string())
    } else {
        // Or after the first word for summarize/find
        let pieces: Vec<&str> = user_text.splitn(2, ' ').collect();
        if pieces.len() == 2 && matches!(intent, IntentType::Summarize | IntentType::Find) {
            Some(pieces[1].trim().to_string())
        } else {
            None
        }
    };

    // Weather date extraction (toy)
    let date = if intent == IntentType::Weather {
        if t.contains("tomorrow") {
            Some("tomorrow".to_string())
        } else if t.contains("today") {
            Some("today".to_string())
        } else {
            None
        }
    } else {
        None
    };

    // Recipient extraction for draft/email (look for "email to <name>" or "to <name>" after email keyword)
    let recipient = if intent == IntentType::Draft {
        // Try "email to X" pattern first
        if let Some(idx) = t.find("email to ") {
            let after_to = &user_text[idx + 9..];
            let recipient_word = after_to.split_whitespace().next();
            recipient_word.map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
        // Try "mail to X" pattern
        } else if let Some(idx) = t.find("mail to ") {
            let after_to = &user_text[idx + 8..];
            let recipient_word = after_to.split_whitespace().next();
            recipient_word.map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
        // Try finding "to X" after "email" keyword
        } else if let Some(email_idx) = t.find("email") {
            let after_email = &t[email_idx..];
            if let Some(to_idx) = after_email.find(" to ") {
                let global_idx = email_idx + to_idx + 4;
                let after_to = &user_text[global_idx..];
                let recipient_word = after_to.split_whitespace().next();
                recipient_word.map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    IntentPayload {
        intent,
        entities: Entities {
            topic,
            date,
            recipient,
            ..Default::default()
        },
        constraints: Constraints {
            source_preference: source_pref,
            ..Default::default()
        },
    }
}

// ============================================================================
// Stub Prolog Router (simulates what router.pl would return)
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Decision {
    Route {
        tool: String,
        args: serde_json::Value,
    },
    NeedInfo {
        question: String,
    },
    Reject {
        reason: String,
    },
}

/// Stub Prolog router - simulates router.pl decisions
pub fn prolog_decide_stub(payload: &IntentPayload) -> Decision {
    let intent = &payload.intent;
    let entities = &payload.entities;
    let constraints = &payload.constraints;

    // Print the query that would be sent to Prolog
    let query = format!(
        "route({}, {}, {}, Tool, Args)",
        intent.as_atom(),
        entities.to_prolog_dict(),
        constraints.to_prolog_dict()
    );
    eprintln!("DEBUG: Prolog query: {}", query);

    // Simulate routing logic from router.pl
    match intent {
        IntentType::Summarize | IntentType::Find => {
            let query_text = entities
                .topic
                .clone()
                .or_else(|| entities.query.clone())
                .unwrap_or_default();

            if query_text.is_empty() {
                return Decision::NeedInfo {
                    question: format!("What should I {}?", intent.as_atom()),
                };
            }

            let tool = match constraints.source_preference {
                SourcePreference::Notes => "search_notes",
                _ => "search_files",
            };

            Decision::Route {
                tool: tool.to_string(),
                args: serde_json::json!({
                    "query": query_text,
                    "scope": "user"
                }),
            }
        }

        IntentType::Weather => {
            let location = match &entities.location {
                Some(loc) => loc.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "What location should I use?".to_string(),
                    }
                }
            };

            let date = match &entities.date {
                Some(d) => d.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "What date should I use? (e.g., today, tomorrow)".to_string(),
                    }
                }
            };

            Decision::Route {
                tool: "get_weather".to_string(),
                args: serde_json::json!({
                    "location": location,
                    "date": date
                }),
            }
        }

        IntentType::Draft => {
            let recipient = match &entities.recipient {
                Some(r) => r.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "Who should I email?".to_string(),
                    }
                }
            };

            let subject = entities.topic.clone().unwrap_or_else(|| "(no subject)".to_string());

            Decision::Route {
                tool: "draft_email".to_string(),
                args: serde_json::json!({
                    "to": recipient,
                    "subject": subject,
                    "body": ""
                }),
            }
        }

        IntentType::Remind => {
            let title = match &entities.topic {
                Some(t) => t.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "What should I remind you about?".to_string(),
                    }
                }
            };

            let due = match &entities.date {
                Some(d) => d.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "When is this due? (e.g., tomorrow, next Friday, 2026-02-01)"
                            .to_string(),
                    }
                }
            };

            let priority = entities
                .priority
                .clone()
                .unwrap_or_else(|| "normal".to_string());

            Decision::Route {
                tool: "create_todo".to_string(),
                args: serde_json::json!({
                    "title": title,
                    "due": due,
                    "priority": priority
                }),
            }
        }

        IntentType::Unknown => Decision::Reject {
            reason: "No matching route or follow-up found.".to_string(),
        },
    }
}

// ============================================================================
// Stub Tool Runner
// ============================================================================

fn run_tool(tool: &str, args: &serde_json::Value) -> String {
    match tool {
        "search_notes" => format!("[stub] searched notes for: {}", args),
        "search_files" => format!("[stub] searched files for: {}", args),
        "get_weather" => format!("[stub] weather result for: {}", args),
        "draft_email" => format!("[stub] drafted email with: {}", args),
        "create_todo" => format!("[stub] created todo with: {}", args),
        _ => format!("[stub] unknown tool: {} args={}", tool, args),
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<()> {
    let args = Args::parse();

    // Determine router path based on backend if not specified
    #[allow(unused_variables)]
    let router_path = args.router_path.unwrap_or_else(|| {
        #[cfg(feature = "scryer-backend")]
        {
            PathBuf::from("router_standard.pl")
        }
        #[cfg(not(feature = "scryer-backend"))]
        {
            PathBuf::from("../router.pl")
        }
    });

    // Extract intent (stub or LLM)
    let mut payload = if args.use_llm {
        eprintln!("DEBUG: Using LLM intent extractor");
        match llm::extract_intent_llm(&args.user_text) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("WARNING: LLM extraction failed ({}), using stub extractor", e);
                extract_intent_stub(&args.user_text)
            }
        }
    } else {
        extract_intent_stub(&args.user_text)
    };

    // Overlay CLI-provided entity slots
    if let Some(date) = args.date {
        payload.entities.date = Some(date);
    }
    if let Some(location) = args.location {
        payload.entities.location = Some(location);
    }
    if let Some(recipient) = args.recipient {
        payload.entities.recipient = Some(recipient);
    }
    if let Some(source) = args.source {
        payload.constraints.source_preference = source.into();
    }

    // Resolve relative dates to absolute YYYY-MM-DD
    if let Some(ref date) = payload.entities.date {
        let resolved = resolve_relative_date(date);
        if resolved != *date {
            eprintln!("DEBUG: Resolved date '{}' -> '{}'", date, resolved);
        }
        payload.entities.date = Some(resolved);
    }

    // Handle unknown intent
    if payload.intent == IntentType::Unknown {
        let response = serde_json::json!({
            "type": "need_info",
            "question": "What are you trying to do (summarize, find, weather, draft, remind)?"
        });
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    // Route via Prolog (stub or real)
    let decision = if args.use_stub {
        eprintln!("DEBUG: Using stub Prolog router");
        prolog_decide_stub(&payload)
    } else {
        // Choose backend based on compiled features
        #[cfg(feature = "scryer-backend")]
        {
            eprintln!("DEBUG: Using Scryer Prolog with router: {}", router_path.display());
            match scryer::scryer_decide(&payload, &router_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("WARNING: Scryer error ({}), falling back to stub", e);
                    prolog_decide_stub(&payload)
                }
            }
        }

        #[cfg(all(feature = "swipl-backend", not(feature = "scryer-backend")))]
        {
            eprintln!("DEBUG: Using SWI-Prolog with router: {}", router_path.display());
            match prolog::prolog_decide_via_json(&payload, &router_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("WARNING: Prolog error ({}), falling back to stub", e);
                    prolog_decide_stub(&payload)
                }
            }
        }

        #[cfg(not(any(feature = "swipl-backend", feature = "scryer-backend")))]
        {
            eprintln!("DEBUG: No Prolog backend compiled, using stub router");
            prolog_decide_stub(&payload)
        }
    };

    // Print results
    println!("Intent JSON:");
    println!("{}", serde_json::to_string_pretty(&payload)?);
    println!("\nProlog Decision:");
    println!("{}", serde_json::to_string_pretty(&decision)?);

    // Run tool if routed
    if let Decision::Route { ref tool, ref args } = decision {
        let result = run_tool(tool, args);
        println!("\nTool Result:");
        println!("{}", result);
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stub_extractor_summarize() {
        let payload = extract_intent_stub("summarize my notes about AI");
        assert_eq!(payload.intent, IntentType::Summarize);
        assert_eq!(payload.entities.topic, Some("AI".to_string()));
        assert_eq!(payload.constraints.source_preference, SourcePreference::Notes);
    }

    #[test]
    fn test_stub_extractor_weather() {
        let payload = extract_intent_stub("what's the weather tomorrow");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.date, Some("tomorrow".to_string()));
    }

    #[test]
    fn test_stub_extractor_unknown() {
        let payload = extract_intent_stub("hello world");
        assert_eq!(payload.intent, IntentType::Unknown);
    }

    #[test]
    fn test_stub_extractor_email_with_recipient() {
        let payload = extract_intent_stub("send an email to Mary");
        assert_eq!(payload.intent, IntentType::Draft);
        assert_eq!(payload.entities.recipient, Some("Mary".to_string()));
    }

    #[test]
    fn test_stub_extractor_email_to_pattern() {
        let payload = extract_intent_stub("I want you to send an email to John about the project");
        assert_eq!(payload.intent, IntentType::Draft);
        assert_eq!(payload.entities.recipient, Some("John".to_string()));
        assert_eq!(payload.entities.topic, Some("the project".to_string()));
    }

    #[test]
    fn test_prolog_decide_summarize() {
        let payload = IntentPayload {
            intent: IntentType::Summarize,
            entities: Entities {
                topic: Some("machine learning".to_string()),
                ..Default::default()
            },
            constraints: Constraints {
                source_preference: SourcePreference::Notes,
                ..Default::default()
            },
        };

        let decision = prolog_decide_stub(&payload);
        match decision {
            Decision::Route { tool, args } => {
                assert_eq!(tool, "search_notes");
                assert_eq!(args["query"], "machine learning");
            }
            _ => panic!("Expected Route decision"),
        }
    }

    #[test]
    fn test_prolog_decide_weather_missing_location() {
        let payload = IntentPayload {
            intent: IntentType::Weather,
            entities: Entities {
                date: Some("tomorrow".to_string()),
                ..Default::default()
            },
            constraints: Constraints::default(),
        };

        let decision = prolog_decide_stub(&payload);
        match decision {
            Decision::NeedInfo { question } => {
                assert!(question.contains("location"));
            }
            _ => panic!("Expected NeedInfo decision"),
        }
    }

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

        let json = serde_json::to_string(&payload).unwrap();
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
    fn test_prolog_list_generation() {
        let entities = Entities {
            topic: Some("AI notes".to_string()),
            location: Some("NYC".to_string()),
            ..Default::default()
        };

        let list = entities.to_prolog_list();
        assert!(list.starts_with('['));
        assert!(list.ends_with(']'));
        assert!(list.contains("topic-'AI notes'"));
        assert!(list.contains("location-'NYC'"));
        assert!(!list.contains("date")); // None fields should be skipped
    }

    #[test]
    fn test_constraints_list_generation() {
        let constraints = Constraints {
            source_preference: SourcePreference::Notes,
            safety: "normal".to_string(),
        };

        let list = constraints.to_prolog_list();
        assert!(list.contains("source_preference-notes"));
        assert!(list.contains("safety-'normal'"));
    }

    #[test]
    fn test_resolve_date_today() {
        let resolved = resolve_relative_date("today");
        let expected = Local::now().date_naive().format("%Y-%m-%d").to_string();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_resolve_date_tomorrow() {
        let resolved = resolve_relative_date("tomorrow");
        let expected = (Local::now().date_naive() + Days::new(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_resolve_date_yesterday() {
        let resolved = resolve_relative_date("yesterday");
        let expected = (Local::now().date_naive() - Days::new(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_resolve_date_weekday() {
        // "friday" should resolve to the next Friday
        let resolved = resolve_relative_date("friday");
        assert!(resolved.len() == 10); // YYYY-MM-DD format
        assert!(resolved.starts_with("20")); // Year starts with 20xx
    }

    #[test]
    fn test_resolve_date_next_weekday() {
        let resolved = resolve_relative_date("next monday");
        assert!(resolved.len() == 10);
        // Parse it back and verify it's a Monday
        let parsed = NaiveDate::parse_from_str(&resolved, "%Y-%m-%d").unwrap();
        assert_eq!(parsed.weekday(), Weekday::Mon);
    }

    #[test]
    fn test_resolve_date_passthrough() {
        // Already absolute dates should pass through unchanged
        assert_eq!(resolve_relative_date("2026-01-25"), "2026-01-25");
        assert_eq!(resolve_relative_date("January 15"), "January 15");
    }

    #[test]
    fn test_resolve_date_case_insensitive() {
        let lower = resolve_relative_date("today");
        let upper = resolve_relative_date("TODAY");
        let mixed = resolve_relative_date("ToDay");
        assert_eq!(lower, upper);
        assert_eq!(lower, mixed);
    }
}
