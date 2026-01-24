# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A Prolog-based intent router for LLM outputs. The system takes structured intent objects (with intent, entities, and constraints) and uses SWI-Prolog rules to determine which tool to invoke and with what arguments.

## Running the Code

```bash
# Install dependencies (requires pyswip which needs SWI-Prolog installed)
uv sync

# Basic usage with stub intent extractor
uv run python main.py "summarize my notes about AI"

# Use LLM-based intent extraction (requires local LLM server)
uv run python main.py "what's the weather tomorrow in NYC" --use-llm

# Provide optional entities via CLI flags
uv run python main.py "remind me about meeting" --date tomorrow
uv run python main.py "weather forecast" --location "San Francisco" --date today
uv run python main.py "draft an email about project update" --recipient "team@example.com"
uv run python main.py "find documentation" --source notes
```

**Prerequisites:** SWI-Prolog must be installed on the system for pyswip to work.

### LLM Endpoint Configuration

When using `--use-llm`, the code calls a local Responses API endpoint. Configure in `main.py`:
```python
LLM_ENDPOINT = "http://alien.local:8000/v1/responses"
LLM_MODEL = "openai/gpt-oss-20b"
```

## Architecture

**main.py** - Python entry point that:
- Parses CLI arguments (user_text, --date, --location, --recipient, --source, --use-llm)
- Extracts intent via stub heuristics or LLM (with `--use-llm`)
- Loads the Prolog knowledge base via pyswip
- Converts Python dicts to SWI-Prolog dict syntax
- Queries `route/5` for tool routing decisions
- Falls back to `need_info/3` when required entities are missing
- Runs stub tool implementations and prints results

**router.pl** - Prolog knowledge base containing:
- `provides/2` - Tool capability declarations (e.g., `provides(search_notes, search(notes))`)
- `route/5` - Main routing rules: `route(Intent, Entities, Constraints, Tool, Args)`
- `need_info/3` - Follow-up questions when entities are missing
- `must_get/3` - Throws `missing_required(Key)` for required entities
- Helper predicates: `preferred_source/2`, `topic_query/2`, `get_with_default/4`

## Intent Object Structure

```python
{
    "intent": "summarize",           # Action type: summarize, find, weather, draft, remind
    "entities": {"topic": "..."},    # Extracted entities (topic, query, location, date, recipient, priority)
    "constraints": {                 # Optional constraints
        "source_preference": "notes",  # notes, files, either
        "safety": "normal"
    }
}
```

## Extending the Router

Add new routing rules in `router.pl` following the pattern (uses SWI-Prolog dict syntax):
```prolog
route(intent_name, E, C, tool_name, _{arg1:Val1, arg2:Val2}) :-
    must_get(E, required_key, Val1),           % throws if missing
    get_with_default(E, optional_key, "", Val2).
```
