//! Prolog-based LLM Intent Router Core Library
//!
//! This crate provides the core functionality for the Prolog router:
//! - Intent extraction (stub and LLM-based)
//! - Prolog routing (Scryer Prolog backend)
//! - Tool execution
//! - Apple integrations (Weather, Maps, Notes)
//! - Conversation memory (memvid-based semantic search)

pub mod types;

pub mod agent;
pub mod apple_maps;
pub mod apple_notes;
pub mod apple_weather;
pub mod intent;
pub mod llm;
pub mod router;
pub mod tools;

// These modules have internal feature gating for memvid-specific functionality
pub mod conversation_memory;
pub mod memvid_notes;

#[cfg(feature = "scryer-backend")]
pub mod scryer;

// Re-export commonly used types at crate root
pub use types::{
    Constraints, Decision, Entities, IntentPayload, IntentType, SourcePreference,
    ToPrologDict, ToPrologList, WeatherQueryType,
    escape_prolog_atom, escape_prolog_string,
    format_date, next_weekday, parse_weekday, resolve_date_range, resolve_relative_date, today_date,
};

// Re-export the stub router and intent extractor
pub use router::prolog_decide_stub;
pub use intent::extract_intent_stub;
