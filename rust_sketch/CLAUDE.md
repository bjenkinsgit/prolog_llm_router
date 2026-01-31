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

# Agent mode without memory (single query, no persistence)
cargo run -- --agent --no-memory "one-off question"

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
| `memvid_notes.rs` | Semantic Notes search via memvid-rs (feature-gated) |
| `conversation_memory.rs` | Conversation persistence via memvid-rs (feature-gated) |

### Feature Flags
- `swipl-backend` (default) - FFI to external SWI-Prolog
- `scryer-backend` - Pure Rust Prolog, no external dependencies
- `memvid` - Semantic Notes search via memvid-rs (requires FFmpeg)

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

## Semantic Notes Search (memvid feature)

The `memvid` feature enables BERT-based semantic search for Apple Notes using memvid-rs.

### Prerequisites
```bash
# Install FFmpeg 7.x (required by memvid-rs, FFmpeg 8.x is not compatible)
brew install ffmpeg@7

# Use rustup's cargo (not Homebrew's) and set paths
export PATH="$HOME/.cargo/bin:$PATH"
export PKG_CONFIG_PATH="/opt/homebrew/opt/ffmpeg@7/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/ffmpeg@7/lib:$LIBRARY_PATH"
export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/opt/ffmpeg@7/include"

# Build with memvid feature
cargo build --features memvid
```

Note: memvid-rs requires FFmpeg 7.x due to API compatibility. FFmpeg 8.x removed `avfft.h`.

### Tools
```bash
# Set environment (required for runtime)
export DYLD_LIBRARY_PATH="/opt/homebrew/opt/ffmpeg@7/lib:/Applications/SWI-Prolog.app/Contents/Frameworks:$DYLD_LIBRARY_PATH"

# Rebuild the semantic index (encodes all notes)
cargo run --features memvid -- --tool notes_rebuild_index ""

# Semantic search (BERT-based similarity)
cargo run --features memvid -- --tool notes_semantic_search "rust async patterns"

# Check index stats
cargo run --features memvid -- --tool notes_index_stats ""

# Smart search (uses semantic if index exists, otherwise AppleScript)
cargo run --features memvid -- --tool notes_smart_search "machine learning"
```

### Storage
- `~/.cache/prolog-router/apple_notes.mp4` - QR-encoded note content
- `~/.cache/prolog-router/apple_notes_index.db` - SQLite vector index
- `~/.cache/prolog-router/apple_notes_meta.json` - Sync metadata

## Conversation Memory (memvid feature)

The `memvid` feature also enables conversation memory persistence. The agent stores user queries and responses in a searchable memvid index, enabling context retrieval from past conversations.

### How it Works

1. **Session Start**: Agent loads conversation memory and searches for relevant past context
2. **Context Injection**: Retrieved past conversations are injected into the system prompt
3. **Session End**: User query and final answer are stored for future retrieval

### Tools
```bash
# Search conversation memory
cargo run --features memvid -- --tool memory_search "rust programming"

# Show memory statistics
cargo run --features memvid -- --tool memory_stats ""
```

### Configuration

In `memvid_config.toml`:
```toml
[memory]
enabled = true                # Enable/disable memory
max_context_results = 3       # Number of past conversations to retrieve
session_timeout_hours = 24    # Session grouping window

[ffmpeg]
# FFmpeg library log level (controls swscaler warnings)
# Options: quiet, panic, fatal, error, warning, info, verbose, debug, trace
library_log_level = "error"

# FFmpeg CLI log level (for concat operations)
cli_log_level = "error"

# Hide FFmpeg CLI banner
hide_banner = true

# x265 encoder log level
# Options: none, error, warning, info, debug, full
x265_log_level = "error"
```

### Storage
- `~/.cache/prolog-router/conversation_memory.mp4` - QR-encoded conversation history
- `~/.cache/prolog-router/conversation_memory_index.db` - BERT embeddings for semantic search
- `~/.cache/prolog-router/conversation_memory_meta.json` - Metadata (exchange count, last update)
