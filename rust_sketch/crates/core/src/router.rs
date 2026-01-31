//! Stub Prolog Router
//!
//! Simulates what router.pl would return when Prolog backends are not available.

use crate::apple_weather;
use crate::types::{
    Decision, IntentPayload, IntentType, SourcePreference, ToPrologDict,
    WeatherQueryType,
};

/// Stub Prolog router - simulates router.pl decisions
pub fn prolog_decide_stub(payload: &IntentPayload, verbose: bool) -> Decision {
    let intent = &payload.intent;
    let entities = &payload.entities;
    let constraints = &payload.constraints;

    // Print the query that would be sent to Prolog
    if verbose {
        let query = format!(
            "route({}, {}, {}, Tool, Args)",
            intent.as_atom(),
            entities.to_prolog_dict(),
            constraints.to_prolog_dict()
        );
        eprintln!("DEBUG: Prolog query: {}", query);
    }

    // Simulate routing logic from router.pl
    match intent {
        IntentType::Summarize | IntentType::Find => {
            let query_text = entities
                .topic
                .clone()
                .or_else(|| entities.query.clone())
                .unwrap_or_default();

            if query_text.is_empty() {
                return Decision::NeedInfo {
                    question: format!("What should I {}?", intent.as_atom()),
                };
            }

            let tool = match constraints.source_preference {
                SourcePreference::Notes => "search_notes",
                _ => "search_files",
            };

            Decision::Route {
                tool: tool.to_string(),
                args: serde_json::json!({
                    "query": query_text,
                    "scope": "user"
                }),
            }
        }

        IntentType::Weather => {
            let location = match &entities.location {
                Some(loc) => loc.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "What location should I use?".to_string(),
                    }
                }
            };

            // Date is now optional - defaults to today for current weather
            let date = entities.date.clone();

            // Get date_end and weather_query for forecasts/assessments
            let date_end = entities.date_end.clone();
            let weather_query = entities.weather_query.as_ref().map(|q| match q {
                WeatherQueryType::Current => "current",
                WeatherQueryType::Forecast => "forecast",
                WeatherQueryType::Assessment => "assessment",
            });

            // Prefer Apple WeatherKit if configured, otherwise use OpenWeatherMap
            let tool = if apple_weather::is_configured() {
                "get_apple_weather"
            } else {
                "get_weather"
            };

            let mut args = serde_json::json!({
                "location": location
            });

            // Add optional fields only if present
            if let Some(d) = date {
                args["date"] = serde_json::Value::String(d);
            }
            if let Some(de) = date_end {
                args["date_end"] = serde_json::Value::String(de);
            }
            if let Some(wq) = weather_query {
                args["weather_query"] = serde_json::Value::String(wq.to_string());
            }

            Decision::Route {
                tool: tool.to_string(),
                args,
            }
        }

        IntentType::Draft => {
            let recipient = match &entities.recipient {
                Some(r) => r.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "Who should I email?".to_string(),
                    }
                }
            };

            let subject = entities
                .topic
                .clone()
                .unwrap_or_else(|| "(no subject)".to_string());

            Decision::Route {
                tool: "draft_email".to_string(),
                args: serde_json::json!({
                    "to": recipient,
                    "subject": subject,
                    "body": ""
                }),
            }
        }

        IntentType::Remind => {
            let title = match &entities.topic {
                Some(t) => t.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "What should I remind you about?".to_string(),
                    }
                }
            };

            let due = match &entities.date {
                Some(d) => d.clone(),
                None => {
                    return Decision::NeedInfo {
                        question: "When is this due? (e.g., tomorrow, next Friday, 2026-02-01)"
                            .to_string(),
                    }
                }
            };

            let priority = entities
                .priority
                .clone()
                .unwrap_or_else(|| "normal".to_string());

            Decision::Route {
                tool: "create_todo".to_string(),
                args: serde_json::json!({
                    "title": title,
                    "due": due,
                    "priority": priority
                }),
            }
        }

        IntentType::Unknown => Decision::Reject {
            reason: "No matching route or follow-up found.".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Constraints, Entities};

    #[test]
    fn test_prolog_decide_summarize() {
        let payload = IntentPayload {
            intent: IntentType::Summarize,
            entities: Entities {
                topic: Some("machine learning".to_string()),
                ..Default::default()
            },
            constraints: Constraints {
                source_preference: SourcePreference::Notes,
                ..Default::default()
            },
        };

        let decision = prolog_decide_stub(&payload, false);
        match decision {
            Decision::Route { tool, args } => {
                assert_eq!(tool, "search_notes");
                assert_eq!(args["query"], "machine learning");
            }
            _ => panic!("Expected Route decision"),
        }
    }

    #[test]
    fn test_prolog_decide_weather_missing_location() {
        let payload = IntentPayload {
            intent: IntentType::Weather,
            entities: Entities {
                date: Some("tomorrow".to_string()),
                ..Default::default()
            },
            constraints: Constraints::default(),
        };

        let decision = prolog_decide_stub(&payload, false);
        match decision {
            Decision::NeedInfo { question } => {
                assert!(question.contains("location"));
            }
            _ => panic!("Expected NeedInfo decision"),
        }
    }
}
