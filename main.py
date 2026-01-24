import json
from pyswip import Prolog

def prolog_route(intent_obj):
    prolog = Prolog()
    prolog.consult("router.pl")

    intent = intent_obj.get("intent", "unknown")
    entities = intent_obj.get("entities", {})
    constraints = intent_obj.get("constraints", {"source_preference": "either", "safety": "normal"})

    # Pass dicts as JSON strings; parse in Prolog via get_dict in PySwip? (We’ll keep it simple)
    # Easiest: represent Entities/Constraints as SWI dict terms using read_term_from_atom, but
    # for a quick MVP, we’ll map only what we need by flattening into Key=Value pairs.

    def dict_to_kv(d):
        def escape(v):
            return str(v).replace("'", "\\'")
        return ",".join([f"{k}:'{escape(v)}'" for k, v in d.items()])

    entities_term = f"_{{{dict_to_kv(entities)}}}"
    constraints_term = f"_{{{dict_to_kv(constraints)}}}"

    q = f"route({intent}, {entities_term}, {constraints_term}, Tool, Args)"
    results = list(prolog.query(q))

    if results:
        return ("route", results[0]["Tool"], results[0]["Args"])

    # try need_info
    q2 = f"need_info({intent}, {entities_term}, Q)"
    results2 = list(prolog.query(q2))
    if results2:
        return ("need_info", results2[0]["Q"])

    return ("reject", "No matching route")

if __name__ == "__main__":
    # TEMP: hardcode an LLM output example
    llm_json = {
        "user_text": "Summarize my notes about printer sharing",
        "intent": "summarize",
        "entities": {"topic": "printer sharing"},
        "constraints": {"source_preference": "notes", "safety": "normal"}
    }

    decision = prolog_route(llm_json)
    print(decision)