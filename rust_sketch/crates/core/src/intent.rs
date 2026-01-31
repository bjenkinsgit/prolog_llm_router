//! Stub Intent Extraction
//!
//! Simple heuristic-based intent extraction for testing routing.

use crate::types::{
    Constraints, Entities, IntentPayload, IntentType, SourcePreference, WeatherQueryType,
    resolve_date_range, resolve_relative_date,
};

/// Extract location from weather queries using "in <city>" or "for <city>" patterns
fn extract_weather_location(lower_text: &str, original_text: &str) -> Option<String> {
    // Try "in <city>" pattern
    for pattern in &[" in ", " for "] {
        if let Some(idx) = lower_text.find(pattern) {
            let start = idx + pattern.len();
            let after = &original_text[start..];
            let after_lower = after.to_lowercase();

            // Take words until we hit a date keyword (with word boundary) or end
            // Use " keyword" patterns to ensure word boundaries
            let end_markers = [
                " today",
                " tomorrow",
                " next",
                " this",
                "?",
                "!",
                ".",
            ];
            let mut end_idx = after.len();
            for marker in end_markers {
                if let Some(m_idx) = after_lower.find(marker) {
                    if m_idx < end_idx {
                        end_idx = m_idx;
                    }
                }
            }
            let city = after[..end_idx].trim();
            if !city.is_empty() {
                return Some(city.to_string());
            }
        }
    }
    None
}

/// Extract "next N days" pattern and return the matched string
fn extract_next_n_days(lower_text: &str) -> Option<String> {
    if let Some(idx) = lower_text.find("next ") {
        let after_next = &lower_text[idx + 5..];
        // Look for "<number> days" pattern
        let words: Vec<&str> = after_next.split_whitespace().collect();
        if words.len() >= 2 && words[1].starts_with("day") {
            if words[0].parse::<u64>().is_ok() {
                return Some(format!("next {} days", words[0]));
            }
        }
    }
    None
}

/// Simple heuristic intent extractor for testing routing
pub fn extract_intent_stub(user_text: &str) -> IntentPayload {
    let t = user_text.to_lowercase();

    // Detect source preference
    let source_pref = if t.contains("notes") {
        SourcePreference::Notes
    } else if t.contains("files") {
        SourcePreference::Files
    } else {
        SourcePreference::Either
    };

    // Naive intent detection
    let intent = if t.starts_with("summarize") || t.contains("summarize") {
        IntentType::Summarize
    } else if t.starts_with("find") || t.starts_with("search") || t.contains("look for") {
        IntentType::Find
    } else if t.contains("weather")
        || t.starts_with("forecast")
        || t.contains("forecast")
        || t.contains("will it rain")
        || t.contains("will it snow")
        || t.contains("bad weather")
    {
        IntentType::Weather
    } else if t.contains("email") || t.starts_with("draft") {
        IntentType::Draft
    } else if t.contains("remind") || t.contains("todo") {
        IntentType::Remind
    } else {
        IntentType::Unknown
    };

    // Naive topic extraction: everything after "about"
    let topic = if let Some(idx) = t.find("about") {
        let after_about = &user_text[idx + 5..];
        Some(after_about.trim().to_string())
    } else {
        // Or after the first word for summarize/find
        let pieces: Vec<&str> = user_text.splitn(2, ' ').collect();
        if pieces.len() == 2 && matches!(intent, IntentType::Summarize | IntentType::Find) {
            Some(pieces[1].trim().to_string())
        } else {
            None
        }
    };

    // Weather-specific extraction
    let (date, date_end, location, weather_query) = if intent == IntentType::Weather {
        // Extract location from "in <city>" or "for <city>" patterns
        let location = extract_weather_location(&t, user_text);

        // Detect weather query type
        let weather_query = if t.contains("bad weather")
            || t.contains("expecting")
            || t.contains("will it rain")
            || t.contains("will it snow")
            || t.contains("expect rain")
            || t.contains("expect snow")
        {
            Some(WeatherQueryType::Assessment)
        } else if t.contains("forecast") || t.contains("next week") || t.contains("next ") && t.contains(" days") {
            Some(WeatherQueryType::Forecast)
        } else {
            Some(WeatherQueryType::Current)
        };

        // Extract date/date range
        let (date, date_end) = if t.contains("next week") {
            let (start, end) = resolve_date_range("next week");
            (Some(start), end)
        } else if t.contains("this weekend") {
            let (start, end) = resolve_date_range("this weekend");
            (Some(start), end)
        } else if let Some(cap) = extract_next_n_days(&t) {
            let (start, end) = resolve_date_range(&cap);
            (Some(start), end)
        } else if t.contains("forecast") && !t.contains("tomorrow") && !t.contains("today") {
            // Generic "forecast" without specific date → default to 7 days
            let (start, end) = resolve_date_range("forecast");
            (Some(start), end)
        } else if t.contains("tomorrow") {
            (Some(resolve_relative_date("tomorrow")), None)
        } else if t.contains("today") {
            (Some(resolve_relative_date("today")), None)
        } else {
            // Default to today for current weather queries
            (None, None)
        };

        (date, date_end, location, weather_query)
    } else {
        (None, None, None, None)
    };

    // For non-weather intents, date extraction (toy)
    let date = date.or_else(|| {
        if intent != IntentType::Weather {
            if t.contains("tomorrow") {
                Some("tomorrow".to_string())
            } else if t.contains("today") {
                Some("today".to_string())
            } else {
                None
            }
        } else {
            None
        }
    });

    // Recipient extraction for draft/email (look for "email to <name>" or "to <name>" after email keyword)
    let recipient = if intent == IntentType::Draft {
        // Try "email to X" pattern first
        if let Some(idx) = t.find("email to ") {
            let after_to = &user_text[idx + 9..];
            let recipient_word = after_to.split_whitespace().next();
            recipient_word.map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
        // Try "mail to X" pattern
        } else if let Some(idx) = t.find("mail to ") {
            let after_to = &user_text[idx + 8..];
            let recipient_word = after_to.split_whitespace().next();
            recipient_word.map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
        // Try finding "to X" after "email" keyword
        } else if let Some(email_idx) = t.find("email") {
            let after_email = &t[email_idx..];
            if let Some(to_idx) = after_email.find(" to ") {
                let global_idx = email_idx + to_idx + 4;
                let after_to = &user_text[global_idx..];
                let recipient_word = after_to.split_whitespace().next();
                recipient_word.map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    IntentPayload {
        intent,
        entities: Entities {
            topic,
            date,
            date_end,
            location,
            recipient,
            weather_query,
            ..Default::default()
        },
        constraints: Constraints {
            source_preference: source_pref,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Days, Local};

    #[test]
    fn test_stub_extractor_summarize() {
        let payload = extract_intent_stub("summarize my notes about AI");
        assert_eq!(payload.intent, IntentType::Summarize);
        assert_eq!(payload.entities.topic, Some("AI".to_string()));
        assert_eq!(payload.constraints.source_preference, SourcePreference::Notes);
    }

    #[test]
    fn test_stub_extractor_weather() {
        let payload = extract_intent_stub("what's the weather tomorrow");
        assert_eq!(payload.intent, IntentType::Weather);
        // Date is now resolved to absolute format
        let expected = (Local::now().date_naive() + Days::new(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(payload.entities.date, Some(expected));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Current));
    }

    #[test]
    fn test_stub_extractor_weather_with_location() {
        let payload = extract_intent_stub("what's the weather in London tomorrow");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("London".to_string()));
        assert!(payload.entities.date.is_some());
    }

    #[test]
    fn test_stub_extractor_weather_forecast() {
        let payload = extract_intent_stub("forecast for Seattle next week");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("Seattle".to_string()));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Forecast));
        // Should have both date and date_end for "next week"
        assert!(payload.entities.date.is_some());
        assert!(payload.entities.date_end.is_some());
    }

    #[test]
    fn test_stub_extractor_weather_assessment() {
        let payload = extract_intent_stub("bad weather in Chicago tomorrow");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("Chicago".to_string()));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Assessment));
    }

    #[test]
    fn test_stub_extractor_will_it_rain() {
        let payload = extract_intent_stub("will it rain in Boston this weekend");
        assert_eq!(payload.intent, IntentType::Weather);
        assert_eq!(payload.entities.location, Some("Boston".to_string()));
        assert_eq!(payload.entities.weather_query, Some(WeatherQueryType::Assessment));
        // "this weekend" should produce a date range
        assert!(payload.entities.date.is_some());
        assert!(payload.entities.date_end.is_some());
    }

    #[test]
    fn test_stub_extractor_unknown() {
        let payload = extract_intent_stub("hello world");
        assert_eq!(payload.intent, IntentType::Unknown);
    }

    #[test]
    fn test_stub_extractor_email_with_recipient() {
        let payload = extract_intent_stub("send an email to Mary");
        assert_eq!(payload.intent, IntentType::Draft);
        assert_eq!(payload.entities.recipient, Some("Mary".to_string()));
    }

    #[test]
    fn test_stub_extractor_email_to_pattern() {
        let payload = extract_intent_stub("I want you to send an email to John about the project");
        assert_eq!(payload.intent, IntentType::Draft);
        assert_eq!(payload.entities.recipient, Some("John".to_string()));
        assert_eq!(payload.entities.topic, Some("the project".to_string()));
    }
}
