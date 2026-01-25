//! Scryer Prolog integration - pure Rust, no external dependencies
//!
//! This module provides an alternative to the swipl crate, using Scryer Prolog
//! which is written in Rust and can be embedded without FFI complexity.

use anyhow::{anyhow, Result};
use scryer_prolog::{LeafAnswer, Machine, MachineBuilder, Term};
use std::path::Path;

use crate::{Decision, IntentPayload, ToPrologList};

/// Query Scryer Prolog router and return a Decision
pub fn scryer_decide(payload: &IntentPayload, router_path: &Path) -> Result<Decision> {
    let intent_atom = payload.intent.as_atom();
    let entities_list = payload.entities.to_prolog_list();
    let constraints_list = payload.constraints.to_prolog_list();

    // Create a fresh machine for each query
    // (Machine is not thread-safe, so we can't use a global static)
    let mut machine = MachineBuilder::default().build();

    // Load the router file
    let router_code = std::fs::read_to_string(router_path)
        .map_err(|e| anyhow!("Failed to read router file: {}", e))?;

    eprintln!("DEBUG: Loading router from: {}", router_path.display());
    // Use "user" as the default module (standard Prolog convention)
    machine.consult_module_string("user", &router_code);

    // Build the query (Scryer may need terminating period)
    let query = format!(
        "route({}, {}, {}, Tool, Args).",
        intent_atom, entities_list, constraints_list
    );
    eprintln!("DEBUG: Scryer query: {}", query);

    // Run the query
    let mut results = machine.run_query(&query);

    match results.next() {
        Some(Ok(answer)) => {
            // Extract Tool and Args from the answer
            extract_decision_from_answer(answer, payload)
        }
        Some(Err(exception)) => {
            // Check if it's a missing_required exception
            eprintln!("DEBUG: Scryer exception: {:?}", exception);
            handle_exception(exception)
        }
        None => {
            // No solutions - query failed
            Ok(Decision::Reject {
                reason: "No matching route found".to_string(),
            })
        }
    }
}

/// Extract Tool and Args from a successful query answer
fn extract_decision_from_answer(answer: LeafAnswer, _payload: &IntentPayload) -> Result<Decision> {
    match answer {
        LeafAnswer::LeafAnswer { bindings, .. } => {
            let tool = bindings.get("Tool")
                .ok_or_else(|| anyhow!("Tool not bound in answer"))?;

            let args = bindings.get("Args")
                .ok_or_else(|| anyhow!("Args not bound in answer"))?;

            let tool_name = term_to_atom(tool)?;
            let args_json = term_to_json(args)?;

            Ok(Decision::Route {
                tool: tool_name,
                args: args_json,
            })
        }
        LeafAnswer::True => {
            // Query succeeded but no bindings - shouldn't happen for route/5
            Ok(Decision::Reject {
                reason: "Query succeeded but no variable bindings".to_string(),
            })
        }
        LeafAnswer::False => {
            Ok(Decision::Reject {
                reason: "No matching route found".to_string(),
            })
        }
        LeafAnswer::Exception(term) => {
            handle_exception(term)
        }
        _ => {
            // Handle any future variants due to #[non_exhaustive]
            Ok(Decision::Reject {
                reason: "Unknown answer type".to_string(),
            })
        }
    }
}

/// Convert a Prolog Term to an atom string
fn term_to_atom(term: &Term) -> Result<String> {
    match term {
        Term::Atom(s) => Ok(s.clone()),
        _ => Err(anyhow!("Expected atom, got: {:?}", term)),
    }
}

/// Convert a Prolog Term (association list) to JSON
fn term_to_json(term: &Term) -> Result<serde_json::Value> {
    match term {
        Term::List(items) => {
            let mut map = serde_json::Map::new();
            for item in items {
                if let Term::Compound(functor, args) = item {
                    // Association list items are Key-Value compounds
                    if functor == "-" && args.len() == 2 {
                        let key = term_to_string(&args[0])?;
                        let value = term_to_json_value(&args[1])?;
                        map.insert(key, value);
                    }
                }
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Err(anyhow!("Expected list, got: {:?}", term)),
    }
}

/// Convert a Term to a string (for keys)
fn term_to_string(term: &Term) -> Result<String> {
    match term {
        Term::Atom(s) => Ok(s.clone()),
        Term::String(s) => Ok(s.clone()),
        _ => Err(anyhow!("Expected atom or string, got: {:?}", term)),
    }
}

/// Convert a Term to a JSON value
fn term_to_json_value(term: &Term) -> Result<serde_json::Value> {
    match term {
        Term::Atom(s) => Ok(serde_json::Value::String(s.clone())),
        Term::String(s) => Ok(serde_json::Value::String(s.clone())),
        Term::Integer(n) => {
            // IBig doesn't have to_i64, use string representation
            // Try to parse as i64, fall back to string for big integers
            let s = n.to_string();
            if let Ok(i) = s.parse::<i64>() {
                Ok(serde_json::Value::Number(i.into()))
            } else {
                Ok(serde_json::Value::String(s))
            }
        }
        Term::Float(f) => Ok(serde_json::json!(*f)),
        Term::List(items) => {
            let arr: Result<Vec<_>> = items.iter().map(term_to_json_value).collect();
            Ok(serde_json::Value::Array(arr?))
        }
        Term::Var(_) => Ok(serde_json::Value::Null),
        _ => Ok(serde_json::Value::String(format!("{:?}", term))),
    }
}

/// Handle a Prolog exception and convert to Decision
fn handle_exception(exception: Term) -> Result<Decision> {
    // Look for error(missing_required(Field), _) pattern
    if let Term::Compound(functor, args) = &exception {
        if functor == "error" && !args.is_empty() {
            if let Term::Compound(inner_functor, inner_args) = &args[0] {
                if inner_functor == "missing_required" && inner_args.len() == 1 {
                    let field = term_to_string(&inner_args[0])?;
                    let question = match field.as_str() {
                        "recipient" => "Who should I email?",
                        "date" => "When is this due? (e.g., tomorrow, next Friday)",
                        "location" => "What location should I use?",
                        "topic" => "What topic should I use?",
                        _ => return Ok(Decision::NeedInfo {
                            question: format!("What {} should I use?", field),
                        }),
                    };
                    return Ok(Decision::NeedInfo {
                        question: question.to_string(),
                    });
                }
            }
        }
    }

    // Unknown exception
    Ok(Decision::Reject {
        reason: format!("Prolog exception: {:?}", exception),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_to_json_simple_list() {
        let term = Term::List(vec![
            Term::Compound("-".to_string(), vec![
                Term::Atom("query".to_string()),
                Term::String("AI".to_string()),
            ]),
        ]);

        let json = term_to_json(&term).unwrap();
        assert_eq!(json["query"], "AI");
    }
}
