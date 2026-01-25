//! Sketch of a proc-macro that would enable true DRY
//!
//! This shows what a custom derive macro COULD look like that generates
//! both serde traits AND Prolog term conversion from a single struct.
//!
//! This is reference/example code, not used in production.
//!
//! Usage would be:
//! ```
//! #[derive(SerdeProlog)]
//! struct Entities {
//!     topic: Option<String>,
//!     location: Option<String>,
//! }
//! ```
//!
//! The macro would generate:
//! 1. impl Serialize for Entities { ... }
//! 2. impl Deserialize for Entities { ... }
//! 3. impl ToPrologTerm for Entities { ... }
//! 4. impl FromPrologTerm for Entities { ... }

// Allow dead code - this is a reference sketch, not production code
#![allow(dead_code)]

// ============================================================================
// What the derive macro would generate (pseudocode)
// ============================================================================

/*
Given this input:

#[derive(SerdeProlog)]
#[prolog(dict)]  // Represent as SWI-Prolog dict
pub struct Entities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

The macro would expand to:

impl serde::Serialize for Entities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer
    {
        // Standard serde map serialization, skipping None fields
        let mut map = serializer.serialize_map(None)?;
        if let Some(ref v) = self.topic {
            map.serialize_entry("topic", v)?;
        }
        if let Some(ref v) = self.location {
            map.serialize_entry("location", v)?;
        }
        map.end()
    }
}

impl serde::Deserialize for Entities { ... }

impl ToPrologTerm for Entities {
    fn to_prolog_term(&self, context: &PrologContext) -> Term {
        let mut dict = context.new_dict();
        if let Some(ref v) = self.topic {
            dict.put("topic", v.to_prolog_term(context));
        }
        if let Some(ref v) = self.location {
            dict.put("location", v.to_prolog_term(context));
        }
        dict.into_term()
    }
}

impl FromPrologTerm for Entities {
    fn from_prolog_term(term: &Term) -> Result<Self, PrologError> {
        let dict = term.as_dict()?;
        Ok(Entities {
            topic: dict.get("topic").map(String::from_prolog_term).transpose()?,
            location: dict.get("location").map(String::from_prolog_term).transpose()?,
        })
    }
}
*/

// ============================================================================
// Alternative: Attribute macro for field-level control
// ============================================================================

/*
For more control, you could have field-level attributes:

#[derive(SerdeProlog)]
pub struct WeatherQuery {
    #[prolog(atom)]           // Serialize as Prolog atom, not string
    pub intent: IntentType,

    #[prolog(dict)]           // Nested dict
    pub entities: Entities,

    #[prolog(skip)]           // Don't include in Prolog term
    #[serde(skip)]
    pub internal_id: u64,

    #[prolog(rename = "loc")] // Different name in Prolog
    #[serde(rename = "location")]
    pub location: String,
}
*/

// ============================================================================
// The actual trait definitions the macro would implement
// ============================================================================

use std::collections::HashMap;

/// Represents a Prolog term (simplified)
#[derive(Debug, Clone)]
pub enum PrologTerm {
    Atom(String),
    String(String),
    Integer(i64),
    Float(f64),
    List(Vec<PrologTerm>),
    Dict(HashMap<String, PrologTerm>),
    Compound { functor: String, args: Vec<PrologTerm> },
    Variable(String),
}

impl PrologTerm {
    /// Convert to SWI-Prolog syntax string
    pub fn to_syntax(&self) -> String {
        match self {
            PrologTerm::Atom(s) => s.clone(),
            PrologTerm::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            PrologTerm::Integer(n) => n.to_string(),
            PrologTerm::Float(f) => f.to_string(),
            PrologTerm::List(items) => {
                let inner: Vec<String> = items.iter().map(|t| t.to_syntax()).collect();
                format!("[{}]", inner.join(", "))
            }
            PrologTerm::Dict(map) => {
                let pairs: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{}:{}", k, v.to_syntax()))
                    .collect();
                format!("_{{{}}}", pairs.join(", "))
            }
            PrologTerm::Compound { functor, args } => {
                let arg_strs: Vec<String> = args.iter().map(|t| t.to_syntax()).collect();
                format!("{}({})", functor, arg_strs.join(", "))
            }
            PrologTerm::Variable(name) => name.clone(),
        }
    }
}

/// Trait for converting TO Prolog terms
pub trait ToPrologTerm {
    fn to_prolog_term(&self) -> PrologTerm;
}

/// Trait for converting FROM Prolog terms
pub trait FromPrologTerm: Sized {
    fn from_prolog_term(term: &PrologTerm) -> Result<Self, String>;
}

// Implementations for primitive types
impl ToPrologTerm for String {
    fn to_prolog_term(&self) -> PrologTerm {
        PrologTerm::String(self.clone())
    }
}

impl ToPrologTerm for &str {
    fn to_prolog_term(&self) -> PrologTerm {
        PrologTerm::String(self.to_string())
    }
}

impl ToPrologTerm for i64 {
    fn to_prolog_term(&self) -> PrologTerm {
        PrologTerm::Integer(*self)
    }
}

impl ToPrologTerm for f64 {
    fn to_prolog_term(&self) -> PrologTerm {
        PrologTerm::Float(*self)
    }
}

impl ToPrologTerm for bool {
    fn to_prolog_term(&self) -> PrologTerm {
        PrologTerm::Atom(if *self { "true" } else { "false" }.to_string())
    }
}

impl<T: ToPrologTerm> ToPrologTerm for Option<T> {
    fn to_prolog_term(&self) -> PrologTerm {
        match self {
            Some(v) => v.to_prolog_term(),
            None => PrologTerm::Atom("null".to_string()),
        }
    }
}

impl<T: ToPrologTerm> ToPrologTerm for Vec<T> {
    fn to_prolog_term(&self) -> PrologTerm {
        PrologTerm::List(self.iter().map(|v| v.to_prolog_term()).collect())
    }
}

// ============================================================================
// Example: What the user's code would look like with the macro
// ============================================================================

// With the macro, you'd write just this:
//
// #[derive(Debug, Clone, SerdeProlog)]
// #[prolog(dict)]
// pub struct Entities {
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub topic: Option<String>,
//     pub location: Option<String>,
// }
//
// And get BOTH serde JSON support AND Prolog dict conversion for free.
//
// Usage:
//   let e = Entities { topic: Some("AI".into()), location: None };
//   let json = serde_json::to_string(&e)?;           // JSON serialization
//   let term = e.to_prolog_term();                   // Prolog term
//   let syntax = term.to_syntax();                   // "_{topic:\"AI\"}"

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prolog_term_syntax() {
        let mut dict = HashMap::new();
        dict.insert("topic".to_string(), PrologTerm::String("AI".to_string()));
        dict.insert("date".to_string(), PrologTerm::String("tomorrow".to_string()));

        let term = PrologTerm::Dict(dict);
        let syntax = term.to_syntax();

        assert!(syntax.starts_with("_{"));
        assert!(syntax.contains("topic:\"AI\""));
        assert!(syntax.contains("date:\"tomorrow\""));
    }

    #[test]
    fn test_nested_terms() {
        let inner = PrologTerm::Dict({
            let mut m = HashMap::new();
            m.insert("x".to_string(), PrologTerm::Integer(42));
            m
        });

        let outer = PrologTerm::Compound {
            functor: "route".to_string(),
            args: vec![
                PrologTerm::Atom("summarize".to_string()),
                inner,
                PrologTerm::Variable("Tool".to_string()),
            ],
        };

        let syntax = outer.to_syntax();
        assert_eq!(syntax, "route(summarize, _{x:42}, Tool)");
    }
}
