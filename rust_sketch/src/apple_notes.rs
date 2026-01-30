//! Apple Notes Integration via AppleScript
//!
//! Provides search, list, and retrieval of Apple Notes using external AppleScript files.
//! Uses a delimiter-based parsing protocol for reliable cross-language communication.
//!
//! Includes a tag indexing system that caches note metadata and extracted hashtags
//! for fast tag-based queries without rescanning all notes.

use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SCRIPTS_DIR: &str = "scripts";

/// Default path for the notes index cache file
fn default_index_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("apple_notes_index.json")
}

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
// Tag Index Data Structures
// ============================================================================

/// Indexed note metadata (stored in cache)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedNote {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub modified: String,
    pub tags: Vec<String>,
}

/// The full notes index (persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotesIndex {
    /// Number of notes when index was built (for staleness check)
    pub note_count: usize,
    /// ISO 8601 timestamp when index was last updated
    pub last_updated: String,
    /// Map from tag -> list of note IDs
    pub tags: HashMap<String, Vec<String>>,
    /// Map from note ID -> indexed note metadata
    pub notes: HashMap<String, IndexedNote>,
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

/// Open a note in Notes.app by ID
pub fn open_note(note_id: &str) -> Result<String> {
    let output = run_script("notes_open.applescript", &[note_id])?;

    // Parse the result - expects "OK: Opened note: <title>" or "ERROR: <message>"
    let output = output.trim();
    if output.starts_with("ERROR:") {
        return Err(anyhow!("{}", output));
    }

    // Extract note title from success message
    let title = output
        .strip_prefix("OK: Opened note: ")
        .unwrap_or("Unknown");

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "message": format!("Opened note '{}' in Notes.app", title),
        "note_id": note_id
    }))?)
}

// ============================================================================
// Tag Index Functions
// ============================================================================

/// Load the notes index from disk
pub fn load_index() -> Result<NotesIndex> {
    let path = default_index_path();
    if !path.exists() {
        return Err(anyhow!("Index not found. Run 'notes_index' to build it."));
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| anyhow!("Failed to read index file: {}", e))?;

    serde_json::from_str(&content)
        .map_err(|e| anyhow!("Failed to parse index file: {}", e))
}

/// Save the notes index to disk
fn save_index(index: &NotesIndex) -> Result<()> {
    let path = default_index_path();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow!("Failed to create cache directory: {}", e))?;
    }

    let content = serde_json::to_string_pretty(index)
        .map_err(|e| anyhow!("Failed to serialize index: {}", e))?;

    fs::write(&path, content)
        .map_err(|e| anyhow!("Failed to write index file: {}", e))?;

    Ok(())
}

/// Get current note count from Notes.app (quick check)
fn get_note_count() -> Result<usize> {
    let output = run_script("notes_count.applescript", &[])?;

    for line in output.lines() {
        if let Some(count_str) = line.strip_prefix("COUNT: ") {
            return count_str
                .trim()
                .parse()
                .map_err(|e| anyhow!("Failed to parse note count: {}", e));
        }
        if line.starts_with("ERROR:") {
            return Err(anyhow!("{}", line));
        }
    }

    Err(anyhow!("Failed to get note count"))
}

/// Check if a tag is a CSS hex color code (e.g., #fff, #ffffff, #rrggbbaa)
#[allow(dead_code)]
fn is_css_color_code(tag: &str) -> bool {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    let len = tag.len();

    // CSS color codes are 3, 6, or 8 hex digits
    if len != 3 && len != 6 && len != 8 {
        return false;
    }

    // All characters must be hex digits
    tag.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse the output from notes_index_build.applescript
#[allow(dead_code)]
fn parse_index_output(output: &str) -> Result<(usize, Vec<IndexedNote>)> {
    let mut note_count = 0;
    let mut notes = Vec::new();
    let mut current: Option<IndexedNote> = None;

    for line in output.lines() {
        let line = line.trim();

        if let Some(count_str) = line.strip_prefix("NOTE_COUNT: ") {
            note_count = count_str.trim().parse().unwrap_or(0);
        } else if line == "RECORD_START" {
            current = Some(IndexedNote {
                id: String::new(),
                title: String::new(),
                folder: String::new(),
                modified: String::new(),
                tags: Vec::new(),
            });
        } else if line == "RECORD_END" {
            if let Some(note) = current.take() {
                notes.push(note);
            }
        } else if line.starts_with("ERROR:") {
            return Err(anyhow!("{}", line));
        } else if let Some(ref mut note) = current {
            if let Some((key, value)) = line.split_once(": ") {
                match key {
                    "id" => note.id = value.to_string(),
                    "title" => note.title = value.to_string(),
                    "folder" => note.folder = value.to_string(),
                    "modified" => note.modified = value.to_string(),
                    "tags" => {
                        note.tags = value
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty() && !is_css_color_code(s))
                            .collect();
                    }
                    _ => {}
                }
            }
        }
    }

    Ok((note_count, notes))
}

// ============================================================================
// SQLite Database Access (for native Apple Notes tags)
// ============================================================================

/// Get the path to the Apple Notes SQLite database
fn notes_database_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite")
    })
}

/// Note with tags from SQLite database
#[derive(Debug)]
struct NoteWithTags {
    identifier: String,
    title: String,
    folder: String,
    modified: String,
    tags: Vec<String>,
}

/// Query notes and their native tags directly from Apple Notes SQLite database
fn query_notes_with_tags_from_db() -> Result<Vec<NoteWithTags>> {
    let db_path = notes_database_path()
        .ok_or_else(|| anyhow!("Could not determine Notes database path"))?;

    if !db_path.exists() {
        return Err(anyhow!("Notes database not found at {:?}", db_path));
    }

    let conn = Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;

    // Get the store UUID from Z_METADATA for constructing x-coredata IDs
    let store_uuid: String = conn.query_row(
        "SELECT Z_UUID FROM Z_METADATA LIMIT 1",
        [],
        |row| row.get(0),
    )?;

    // Query notes with their tags using inline attachments
    // Z_ENT = 12 is ICNote, Z_ENT = 9 is ICInlineAttachment
    // Construct x-coredata ID format: x-coredata://STORE_UUID/ICNote/pNN
    let mut stmt = conn.prepare(
        "SELECT
            'x-coredata://' || ? || '/ICNote/p' || n.Z_PK as note_id,
            n.ZTITLE1,
            COALESCE(f.ZTITLE2, 'Notes') as folder,
            datetime(n.ZMODIFICATIONDATE1 + 978307200, 'unixepoch') as modified,
            GROUP_CONCAT(DISTINCT '#' || LOWER(ia.ZTOKENCONTENTIDENTIFIER)) as tags
        FROM ZICCLOUDSYNCINGOBJECT n
        LEFT JOIN ZICCLOUDSYNCINGOBJECT f ON n.ZFOLDER = f.Z_PK
        LEFT JOIN ZICCLOUDSYNCINGOBJECT ia ON ia.ZNOTE1 = n.Z_PK
            AND ia.Z_ENT = 9
            AND ia.ZTYPEUTI1 = 'com.apple.notes.inlinetextattachment.hashtag'
        WHERE n.Z_ENT = 12
            AND n.ZMARKEDFORDELETION != 1
        GROUP BY n.Z_PK"
    )?;

    let notes = stmt.query_map([&store_uuid], |row| {
        let tags_str: Option<String> = row.get(4)?;
        let tags = tags_str
            .map(|s| s.split(',').map(|t| t.to_string()).collect())
            .unwrap_or_default();

        Ok(NoteWithTags {
            identifier: row.get(0)?,
            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            folder: row.get(2)?,
            modified: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            tags,
        })
    })?;

    notes.collect::<Result<Vec<_>, _>>().map_err(|e| anyhow!("Database query error: {}", e))
}

/// Build or rebuild the notes index
pub fn build_index() -> Result<String> {
    eprint!("Reading Notes database... ");
    let db_notes = query_notes_with_tags_from_db()?;
    let note_count = db_notes.len();
    eprintln!("{} notes found", note_count);

    eprint!("Building tag index... ");
    // Build tag -> note_ids map
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    let mut notes_map: HashMap<String, IndexedNote> = HashMap::new();

    for note in db_notes {
        // Build the note ID in x-coredata format for compatibility
        // The identifier from SQLite is a UUID, we need to look it up for open commands
        let note_id = note.identifier.clone();

        for tag in &note.tags {
            if !tag.is_empty() {
                tags.entry(tag.clone())
                    .or_default()
                    .push(note_id.clone());
            }
        }

        notes_map.insert(
            note_id.clone(),
            IndexedNote {
                id: note_id,
                title: note.title,
                folder: note.folder,
                modified: note.modified,
                tags: note.tags,
            },
        );
    }
    eprintln!("{} tags found", tags.len());

    let index = NotesIndex {
        note_count,
        last_updated: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        tags,
        notes: notes_map,
    };

    eprint!("Saving index... ");
    save_index(&index)?;
    eprintln!("done");

    let tag_count = index.tags.len();
    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "note_count": index.note_count,
        "tag_count": tag_count,
        "last_updated": index.last_updated,
        "index_path": default_index_path().to_string_lossy()
    }))?)
}

/// Check if the index is stale (note count changed)
pub fn check_index() -> Result<String> {
    let current_count = get_note_count()?;

    match load_index() {
        Ok(index) => {
            let is_stale = current_count != index.note_count;
            Ok(serde_json::to_string_pretty(&json!({
                "success": true,
                "index_exists": true,
                "is_stale": is_stale,
                "current_note_count": current_count,
                "indexed_note_count": index.note_count,
                "last_updated": index.last_updated,
                "tag_count": index.tags.len()
            }))?)
        }
        Err(_) => {
            Ok(serde_json::to_string_pretty(&json!({
                "success": true,
                "index_exists": false,
                "is_stale": true,
                "current_note_count": current_count,
                "message": "Index not found. Run notes_index with action 'build' to create it."
            }))?)
        }
    }
}

/// List all tags from the index
pub fn list_tags() -> Result<String> {
    let index = load_index()?;

    // Sort tags by count (descending), then alphabetically
    let mut tag_list: Vec<(&String, usize)> = index
        .tags
        .iter()
        .map(|(tag, ids)| (tag, ids.len()))
        .collect();
    tag_list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let tags: Vec<Value> = tag_list
        .iter()
        .map(|(tag, count)| {
            json!({
                "tag": tag,
                "count": count
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "tag_count": tags.len(),
        "tags": tags
    }))?)
}

/// Search notes by tag
pub fn search_by_tag(tag: &str) -> Result<String> {
    let index = load_index()?;

    // Normalize tag (ensure it starts with #)
    let normalized_tag = if tag.starts_with('#') {
        tag.to_string()
    } else {
        format!("#{}", tag)
    };

    let note_ids = index.tags.get(&normalized_tag);

    let notes: Vec<Value> = match note_ids {
        Some(ids) => ids
            .iter()
            .filter_map(|id| index.notes.get(id))
            .map(|note| {
                json!({
                    "id": note.id,
                    "title": note.title,
                    "folder": note.folder,
                    "modified": note.modified,
                    "tags": note.tags,
                    "open_cmd": format!("osascript scripts/notes_open.applescript \"{}\"", note.id)
                })
            })
            .collect(),
        None => Vec::new(),
    };

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "tag": normalized_tag,
        "count": notes.len(),
        "notes": notes
    }))?)
}

// ============================================================================
// Tool Executor Integration
// ============================================================================

/// Main entry point for agent tool execution
pub fn execute_apple_notes(action: &str, args: &Value) -> Result<String> {
    match action {
        "search" => {
            // Check if searching by tag
            if let Some(tag) = args.get("tag").and_then(|v| v.as_str()) {
                return search_by_tag(tag);
            }
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
        "open" => {
            let id = args["id"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing required 'id' argument"))?;
            open_note(id)
        }
        // Tag index operations
        "index_build" => build_index(),
        "index_check" => check_index(),
        "tags" => list_tags(),
        "search_by_tag" => {
            let tag = args["tag"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing required 'tag' argument"))?;
            search_by_tag(tag)
        }
        // Semantic search operations (memvid-powered)
        "semantic_search" => {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing required 'query' argument"))?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            crate::memvid_notes::search_json(query, top_k)
        }
        "rebuild_memvid_index" => crate::memvid_notes::rebuild_index_json(),
        "memvid_stats" => crate::memvid_notes::stats_json(),
        "smart_search" => {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing required 'query' argument"))?;
            crate::memvid_notes::smart_search(query)
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

    #[test]
    fn test_is_css_color_code() {
        // 3-digit hex colors
        assert!(is_css_color_code("#fff"));
        assert!(is_css_color_code("#FFF"));
        assert!(is_css_color_code("#abc"));
        assert!(is_css_color_code("#123"));

        // 6-digit hex colors
        assert!(is_css_color_code("#ffffff"));
        assert!(is_css_color_code("#FFFFFF"));
        assert!(is_css_color_code("#dee2e6"));
        assert!(is_css_color_code("#e9ecef"));
        assert!(is_css_color_code("#000000"));

        // 8-digit hex colors (with alpha)
        assert!(is_css_color_code("#ffffffff"));
        assert!(is_css_color_code("#00000080"));

        // Not color codes - real tags
        assert!(!is_css_color_code("#project"));
        assert!(!is_css_color_code("#todo"));
        assert!(!is_css_color_code("#work"));
        assert!(!is_css_color_code("#meeting-notes"));

        // Edge cases
        assert!(!is_css_color_code("#ff")); // Too short
        assert!(!is_css_color_code("#ffff")); // 4 digits - not valid
        assert!(!is_css_color_code("#fffff")); // 5 digits - not valid
        assert!(!is_css_color_code("#fffffff")); // 7 digits - not valid
        assert!(!is_css_color_code("#fffffffff")); // 9 digits - too long
        assert!(!is_css_color_code("#ghijkl")); // Not hex
    }

    #[test]
    fn test_parse_index_filters_color_codes() {
        let output = r#"NOTE_COUNT: 1
RECORD_START
id: note-1
title: Test Note
folder: Notes
modified: 2026-01-27T10:00:00Z
tags: #project,#fff,#work,#dee2e6,#todo
RECORD_END"#;

        let (count, notes) = parse_index_output(output).unwrap();
        assert_eq!(count, 1);
        assert_eq!(notes.len(), 1);
        // Should filter out #fff and #dee2e6
        assert_eq!(notes[0].tags, vec!["#project", "#work", "#todo"]);
    }
}
