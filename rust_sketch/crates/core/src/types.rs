//! Domain models and Prolog conversion traits for the router
//!
//! This module contains the core types used throughout the router, including:
//! - Intent types and payloads
//! - Entity and constraint structures
//! - Prolog conversion traits (ToPrologDict, ToPrologList)
//! - Date resolution utilities

use chrono::{Datelike, Days, Local, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

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
    pub fn as_atom(&self) -> &'static str {
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

/// Routing decision from Prolog or stub router
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

pub fn escape_prolog_string(s: &str) -> String {
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

pub fn escape_prolog_atom(s: &str) -> String {
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
pub fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Parse weekday name to chrono::Weekday
pub fn parse_weekday(s: &str) -> Option<Weekday> {
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
pub fn next_weekday(from: NaiveDate, target: Weekday) -> NaiveDate {
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

    // "next week" -> 7 days starting today
    if trimmed == "next week" {
        let start = today;
        let end = today + Days::new(6);
        return (format_date(start), Some(format_date(end)));
    }

    // "next N days" -> N days starting today
    if let Some(rest) = trimmed.strip_prefix("next ") {
        if let Some(days_str) = rest.strip_suffix(" days") {
            if let Ok(n) = days_str.trim().parse::<u64>() {
                let start = today;
                let end = today + Days::new(n.saturating_sub(1));
                return (format_date(start), Some(format_date(end)));
            }
        }
    }

    // "this weekend" -> Saturday to Sunday
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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_type_atom() {
        assert_eq!(IntentType::Summarize.as_atom(), "summarize");
        assert_eq!(IntentType::Weather.as_atom(), "weather");
        assert_eq!(IntentType::Unknown.as_atom(), "unknown");
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
}
