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

mod agent;
mod apple_maps;
mod apple_notes;
mod apple_weather;
mod conversation_memory;
mod derive_sketch;
mod llm;
mod memvid_notes;
mod tools;

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
    /// User request in natural language (or JSON args when using --tool)
    user_text: String,

    /// Execute a tool directly by name (bypasses intent routing)
    #[arg(long = "tool")]
    tool_name: Option<String>,

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

    /// Path to tools configuration JSON file for HTTP tool execution
    #[arg(long = "tools")]
    tools_path: Option<PathBuf>,

    /// Use agentic mode (LLM decides actions in a loop)
    #[arg(long = "agent")]
    agent_mode: bool,

    /// Maximum turns for agent mode before stopping
    #[arg(long = "max-turns", default_value = "10")]
    max_turns: u32,

    /// Disable conversation memory for this query (agent mode only)
    #[arg(long = "no-memory")]
    no_memory: bool,

    /// Enable verbose debug output
    #[arg(long, short = 'v')]
    verbose: bool,
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

/// Weather query type for different kinds of weather requests
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherQueryType {
    Current,
    Forecast,
    Assessment,
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

    /// End date for date ranges (e.g., "next week" has date and date_end)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_end: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,

    /// Type of weather query: current, forecast, or assessment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weather_query: Option<WeatherQueryType>,
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
        if let Some(ref v) = self.date_end {
            parts.push(format!("date_end:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.recipient {
            parts.push(format!("recipient:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.priority {
            parts.push(format!("priority:\"{}\"", escape_prolog_string(v)));
        }
        if let Some(ref v) = self.weather_query {
            let s = match v {
                WeatherQueryType::Current => "current",
                WeatherQueryType::Forecast => "forecast",
                WeatherQueryType::Assessment => "assessment",
            };
            parts.push(format!("weather_query:\"{}\"", s));
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
        if let Some(ref v) = self.date_end {
            parts.push(format!("date_end-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.recipient {
            parts.push(format!("recipient-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.priority {
            parts.push(format!("priority-'{}'", escape_prolog_atom(v)));
        }
        if let Some(ref v) = self.weather_query {
            let s = match v {
                WeatherQueryType::Current => "current",
                WeatherQueryType::Forecast => "forecast",
                WeatherQueryType::Assessment => "assessment",
            };
            parts.push(format!("weather_query-{}", s));
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

/// Resolve date range strings to (start_date, Option<end_date>) in YYYY-MM-DD format
/// Returns (start, None) for single dates, (start, Some(end)) for ranges
pub fn resolve_date_range(date_str: &str) -> (String, Option<String>) {
    let today = Local::now().date_naive();
    let lower = date_str.to_lowercase();
    let trimmed = lower.trim();

    // "next week" → 7 days starting today
    if trimmed == "next week" {
        let start = today;
        let end = today + Days::new(6);
        return (format_date(start), Some(format_date(end)));
    }

    // "next N days" → N days starting today
    if let Some(rest) = trimmed.strip_prefix("next ") {
        if let Some(days_str) = rest.strip_suffix(" days") {
            if let Ok(n) = days_str.trim().parse::<u64>() {
                let start = today;
                let end = today + Days::new(n.saturating_sub(1));
                return (format_date(start), Some(format_date(end)));
            }
        }
    }

    // "this weekend" → Saturday to Sunday
    if trimmed == "this weekend" {
        let current_weekday = today.weekday();
        let days_until_saturday = (Weekday::Sat.num_days_from_monday() as i64
            - current_weekday.num_days_from_monday() as i64
            + 7)
            % 7;
        let days_until_saturday = if days_until_saturday == 0 && current_weekday == Weekday::Sat {
            0 // Already Saturday
        } else if days_until_saturday == 0 {
            7 // Next Saturday
        } else {
            days_until_saturday as u64
        };

        let saturday = today + Days::new(days_until_saturday);
        let sunday = saturday + Days::new(1);
        return (format_date(saturday), Some(format_date(sunday)));
    }

    // "forecast" keyword without specific range implies multi-day
    if trimmed.contains("forecast") {
        // Default to 7-day forecast
        let start = today;
        let end = today + Days::new(6);
        return (format_date(start), Some(format_date(end)));
    }

    // Single date - use existing resolver
    (resolve_relative_date(date_str), None)
}

// ============================================================================
// Stub Intent Extractor (heuristic-based, like Python version)
// ============================================================================

/// Extract location from weather queries using "in <city>" or "for <city>" patterns
fn extract_weather_location(lower_text: &str, original_text: &str) -> Option<String> {
    // Try "in <city>" pattern
    for pattern in &[" in ", " for "] {
        if let Some(idx) = lower_text.find(pattern) {
            let start = idx + pattern.len();
            let after = &original_text[start..];
            let after_lower = after.to_lowercase();

            // Take words until we hit a date keyword (with word boundary) or end
            // Use " keyword" patterns to ensure word boundaries
            let end_markers = [
                " today",
                " tomorrow",
                " next",
                " this",
                "?",
                "!",
                ".",
            ];
            let mut end_idx = after.len();
            for marker in end_markers {
                if let Some(m_idx) = after_lower.find(marker) {
                    if m_idx < end_idx {
                        end_idx = m_idx;
                    }
                }
            }
            let city = after[..end_idx].trim();
            if !city.is_empty() {
                return Some(city.to_string());
            }
        }
    }
    None
}

/// Extract "next N days" pattern and return the matched string
fn extract_next_n_days(lower_text: &str) -> Option<String> {
    if let Some(idx) = lower_text.find("next ") {
        let after_next = &lower_text[idx + 5..];
        // Look for "<number> days" pattern
        let words: Vec<&str> = after_next.split_whitespace().collect();
        if words.len() >= 2 && words[1].starts_with("day") {
            if words[0].parse::<u64>().is_ok() {
                return Some(format!("next {} days", words[0]));
            }
        }
    }
    None
}

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
    } else if t.contains("weather")
        || t.starts_with("forecast")
        || t.contains("forecast")
        || t.contains("will it rain")
        || t.contains("will it snow")
        || t.contains("bad weather")
    {
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

    // Weather-specific extraction
    let (date, date_end, location, weather_query) = if intent == IntentType::Weather {
        // Extract location from "in <city>" or "for <city>" patterns
        let location = extract_weather_location(&t, user_text);

        // Detect weather query type
        let weather_query = if t.contains("bad weather")
            || t.contains("expecting")
            || t.contains("will it rain")
            || t.contains("will it snow")
            || t.contains("expect rain")
            || t.contains("expect snow")
        {
            Some(WeatherQueryType::Assessment)
        } else if t.contains("forecast") || t.contains("next week") || t.contains("next ") && t.contains(" days") {
            Some(WeatherQueryType::Forecast)
        } else {
            Some(WeatherQueryType::Current)
        };

        // Extract date/date range
        let (date, date_end) = if t.contains("next week") {
            let (start, end) = resolve_date_range("next week");
            (Some(start), end)
        } else if t.contains("this weekend") {
            let (start, end) = resolve_date_range("this weekend");
            (Some(start), end)
        } else if let Some(cap) = extract_next_n_days(&t) {
            let (start, end) = resolve_date_range(&cap);
            (Some(start), end)
        } else if t.contains("forecast") && !t.contains("tomorrow") && !t.contains("today") {
            // Generic "forecast" without specific date → default to 7 days
            let (start, end) = resolve_date_range("forecast");
            (Some(start), end)
        } else if t.contains("tomorrow") {
            (Some(resolve_relative_date("tomorrow")), None)
        } else if t.contains("today") {
            (Some(resolve_relative_date("today")), None)
        } else {
            // Default to today for current weather queries
            (None, None)
        };

        (date, date_end, location, weather_query)
    } else {
        (None, None, None, None)
    };

    // For non-weather intents, date extraction (toy)
    let date = date.or_else(|| {
        if intent != IntentType::Weather {
            if t.contains("tomorrow") {
                Some("tomorrow".to_string())
            } else if t.contains("today") {
                Some("today".to_string())
            } else {
                None
            }
        } else {
            None
        }
    });

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
            date_end,
            location,
            recipient,
            weather_query,
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
pub fn prolog_decide_stub(payload: &IntentPayload, verbose: bool) -> Decision {
    let intent = &payload.intent;
    let entities = &payload.entities;
    let constraints = &payload.constraints;

    // Print the query that would be sent to Prolog
    if verbose {
        let query = format!(
            "route({}, {}, {}, Tool, Args)",
            intent.as_atom(),
            entities.to_prolog_dict(),
            constraints.to_prolog_dict()
        );
        eprintln!("DEBUG: Prolog query: {}", query);
    }

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

            // Date is now optional - defaults to today for current weather
            let date = entities.date.clone();

            // Get date_end and weather_query for forecasts/assessments
            let date_end = entities.date_end.clone();
            let weather_query = entities.weather_query.as_ref().map(|q| match q {
                WeatherQueryType::Current => "current",
                WeatherQueryType::Forecast => "forecast",
                WeatherQueryType::Assessment => "assessment",
            });

            // Prefer Apple WeatherKit if configured, otherwise use OpenWeatherMap
            let tool = if apple_weather::is_configured() {
                "get_apple_weather"
            } else {
                "get_weather"
            };

            let mut args = serde_json::json!({
                "location": location
            });

            // Add optional fields only if present
            if let Some(d) = date {
                args["date"] = serde_json::Value::String(d);
            }
            if let Some(de) = date_end {
                args["date_end"] = serde_json::Value::String(de);
            }
            if let Some(wq) = weather_query {
                args["weather_query"] = serde_json::Value::String(wq.to_string());
            }

            Decision::Route {
                tool: tool.to_string(),
                args,
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
// Tool Runner (with optional HTTP execution)
// ============================================================================

fn run_tool_stub(tool: &str, args: &serde_json::Value) -> String {
    match tool {
        "search_notes" => format!("[stub] searched notes for: {}", args),
        "search_files" => format!("[stub] searched files for: {}", args),
        "get_weather" => format!("[stub] weather result for: {}", args),
        "get_apple_weather" => format!("[stub] apple weather result for: {}", args),
        "draft_email" => format!("[stub] drafted email with: {}", args),
        "create_todo" => format!("[stub] created todo with: {}", args),
        _ => format!("[stub] unknown tool: {} args={}", tool, args),
    }
}

fn run_tool(
    tool: &str,
    args: &serde_json::Value,
    executor: Option<&tools::ToolExecutor>,
) -> String {
    // Special handling for Apple WeatherKit (requires JWT auth)
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
                Ok(result) => return result,
                Err(e) => {
                    eprintln!("WARNING: Apple Weather failed: {}", e);
                    // Fall through to try OpenWeather or stub
                }
            }
        } else {
            eprintln!("DEBUG: Apple WeatherKit not configured, trying fallback");
        }
    }

    // Try to execute via configured endpoint
    if let Some(exec) = executor {
        if exec.has_endpoint(tool) {
            match exec.execute(tool, args) {
                Ok(Some(result)) => return result,
                Ok(None) => {
                    // No endpoint configured, fall through to stub
                }
                Err(e) => {
                    eprintln!("WARNING: Tool execution failed: {}", e);
                    return format!("[error] {}: {}", tool, e);
                }
            }
        }
    }

    // Fall back to stub
    run_tool_stub(tool, args)
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<()> {
    // Load environment variables from .env file (if present)
    dotenvy::dotenv().ok();

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

    // Load tool executor if config provided
    let tool_executor = if let Some(ref tools_path) = args.tools_path {
        if args.verbose {
            eprintln!("DEBUG: Loading tools config from: {}", tools_path.display());
        }
        match tools::ToolExecutor::load(tools_path) {
            Ok(exec) => {
                if args.verbose {
                    eprintln!("DEBUG: Loaded {} tool(s)", exec.all_tools().count());
                }
                Some(exec)
            }
            Err(e) => {
                eprintln!("WARNING: Failed to load tools config: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Direct tool execution mode: bypass intent routing entirely
    if let Some(ref tool_name) = args.tool_name {
        if args.verbose {
            eprintln!("DEBUG: Direct tool execution: {}", tool_name);
        }

        // Parse user_text as JSON args, or build args from CLI flags
        let tool_args: serde_json::Value = if args.user_text.trim().starts_with('{') {
            // User provided JSON directly
            serde_json::from_str(&args.user_text).unwrap_or_else(|e| {
                eprintln!("WARNING: Invalid JSON args: {}, using empty object", e);
                serde_json::json!({})
            })
        } else {
            // Build args from user_text and CLI flags
            let mut obj = serde_json::Map::new();

            // Use user_text as the primary argument (query, tag, id, etc.)
            let text = args.user_text.trim();
            if !text.is_empty() {
                // Determine the arg name based on tool
                let arg_name = match tool_name.as_str() {
                    "search_notes" | "memory_search" => "query",
                    "notes_search_by_tag" => "tag",
                    "get_note" | "open_note" => "id",
                    "list_notes" => "folder",
                    "notes_index" => "action",
                    _ => "query", // default
                };
                obj.insert(arg_name.to_string(), serde_json::Value::String(text.to_string()));
            }

            // Also add CLI flags if provided
            if let Some(ref date) = args.date {
                obj.insert("date".to_string(), serde_json::Value::String(date.clone()));
            }
            if let Some(ref location) = args.location {
                obj.insert("location".to_string(), serde_json::Value::String(location.clone()));
            }

            serde_json::Value::Object(obj)
        };

        if args.verbose {
            eprintln!("DEBUG: Tool args: {}", tool_args);
        }

        let (success, result) = agent::execute_tool(tool_name, &tool_args, tool_executor.as_ref());
        if !success {
            eprintln!("Tool execution failed");
        }
        println!("{}", result);
        return Ok(());
    }

    // Agent mode: run agentic loop instead of single-shot
    if args.agent_mode {
        if args.verbose {
            eprintln!("DEBUG: Running in agent mode with max_turns={}", args.max_turns);
            if args.no_memory {
                eprintln!("DEBUG: Conversation memory disabled for this query");
            }
        }
        let config = agent::AgentConfig {
            max_turns: args.max_turns,
            verbose: args.verbose,
            use_memory: !args.no_memory,
        };
        let answer = agent::run_agent_loop(&args.user_text, &config, tool_executor.as_ref())?;
        println!("{}", answer);
        return Ok(());
    }

    // Single-shot mode: extract intent (stub or LLM)
    let mut payload = if args.use_llm {
        if args.verbose {
            eprintln!("DEBUG: Using LLM intent extractor");
        }
        match llm::extract_intent_llm(&args.user_text, args.verbose) {
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
        if resolved != *date && args.verbose {
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
        if args.verbose {
            eprintln!("DEBUG: Using stub Prolog router");
        }
        prolog_decide_stub(&payload, args.verbose)
    } else {
        // Choose backend based on compiled features
        #[cfg(feature = "scryer-backend")]
        {
            if args.verbose {
                eprintln!("DEBUG: Using Scryer Prolog with router: {}", router_path.display());
            }
            match scryer::scryer_decide(&payload, &router_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("WARNING: Scryer error ({}), falling back to stub", e);
                    prolog_decide_stub(&payload, args.verbose)
                }
            }
        }

        #[cfg(all(feature = "swipl-backend", not(feature = "scryer-backend")))]
        {
            if args.verbose {
                eprintln!("DEBUG: Using SWI-Prolog with router: {}", router_path.display());
            }
            match prolog::prolog_decide_via_json(&payload, &router_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("WARNING: Prolog error ({}), falling back to stub", e);
                    prolog_decide_stub(&payload, args.verbose)
                }
            }
        }

        #[cfg(not(any(feature = "swipl-backend", feature = "scryer-backend")))]
        {
            if args.verbose {
                eprintln!("DEBUG: No Prolog backend compiled, using stub router");
            }
            prolog_decide_stub(&payload, args.verbose)
        }
    };

    // Print results
    println!("Intent JSON:");
    println!("{}", serde_json::to_string_pretty(&payload)?);
    println!("\nProlog Decision:");
    println!("{}", serde_json::to_string_pretty(&decision)?);

    // Run tool if routed
    if let Decision::Route { ref tool, ref args } = decision {
        let result = run_tool(tool, args, tool_executor.as_ref());
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
        // Date is now resolved to absolute format
        let expected = (Local::now().date_naive() + Days::new(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(payload.entities.date, Some(expected));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Current));
    }

    #[test]
    fn test_stub_extractor_weather_with_location() {
        let payload = extract_intent_stub("what's the weather in London tomorrow");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("London".to_string()));
        assert!(payload.entities.date.is_some());
    }

    #[test]
    fn test_stub_extractor_weather_forecast() {
        let payload = extract_intent_stub("forecast for Seattle next week");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("Seattle".to_string()));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Forecast));
        // Should have both date and date_end for "next week"
        assert!(payload.entities.date.is_some());
        assert!(payload.entities.date_end.is_some());
    }

    #[test]
    fn test_stub_extractor_weather_assessment() {
        let payload = extract_intent_stub("bad weather in Chicago tomorrow");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("Chicago".to_string()));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Assessment));
    }

    #[test]
    fn test_stub_extractor_will_it_rain() {
        let payload = extract_intent_stub("will it rain in Boston this weekend");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("Boston".to_string()));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Assessment));
        // "this weekend" should produce a date range
        assert!(payload.entities.date.is_some());
        assert!(payload.entities.date_end.is_some());
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

        let decision = prolog_decide_stub(&payload, false);
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

        let decision = prolog_decide_stub(&payload, false);
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

    #[test]
    fn test_resolve_date_range_next_week() {
        let (start, end) = resolve_date_range("next week");
        let today = Local::now().date_naive();
        let expected_start = format_date(today);
        let expected_end = format_date(today + Days::new(6));
        assert_eq!(start, expected_start);
        assert_eq!(end, Some(expected_end));
    }

    #[test]
    fn test_resolve_date_range_next_n_days() {
        let (start, end) = resolve_date_range("next 5 days");
        let today = Local::now().date_naive();
        let expected_start = format_date(today);
        let expected_end = format_date(today + Days::new(4));
        assert_eq!(start, expected_start);
        assert_eq!(end, Some(expected_end));
    }

    #[test]
    fn test_resolve_date_range_this_weekend() {
        let (start, end) = resolve_date_range("this weekend");
        // Should return Saturday and Sunday
        assert!(start.len() == 10); // YYYY-MM-DD
        assert!(end.is_some());
        let end = end.unwrap();
        assert!(end.len() == 10);
        // End should be one day after start
        let start_date = NaiveDate::parse_from_str(&start, "%Y-%m-%d").unwrap();
        let end_date = NaiveDate::parse_from_str(&end, "%Y-%m-%d").unwrap();
        assert_eq!(start_date.weekday(), Weekday::Sat);
        assert_eq!(end_date.weekday(), Weekday::Sun);
    }

    #[test]
    fn test_resolve_date_range_single_date() {
        let (start, end) = resolve_date_range("tomorrow");
        let expected = resolve_relative_date("tomorrow");
        assert_eq!(start, expected);
        assert_eq!(end, None);
    }

    #[test]
    fn test_weather_query_type_serialization() {
        let entities = Entities {
            location: Some("NYC".to_string()),
            weather_query: Some(WeatherQueryType::Forecast),
            ..Default::default()
        };
        let json = serde_json::to_string(&entities).unwrap();
        assert!(json.contains("\"weather_query\":\"forecast\""));
    }

    #[test]
    fn test_prolog_dict_with_weather_query() {
        let entities = Entities {
            location: Some("NYC".to_string()),
            date: Some("2026-01-27".to_string()),
            date_end: Some("2026-02-02".to_string()),
            weather_query: Some(WeatherQueryType::Forecast),
            ..Default::default()
        };
        let dict = entities.to_prolog_dict();
        assert!(dict.contains("location:\"NYC\""));
        assert!(dict.contains("date:\"2026-01-27\""));
        assert!(dict.contains("date_end:\"2026-02-02\""));
        assert!(dict.contains("weather_query:\"forecast\""));
    }
}
