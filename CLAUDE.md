# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A Prolog-based intent router for LLM outputs. The system takes structured intent objects (with intent, entities, and constraints) and uses SWI-Prolog rules to determine which tool to invoke and with what arguments.

## Running the Code

```bash
# Install dependencies (requires pyswip which needs SWI-Prolog installed)
uv sync

# Run the router with hardcoded example
uv run python main.py
```

**Prerequisites:** SWI-Prolog must be installed on the system for pyswip to work.

## Architecture

**main.py** - Python entry point that:
- Loads the Prolog knowledge base via pyswip
- Converts Python dicts to SWI-Prolog dict syntax
- Queries `route/5` for tool routing decisions
- Falls back to `need_info/3` when required entities are missing
- Returns one of: `("route", tool, args)`, `("need_info", question)`, or `("reject", reason)`

**router.pl** - Prolog knowledge base containing:
- `provides/2` - Tool capability declarations (e.g., `provides(search_notes, search(notes))`)
- `forbidden/2` - Safety/policy rules blocking tools in certain contexts
- `route/5` - Main routing rules: `route(Intent, Entities, Constraints, Tool, Args)`
- `need_info/3` - Follow-up questions when entities are missing
- Helper predicates for extracting entities and constraints from dicts

## Intent Object Structure

```python
{
    "intent": "summarize",           # Action type: summarize, find, weather, draft, remind
    "entities": {"topic": "..."},    # Extracted entities (topic, location, date, recipient)
    "constraints": {                 # Optional constraints
        "source_preference": "notes",  # notes, files, either
        "safety": "normal"
    }
}
```

## Extending the Router

Add new routing rules in `router.pl` following the pattern:
```prolog
route(intent_name, Entities, Constraints, tool_name, [arg1(Val1), arg2(Val2)]) :-
    entity(Entities, key, Val1),
    constraint(Constraints, key2, Val2).
```
