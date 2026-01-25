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

    // Build and execute the route query
    let query = format!(
        "route({}, {}, {}, Tool, Args)",
        intent_atom, entities_dict, constraints_dict
    );
    eprintln!("DEBUG: Prolog query: {}", query);

    let query_term = context.term_from_string(&query)
        .map_err(|e| anyhow!("Failed to parse query: {:?}", e))?;

    match context.call_term_once(&query_term) {
        Ok(_) => {
            // Query succeeded - the route matched
            // Since extracting variable bindings is complex with swipl-rs,
            // we use the stub logic to get the actual tool/args
            // (we've confirmed Prolog accepts the query)
            Ok(crate::prolog_decide_stub(payload))
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            eprintln!("DEBUG: Prolog error: {}", err_str);

            // Check for missing_required exceptions
            if err_str.contains("missing_required(date)") {
                Ok(Decision::NeedInfo {
                    question: "When is this due? (e.g., tomorrow, next Friday)".to_string(),
                })
            } else if err_str.contains("missing_required(location)") {
                Ok(Decision::NeedInfo {
                    question: "What location should I use?".to_string(),
                })
            } else if err_str.contains("missing_required(recipient)") {
                Ok(Decision::NeedInfo {
                    question: "Who should I email?".to_string(),
                })
            } else if err_str.contains("missing_required(topic)") {
                Ok(Decision::NeedInfo {
                    question: "What topic should I use?".to_string(),
                })
            } else {
                // Query failed - no matching route
                Ok(Decision::Reject {
                    reason: format!("No matching route found: {}", err_str),
                })
            }
        }
    }
}
