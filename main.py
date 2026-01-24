import argparse
import json
from enum import Enum
from typing import Any, Dict, Optional, cast

from openai import OpenAI
from pydantic import BaseModel, Field
from pyswip import Prolog
from pyswip.prolog import PrologError


# ---- Pydantic Models ----
class IntentType(str, Enum):
    SUMMARIZE = "summarize"
    FIND = "find"
    DRAFT = "draft"
    REMIND = "remind"
    WEATHER = "weather"
    UNKNOWN = "unknown"


class SourcePreference(str, Enum):
    NOTES = "notes"
    FILES = "files"
    EITHER = "either"


class Entities(BaseModel):
    """Extracted entities from user input."""

    topic: Optional[str] = Field(default=None, description="Main subject or topic")
    query: Optional[str] = Field(default=None, description="Search query string")
    location: Optional[str] = Field(default=None, description="Location for weather")
    date: Optional[str] = Field(
        default=None, description="Date (e.g., today, tomorrow, 2026-02-10)"
    )
    recipient: Optional[str] = Field(
        default=None, description="Email recipient address"
    )
    priority: Optional[str] = Field(
        default=None, description="Priority level (low, normal, high)"
    )


class Constraints(BaseModel):
    """Routing constraints."""

    source_preference: SourcePreference = Field(
        default=SourcePreference.EITHER,
        description="Preferred source: notes, files, or either",
    )
    safety: str = Field(default="normal", description="Safety level")


class IntentPayload(BaseModel):
    """Structured intent extracted from user input."""

    intent: IntentType = Field(
        description="The user's intent: summarize, find, draft, remind, weather, or unknown"
    )
    entities: Entities = Field(
        default_factory=Entities, description="Extracted entities"
    )
    constraints: Constraints = Field(
        default_factory=Constraints, description="Routing constraints"
    )


def normalize(obj):
    """
    Recursively convert bytes (from PySwip/SWI) into UTF-8 strings
    so json.dumps works.
    """
    if isinstance(obj, bytes):
        return obj.decode("utf-8", errors="replace")
    if isinstance(obj, dict):
        return {normalize(k): normalize(v) for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        return [normalize(x) for x in obj]
    return obj


def to_swi_dict(d: dict) -> str:
    """
    Convert a flat Python dict into a SWI-Prolog dict term string: _{k:"v", n:123}
    Keeps it simple for this project: strings, numbers, booleans.
    """
    parts = []
    for k, v in d.items():
        if isinstance(v, bool):
            v_str = "true" if v else "false"
        elif isinstance(v, (int, float)):
            v_str = str(v)
        else:
            # escape backslashes and quotes for Prolog double-quoted strings
            s = str(v).replace("\\", "\\\\").replace('"', '\\"')
            v_str = f'"{s}"'
        parts.append(f"{k}:{v_str}")
    return "_{" + ", ".join(parts) + "}"


def entities_to_dict(entities: Entities) -> dict:
    """Convert Entities model to dict, excluding None values."""
    return {k: v for k, v in entities.model_dump().items() if v is not None}


def constraints_to_dict(constraints: Constraints) -> dict:
    """Convert Constraints model to dict with enum values as strings."""
    return {
        "source_preference": constraints.source_preference.value,
        "safety": constraints.safety,
    }


def prolog_decide(intent: IntentType, entities: Entities, constraints: Constraints):
    prolog = Prolog()
    prolog.consult("router.pl")

    e_term = to_swi_dict(entities_to_dict(entities))
    c_term = to_swi_dict(constraints_to_dict(constraints))

    try:
        q_route = f"route({intent.value}, {e_term}, {c_term}, Tool, Args)"
        route_results = list(prolog.query(q_route, maxresult=1))
        if route_results:
            return {
                "type": "route",
                "tool": route_results[0]["Tool"],
                "args": route_results[0]["Args"],
            }
    except PrologError as e:
        msg = str(e)
        # Map common missing-required errors to need_info
        if "missing_required(date)" in msg:
            return {
                "type": "need_info",
                "question": "When is this due? (e.g., tomorrow, next Friday, 2026-02-01)",
            }
        if "missing_required(location)" in msg:
            return {"type": "need_info", "question": "What location should I use?"}
        if "missing_required(recipient)" in msg:
            return {"type": "need_info", "question": "Who should I email?"}
        # Otherwise fall through to generic reject

    q_need = f"need_info({intent.value}, {e_term}, Q)"
    need_results = list(prolog.query(q_need, maxresult=1))
    if need_results:
        return {"type": "need_info", "question": need_results[0]["Q"]}

    return {"type": "reject", "reason": "No matching route or follow-up found."}


# ---- Stub "LLM intent extractor" (replace later) ----
def extract_intent_stub(user_text: str) -> IntentPayload:
    """
    Simple heuristic so you can test routing now.
    Replace this with an actual LLM call later.
    """
    t = user_text.lower().strip()

    source_pref = SourcePreference.EITHER
    if "notes" in t:
        source_pref = SourcePreference.NOTES
    if "files" in t:
        source_pref = SourcePreference.FILES

    # very naive intent detection
    if t.startswith("summarize") or "summarize" in t:
        intent = IntentType.SUMMARIZE
    elif t.startswith("find") or t.startswith("search") or "look for" in t:
        intent = IntentType.FIND
    elif "weather" in t:
        intent = IntentType.WEATHER
    elif "email" in t or t.startswith("draft"):
        intent = IntentType.DRAFT
    elif "remind" in t or "todo" in t:
        intent = IntentType.REMIND
    else:
        intent = IntentType.UNKNOWN

    # naive topic extraction: everything after "about"
    topic = None
    if "about" in t:
        topic = user_text.split("about", 1)[1].strip()
    else:
        # or after the first verb
        pieces = user_text.split(" ", 1)
        if len(pieces) == 2 and intent in (IntentType.SUMMARIZE, IntentType.FIND):
            topic = pieces[1].strip()

    # weather location/date (toy)
    date = None
    if intent == IntentType.WEATHER:
        if "tomorrow" in t:
            date = "tomorrow"
        elif "today" in t:
            date = "today"

    return IntentPayload(
        intent=intent,
        entities=Entities(topic=topic, date=date),
        constraints=Constraints(source_preference=source_pref),
    )


# ---- Tool runner stubs ----
def run_tool(tool: str, args: Dict[str, Any]) -> str:
    # For now: just print what would run
    if tool == "search_notes":
        return f"[stub] searched notes for: {args}"
    if tool == "search_files":
        return f"[stub] searched files for: {args}"
    if tool == "get_weather":
        return f"[stub] weather result for: {args}"
    if tool == "draft_email":
        return f"[stub] drafted email with: {args}"
    if tool == "create_todo":
        return f"[stub] created todo with: {args}"
    return f"[stub] unknown tool: {tool} args={args}"


LLM_BASE_URL = "http://alien.local:8000/v1"
LLM_MODEL = "openai/gpt-oss-20b"

SYSTEM_PROMPT = """You are an intent extractor for a tool-routing system.
Extract the user's intent and relevant entities from their message.

Rules:
- Choose intent from: summarize, find, draft, remind, weather, unknown
- Put the main subject into entities.topic when relevant
- If user explicitly mentions notes/files, set source_preference accordingly; otherwise 'either'
- If missing critical info (e.g. weather without location/date), leave those entities as null
- Extract dates in natural language form (e.g., "tomorrow", "next Friday", "2026-02-10")
"""


def extract_intent_llm(user_text: str) -> IntentPayload:
    """
    Call local LLM using OpenAI client's responses.parse API for structured output.
    Returns a validated IntentPayload.
    """
    client = OpenAI(base_url=LLM_BASE_URL, api_key="not-needed")

    response = client.responses.parse(
        model=LLM_MODEL,
        instructions=SYSTEM_PROMPT,
        input=user_text,
        text_format=IntentPayload,
    )

    return response.output_parsed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("user_text", help="User request in natural language")
    parser.add_argument(
        "--date", help="Optional date (e.g., today, tomorrow, 2026-02-10)"
    )
    parser.add_argument("--location", help="Optional location for weather")
    parser.add_argument("--recipient", help="Optional recipient for draft email")
    parser.add_argument(
        "--source",
        choices=["notes", "files", "either"],
        help="Optional source preference",
    )
    parser.add_argument(
        "--use-llm", action="store_true", help="Use the local LLM intent extractor"
    )

    args = parser.parse_args()

    user_text = args.user_text
    intent_payload: IntentPayload = (
        extract_intent_llm(user_text)
        if args.use_llm
        else extract_intent_stub(user_text)
    )

    # Overlay CLI-provided entity slots by creating updated models
    entities_updates = {}
    if args.date:
        entities_updates["date"] = args.date
    if args.location:
        entities_updates["location"] = args.location
    if args.recipient:
        entities_updates["recipient"] = args.recipient

    if entities_updates:
        intent_payload = intent_payload.model_copy(
            update={"entities": intent_payload.entities.model_copy(update=entities_updates)}
        )

    if args.source:
        new_constraints = intent_payload.constraints.model_copy(
            update={"source_preference": SourcePreference(args.source)}
        )
        intent_payload = intent_payload.model_copy(update={"constraints": new_constraints})

    if intent_payload.intent == IntentType.UNKNOWN:
        print(
            json.dumps(
                {
                    "type": "need_info",
                    "question": "What are you trying to do (summarize, find, weather, draft, remind)?",
                },
                indent=2,
            )
        )
        return

    decision: Dict[str, Any] = prolog_decide(
        intent_payload.intent, intent_payload.entities, intent_payload.constraints
    )

    print("Intent JSON:")
    print(intent_payload.model_dump_json(indent=2, exclude_none=True))
    print("\nProlog Decision:")
    print(json.dumps(normalize(decision), indent=2))

    if decision["type"] == "route":
        tool = cast(str, normalize(decision["tool"]))
        args2 = cast(Dict[str, Any], normalize(decision["args"]))
        result = run_tool(tool, args2)
        print("\nTool Result:")
        print(result)


if __name__ == "__main__":
    main()
