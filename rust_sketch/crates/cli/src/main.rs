//! Prolog-based LLM Intent Router CLI
//!
//! A thin wrapper around prolog-router-core that provides the command-line interface.

use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

use prolog_router_core::{
    agent, apple_weather, extract_intent_stub, llm, prolog_decide_stub, tools,
    Decision, IntentType, SourcePreference, resolve_relative_date,
};

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
// Tool Running
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
            match prolog_router_core::scryer::scryer_decide(&payload, &router_path) {
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
            match prolog_router_core::prolog::prolog_decide_via_json(&payload, &router_path) {
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
    use chrono::{Datelike, Days, Local, NaiveDate, Weekday};
    use prolog_router_core::types::{
        Constraints, Entities, IntentPayload, WeatherQueryType,
        format_date, resolve_date_range,
    };

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
        use prolog_router_core::ToPrologDict;
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
    fn test_prolog_dict_generation() {
        use prolog_router_core::ToPrologDict;
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
        use prolog_router_core::ToPrologList;
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
        use prolog_router_core::ToPrologList;
        let constraints = Constraints {
            source_preference: SourcePreference::Notes,
            safety: "normal".to_string(),
        };

        let list = constraints.to_prolog_list();
        assert!(list.contains("source_preference-notes"));
        assert!(list.contains("safety-'normal'"));
    }
}
