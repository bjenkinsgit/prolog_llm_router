# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A Rust port of the Prolog-based intent router for LLM outputs. Takes user text, extracts structured intent (intent type + entities + constraints), and uses Prolog rules to route to specific tools with arguments. The key innovation is using the same Rust structs for both JSON serialization (serde) and Prolog term conversion via `ToPrologDict` and `ToPrologList` traits.

## Build & Run Commands

```bash
# Build with SWI-Prolog backend (default, requires SWI-Prolog installed)
cargo build --release

# Build with Scryer backend (pure Rust, no external deps)
cargo build --release --no-default-features --features scryer-backend

# Run with stub extractor and stub router
cargo run -- "summarize my notes about AI"

# Run with LLM-based intent extraction
cargo run -- --use-llm "what's the weather tomorrow"

# Run with real Prolog router (SWI-Prolog)
export SWI_HOME_DIR=/Applications/SWI-Prolog.app/Contents/swipl
cargo run -- "summarize my notes about AI"

# Run with tool execution (HTTP endpoints)
cargo run -- --tools tools.json "get weather for NYC"

# Agent mode (multi-turn LLM loop)
cargo run -- --agent "I need weather and a reminder"

# Direct tool execution (bypasses intent routing)
cargo run -- --tool notes_search_by_tag "#mytag"
cargo run -- --tool search_notes "python"
cargo run -- --tool notes_tags ""
cargo run -- --tool list_notes ""

# Verbose output
cargo run -- -v "find notes about python"
```

### Testing

```bash
cargo test                           # All tests (SWI backend)
cargo test --no-default-features --features scryer-backend  # Scryer backend
cargo test test_stub_extractor_weather  # Single test
cargo test -- --nocapture            # With output
```

## Architecture

### Core Flow
1. **CLI Parsing** (`main.rs`) → Parse arguments (user_text, --date, --location, etc.)
2. **Intent Extraction** → Stub heuristics (default) or LLM via HTTP (`--use-llm`)
3. **Date Resolution** → Convert relative dates ("tomorrow") to YYYY-MM-DD
4. **Prolog Routing** → Query `route/5` for tool + args (SWI-Prolog, Scryer, or stub)
5. **Tool Execution** → Run tool (HTTP endpoint or stub implementation)

### Key Modules

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point, CLI, stub extractor, stub router, tests |
| `llm.rs` | LLM intent extraction via HTTP to local server |
| `agent.rs` | Agentic chat loop - LLM decides action, execute, repeat |
| `prolog.rs` | SWI-Prolog FFI wrapper (feature-gated) |
| `scryer.rs` | Scryer Prolog embedding (pure Rust, feature-gated) |
| `tools.rs` | Tool definitions from JSON, HTTP execution |
| `apple_weather.rs` | Apple WeatherKit API with JWT auth |
| `apple_maps.rs` | Apple Maps geocoding API |
| `apple_notes.rs` | AppleScript integration for Notes.app |

### Feature Flags
- `swipl-backend` (default) - FFI to external SWI-Prolog
- `scryer-backend` - Pure Rust Prolog, no external dependencies

### Prolog Backends

**SWI-Prolog** uses dict syntax:
```prolog
route(summarize, _{topic:"AI"}, _{source_preference:"either"}, Tool, Args)
```

**Scryer** uses association lists:
```prolog
route(summarize, [topic-'AI'], [source_preference-either], Tool, Args)
```

Router files: `router_standard.pl` (Scryer compatible), `../router.pl` (SWI-Prolog specific)

## Intent Object Structure

```rust
IntentPayload {
    intent: IntentType,     // Summarize, Find, Draft, Remind, Weather, Unknown
    entities: Entities {
        topic, query, location, date, date_end, recipient, priority, weather_query
    },
    constraints: Constraints {
        source_preference,  // Notes, Files, Either
        safety              // "normal"
    }
}
```

## Configuration

Environment variables (`.env` file):
- `APPLE_TEAM_ID`, `APPLE_SERVICE_ID`, `APPLE_KEY_ID`, `APPLE_PRIVATE_KEY_PATH` - WeatherKit
- `APPLE_MAPS_ID`, `APPLE_MAPS_KEY`, `APPLE_MAPS_KEY_PATH` - Maps geocoding
- `OPENWEATHER_API_KEY` - Fallback weather API
- `TEMPERATURE_UNIT` - F or C

LLM endpoint configured in `llm.rs`: `http://alien.local:8000/v1/responses`

## Key Design Patterns

- **DRY via Traits**: `ToPrologDict` and `ToPrologList` allow same struct to serialize to different Prolog syntaxes
- **Feature Gates**: Compile-time backend selection via Cargo features
- **Fallback Chain**: LLM → Prolog → Stub router, graceful degradation on errors
- **Template Substitution**: Tool URLs use `{{arg}}` placeholders filled at runtime

## AppleScript Integration (macOS)

Scripts in `scripts/` for Notes.app: search, list, get, open, count, tag indexing. Use delimiter-based protocol (RECORD_START/RECORD_END) for reliable parsing.
