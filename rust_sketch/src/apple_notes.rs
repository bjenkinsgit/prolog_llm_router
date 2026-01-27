//! Apple Notes Integration via AppleScript
//!
//! Provides search, list, and retrieval of Apple Notes using external AppleScript files.
//! Uses a delimiter-based parsing protocol for reliable cross-language communication.

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::process::Command;

const SCRIPTS_DIR: &str = "scripts";

// ============================================================================
// Data Structures
// ============================================================================

/// A note record parsed from AppleScript output
#[derive(Debug, Serialize)]
pub struct NoteRecord {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub modified: String,
    pub snippet: String,
    /// Command to open this note in Notes.app
    pub open_cmd: String,
}

/// Full note content (includes body)
#[derive(Debug, Serialize)]
pub struct NoteContent {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub modified: String,
    pub body: String,
    /// Command to open this note in Notes.app
    pub open_cmd: String,
}

// ============================================================================
// Availability Check
// ============================================================================

/// Check if Apple Notes scripts are available (macOS only)
pub fn is_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new(SCRIPTS_DIR)
            .join("notes_search.applescript")
            .exists()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

// ============================================================================
// Script Execution
// ============================================================================

/// Execute an AppleScript and return raw output
fn run_script(script_name: &str, args: &[&str]) -> Result<String> {
    let script_path = format!("{}/{}", SCRIPTS_DIR, script_name);

    // Verify script exists
    if !std::path::Path::new(&script_path).exists() {
        return Err(anyhow!("Script not found: {}", script_path));
    }

    let mut cmd = Command::new("osascript");
    cmd.arg(&script_path);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd
        .output()
        .map_err(|e| anyhow!("Failed to execute osascript: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("AppleScript error: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ============================================================================
// Output Parsing
// ============================================================================

/// Parse delimiter-based output into NoteRecords
///
/// Expected format:
/// ```text
/// RECORD_START
/// id: x-coredata://...
/// title: Note Title
/// folder: Folder Name
/// modified: 2026-01-27T10:30:00Z
/// snippet: First 200 characters...
/// RECORD_END
/// ```
fn parse_records(output: &str) -> Result<Vec<NoteRecord>> {
    let mut records = Vec::new();
    let mut current: Option<NoteRecord> = None;

    for line in output.lines() {
        let line = line.trim();

        if line == "RECORD_START" {
            current = Some(NoteRecord {
                id: String::new(),
                title: String::new(),
                folder: String::new(),
                modified: String::new(),
                snippet: String::new(),
                open_cmd: String::new(),
            });
        } else if line == "RECORD_END" {
            if let Some(mut record) = current.take() {
                // Generate command to open this note
                if !record.id.is_empty() {
                    record.open_cmd =
                        format!("osascript scripts/notes_open.applescript \"{}\"", record.id);
                }
                records.push(record);
            }
        } else if line.starts_with("ERROR:") {
            return Err(anyhow!("{}", line));
        } else if let Some(ref mut record) = current {
            if let Some((key, value)) = line.split_once(": ") {
                match key {
                    "id" => record.id = value.to_string(),
                    "title" => record.title = value.to_string(),
                    "folder" => record.folder = value.to_string(),
                    "modified" => record.modified = value.to_string(),
                    "snippet" => record.snippet = value.to_string(),
                    _ => {}
                }
            }
        }
    }

    Ok(records)
}

/// Parse full note content from AppleScript output
fn parse_note_content(output: &str) -> Result<NoteContent> {
    let mut note = NoteContent {
        id: String::new(),
        title: String::new(),
        folder: String::new(),
        modified: String::new(),
        body: String::new(),
        open_cmd: String::new(),
    };

    let mut in_body = false;
    let mut body_lines = Vec::new();

    for line in output.lines() {
        let line_trimmed = line.trim();

        if line_trimmed.starts_with("ERROR:") {
            return Err(anyhow!("{}", line_trimmed));
        }

        if in_body {
            if line_trimmed == "BODY_END" {
                in_body = false;
                note.body = body_lines.join("\n");
            } else {
                body_lines.push(line.to_string());
            }
        } else if line_trimmed == "BODY_START" {
            in_body = true;
        } else if let Some((key, value)) = line_trimmed.split_once(": ") {
            match key {
                "id" => note.id = value.to_string(),
                "title" => note.title = value.to_string(),
                "folder" => note.folder = value.to_string(),
                "modified" => note.modified = value.to_string(),
                _ => {}
            }
        }
    }

    if note.id.is_empty() {
        return Err(anyhow!("Failed to parse note content"));
    }

    // Generate command to open this note
    note.open_cmd = format!("osascript scripts/notes_open.applescript \"{}\"", note.id);

    Ok(note)
}

// ============================================================================
// Public API
// ============================================================================

/// Search notes by query string
pub fn search_notes(query: &str, folder: Option<&str>) -> Result<String> {
    let args: Vec<&str> = match folder {
        Some(f) => vec![query, f],
        None => vec![query],
    };

    let output = run_script("notes_search.applescript", &args)?;
    let records = parse_records(&output)?;

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "count": records.len(),
        "notes": records
    }))?)
}

/// List notes, optionally filtered by folder
pub fn list_notes(folder: Option<&str>) -> Result<String> {
    let args: Vec<&str> = folder.map(|f| vec![f]).unwrap_or_default();

    let output = run_script("notes_list.applescript", &args)?;
    let records = parse_records(&output)?;

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "count": records.len(),
        "notes": records
    }))?)
}

/// Get full note content by ID
pub fn get_note(note_id: &str) -> Result<String> {
    let output = run_script("notes_get.applescript", &[note_id])?;
    let note = parse_note_content(&output)?;

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "note": note
    }))?)
}

// ============================================================================
// Tool Executor Integration
// ============================================================================

/// Main entry point for agent tool execution
pub fn execute_apple_notes(action: &str, args: &Value) -> Result<String> {
    match action {
        "search" => {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing required 'query' argument"))?;
            let folder = args.get("folder").and_then(|v| v.as_str());
            search_notes(query, folder)
        }
        "list" => {
            let folder = args.get("folder").and_then(|v| v.as_str());
            list_notes(folder)
        }
        "get" => {
            let id = args["id"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing required 'id' argument"))?;
            get_note(id)
        }
        _ => Err(anyhow!("Unknown Apple Notes action: {}", action)),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_records_empty() {
        let output = "";
        let records = parse_records(output).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn test_parse_records_single() {
        let output = r#"RECORD_START
id: x-coredata://123
title: Test Note
folder: Notes
modified: 2026-01-27T10:30:00Z
snippet: This is a test note...
RECORD_END"#;

        let records = parse_records(output).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "x-coredata://123");
        assert_eq!(records[0].title, "Test Note");
        assert_eq!(records[0].folder, "Notes");
        assert_eq!(records[0].snippet, "This is a test note...");
        assert_eq!(
            records[0].open_cmd,
            "osascript scripts/notes_open.applescript \"x-coredata://123\""
        );
    }

    #[test]
    fn test_parse_records_multiple() {
        let output = r#"RECORD_START
id: note-1
title: First Note
folder: Work
modified: 2026-01-27T10:00:00Z
snippet: First note content
RECORD_END
RECORD_START
id: note-2
title: Second Note
folder: Personal
modified: 2026-01-27T11:00:00Z
snippet: Second note content
RECORD_END"#;

        let records = parse_records(output).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].title, "First Note");
        assert_eq!(records[1].title, "Second Note");
    }

    #[test]
    fn test_parse_records_error() {
        let output = "ERROR: Notes application not available";
        let result = parse_records(output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ERROR:"));
    }

    #[test]
    fn test_parse_note_content() {
        let output = r#"id: x-coredata://123
title: Full Note
folder: Notes
modified: 2026-01-27T10:30:00Z
BODY_START
This is the full body of the note.
It can have multiple lines.

And paragraphs.
BODY_END"#;

        let note = parse_note_content(output).unwrap();
        assert_eq!(note.id, "x-coredata://123");
        assert_eq!(note.title, "Full Note");
        assert!(note.body.contains("multiple lines"));
        assert!(note.body.contains("paragraphs"));
        assert_eq!(
            note.open_cmd,
            "osascript scripts/notes_open.applescript \"x-coredata://123\""
        );
    }

    #[test]
    fn test_is_available_without_scripts() {
        // This test just ensures the function runs without panicking
        // Actual availability depends on whether scripts exist
        let _ = is_available();
    }
}
