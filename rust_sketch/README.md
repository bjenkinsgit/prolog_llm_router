# Rust DRY Approach: Unified Serde + Prolog Structs

This sketch explores how to achieve the DRY principle in Rust for a system that needs:
1. **JSON serialization** (for LLM structured outputs via serde)
2. **Prolog term conversion** (for logic programming queries)

## The Problem

In the Python version, we have:
- `IntentPayload` Pydantic model for JSON
- `entities_to_dict()` / `to_swi_dict()` functions for Prolog
- Data is copied between representations

## Three Approaches

### Approach 1: Dual Derive (Ideal)

```rust
#[derive(Serialize, Deserialize, Unifiable)]  // Both!
pub struct Entities {
    pub topic: Option<String>,
    pub location: Option<String>,
}
```

**Pros:** True DRY, compile-time checked
**Cons:** Requires `swipl` crate's `Unifiable` to map well to SWI dicts

### Approach 2: Manual Trait Implementation

```rust
#[derive(Serialize, Deserialize)]
pub struct Entities { ... }

impl ToPrologDict for Entities {
    fn to_prolog_dict(&self) -> String { ... }
}
```

**Pros:** Full control over Prolog representation
**Cons:** Still some duplication in trait impl

### Approach 3: JSON as Intermediate

```rust
// Any Serialize type can become a Prolog dict
impl<T: Serialize> ToPrologDictViaJson for T {}

let prolog_syntax = payload.to_prolog_dict_via_json()?;
```

**Pros:** Zero duplication, works with any serde type
**Cons:** Runtime JSON conversion overhead

## Key Insight

The cleanest solution would be a **custom proc-macro** that generates both:

```rust
#[derive(SerdeProlog)]
#[prolog(dict)]
pub struct Entities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

// Expands to impl Serialize, Deserialize, ToPrologTerm, FromPrologTerm
```

See `src/derive_sketch.rs` for the trait definitions.

## Comparison to Python

| Aspect | Python (current) | Rust (proposed) |
|--------|------------------|-----------------|
| Type safety | Runtime (Pydantic) | Compile-time |
| Conversion | Manual functions | Derive macros |
| Single source of truth | No (models + converters) | Yes (one struct) |
| Prolog binding | pyswip (dynamic) | swipl-rs (typed) |

## Running the Sketch

```bash
cd rust_sketch

# Run with real SWI-Prolog (requires SWI_HOME_DIR on macOS)
export SWI_HOME_DIR=/Applications/SWI-Prolog.app/Contents/swipl
cargo run -- "summarize my notes about AI"

# Run with stub router (no SWI-Prolog needed)
cargo run -- --stub "summarize my notes about AI"

# Run tests
cargo test
```

**Prerequisites:**
- SWI-Prolog must be installed for the `swipl` crate to build
- On macOS, set `SWI_HOME_DIR` to the swipl resources directory

**CLI Options:**
- `--stub` - Use stub Prolog router instead of real SWI-Prolog
- `--router <path>` - Path to router.pl file (default: `../router.pl`)
- `--date`, `--location`, `--recipient`, `--source` - Override extracted entities
