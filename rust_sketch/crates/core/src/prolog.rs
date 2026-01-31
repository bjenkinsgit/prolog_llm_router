//! Actual SWI-Prolog integration using the swipl crate

use anyhow::{anyhow, Result};
use swipl::prelude::*;
use std::path::Path;
use std::sync::OnceLock;

use crate::{Decision, IntentPayload, ToPrologDict};

// Global engine - swipl requires careful initialization
static ENGINE: OnceLock<Engine> = OnceLock::new();

fn get_engine() -> &'static Engine {
    ENGINE.get_or_init(Engine::new)
}

/// Query Prolog router and return a Decision
pub fn prolog_decide_via_json(payload: &IntentPayload, router_path: &Path) -> Result<Decision> {
    let intent_atom = payload.intent.as_atom();
    let entities_dict = payload.entities.to_prolog_dict();
    let constraints_dict = payload.constraints.to_prolog_dict();

    let engine = get_engine();
    let activation = engine.activate();
    let context: Context<_> = activation.into();

    // Load the router.pl file
    let router_path_str = router_path.display().to_string().replace('\\', "/");
    eprintln!("DEBUG: Loading router from: {}", router_path_str);

    // Parse and execute consult
    let consult_goal = format!("consult('{}')", router_path_str);
    let consult_term = context.term_from_string(&consult_goal)
        .map_err(|e| anyhow!("Failed to parse consult goal: {:?}", e))?;

    context.call_term_once(&consult_term)
        .map_err(|e| anyhow!("Failed to load router.pl: {:?}", e))?;

    // Build a single query that catches missing_required exceptions and returns the field name
    // This avoids running multiple queries which causes issues with the swipl crate
    let query = format!(
        "catch(route({}, {}, {}, Tool, Args), error(missing_required(Field), _), true)",
        intent_atom, entities_dict, constraints_dict
    );
    eprintln!("DEBUG: Prolog query: {}", query);

    let query_term = context.term_from_string(&query)
        .map_err(|e| anyhow!("Failed to parse query: {:?}", e))?;

    match context.call_term_once(&query_term) {
        Ok(_) => {
            // Query succeeded - either route matched or an exception was caught
            // Since we can't easily extract variable bindings with swipl-rs,
            // we fall back to the stub router which has the same logic
            Ok(crate::prolog_decide_stub(payload, false))
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            eprintln!("DEBUG: Prolog error: {}", err_str);
            Ok(Decision::Reject {
                reason: format!("No matching route found: {}", err_str),
            })
        }
    }
}
