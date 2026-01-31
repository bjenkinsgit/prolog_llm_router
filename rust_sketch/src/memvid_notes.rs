//! Memvid-powered Notes Semantic Search
//!
//! Replaces the slow AppleScript-based Notes search with semantic BERT-based search
//! using memvid-rs. Notes content is indexed into a QR-encoded MP4 video with a
//! SQLite vector index for fast semantic retrieval.
//!
//! This module requires the `memvid` feature to be enabled:
//!   cargo build --features memvid
//!
//! FFmpeg must be installed on the system (brew install ffmpeg).
//!
//! Storage locations:
//! - ~/.cache/prolog-router/apple_notes.mp4       - QR-encoded note content
//! - ~/.cache/prolog-router/apple_notes_index.db  - SQLite vector index
//! - ~/.cache/prolog-router/apple_notes_meta.json - Sync metadata for staleness

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[cfg(feature = "memvid")]
use memvid_rs::{Config, MemvidEncoder, MemvidRetriever};
#[cfg(feature = "memvid")]
use memvid_rs::config::ErrorCorrectionLevel;

/// Convert string log level to ffmpeg_next::log::Level
#[cfg(feature = "memvid")]
fn parse_ffmpeg_log_level(level: &str) -> ffmpeg_next::log::Level {
    match level.to_lowercase().as_str() {
        "quiet" => ffmpeg_next::log::Level::Quiet,
        "panic" => ffmpeg_next::log::Level::Panic,
        "fatal" => ffmpeg_next::log::Level::Fatal,
        "error" => ffmpeg_next::log::Level::Error,
        "warning" => ffmpeg_next::log::Level::Warning,
        "info" => ffmpeg_next::log::Level::Info,
        "verbose" => ffmpeg_next::log::Level::Verbose,
        "debug" => ffmpeg_next::log::Level::Debug,
        "trace" => ffmpeg_next::log::Level::Trace,
        _ => ffmpeg_next::log::Level::Error, // Default to error
    }
}

/// Initialize FFmpeg with configurable log verbosity
#[cfg(feature = "memvid")]
fn init_ffmpeg_quiet() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let config = load_config();
        ffmpeg_next::init().ok();
        let level = parse_ffmpeg_log_level(&config.ffmpeg.library_log_level);
        ffmpeg_next::log::set_level(level);
    });
}

/// Get the FFmpeg config (for use by other modules)
pub fn get_ffmpeg_config() -> FfmpegConfig {
    load_config().ffmpeg
}

/// Get the full memvid config (for use by other modules)
pub fn get_full_config() -> MemvidConfig {
    load_config()
}

#[allow(unused_imports)]
use crate::apple_notes::{self, NoteContent};

#[cfg(feature = "memvid")]
use crate::apple_notes::IndexedNote;

// ============================================================================
// Runtime Configuration
// ============================================================================

/// Runtime configuration loaded from memvid_config.toml
#[derive(Debug, Clone, Deserialize)]
pub struct MemvidConfig {
    #[serde(default)]
    pub chunking: ChunkingConfig,
    #[serde(default)]
    pub ml: MlConfig,
    #[serde(default)]
    pub qr: QrConfig,
    #[serde(default)]
    pub metadata: MetadataConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub ffmpeg: FfmpegConfig,
}

/// FFmpeg logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct FfmpegConfig {
    /// FFmpeg library log level (swscaler, etc.)
    #[serde(default = "default_library_log_level")]
    pub library_log_level: String,
    /// FFmpeg CLI log level
    #[serde(default = "default_cli_log_level")]
    pub cli_log_level: String,
    /// Hide FFmpeg CLI banner
    #[serde(default = "default_hide_banner")]
    pub hide_banner: bool,
    /// x265 encoder log level
    #[serde(default = "default_x265_log_level")]
    pub x265_log_level: String,
}

fn default_library_log_level() -> String { "error".to_string() }
fn default_cli_log_level() -> String { "error".to_string() }
fn default_hide_banner() -> bool { true }
fn default_x265_log_level() -> String { "error".to_string() }

impl Default for FfmpegConfig {
    fn default() -> Self {
        Self {
            library_log_level: default_library_log_level(),
            cli_log_level: default_cli_log_level(),
            hide_banner: default_hide_banner(),
            x265_log_level: default_x265_log_level(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkingConfig {
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_overlap")]
    pub overlap: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MlConfig {
    #[serde(default = "default_device")]
    pub device: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QrConfig {
    #[serde(default = "default_error_correction")]
    pub error_correction: String,
    #[serde(default)]
    pub version: Option<i16>,
    #[serde(default = "default_enable_compression")]
    pub enable_compression: bool,
    #[serde(default = "default_compression_threshold")]
    pub compression_threshold: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetadataConfig {
    #[serde(default = "default_metadata_strategy")]
    pub strategy: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_enable_notes_cache")]
    pub enable_notes_cache: bool,
}

fn default_enable_compression() -> bool { true }
fn default_compression_threshold() -> usize { 100 }
fn default_enable_notes_cache() -> bool { true }

fn default_chunk_size() -> usize { 500 }
fn default_overlap() -> usize { 100 }
fn default_device() -> String { "metal".to_string() }
fn default_error_correction() -> String { "low".to_string() }
fn default_metadata_strategy() -> String { "indexed".to_string() }

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self { chunk_size: default_chunk_size(), overlap: default_overlap() }
    }
}

impl Default for MlConfig {
    fn default() -> Self { Self { device: default_device() } }
}

impl Default for QrConfig {
    fn default() -> Self {
        Self {
            error_correction: default_error_correction(),
            version: None,
            enable_compression: default_enable_compression(),
            compression_threshold: default_compression_threshold(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self { Self { enable_notes_cache: default_enable_notes_cache() } }
}

impl Default for MetadataConfig {
    fn default() -> Self { Self { strategy: default_metadata_strategy() } }
}

impl Default for MemvidConfig {
    fn default() -> Self {
        Self {
            chunking: ChunkingConfig::default(),
            ml: MlConfig::default(),
            qr: QrConfig::default(),
            metadata: MetadataConfig::default(),
            cache: CacheConfig::default(),
            ffmpeg: FfmpegConfig::default(),
        }
    }
}

/// Load config from memvid_config.toml (or use defaults)
fn load_config() -> MemvidConfig {
    let config_path = std::env::current_dir()
        .map(|p| p.join("memvid_config.toml"))
        .unwrap_or_else(|_| PathBuf::from("memvid_config.toml"));

    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            if let Ok(config) = toml::from_str(&content) {
                return config;
            } else {
                eprintln!("Warning: Failed to parse memvid_config.toml, using defaults");
            }
        }
    }
    MemvidConfig::default()
}

/// Note metadata stored in index file for recovery during search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMetadataEntry {
    pub note_id: String,
    pub title: String,
    pub folder: String,
    pub modified: String,
}

// ============================================================================
// File Paths
// ============================================================================

/// Directory name within cache for memvid files
const CACHE_SUBDIR: &str = "prolog-router";

/// Video file name (QR-encoded note content)
const VIDEO_FILE: &str = "apple_notes.mp4";

/// Index database file name (vector embeddings)
const INDEX_FILE: &str = "apple_notes_index.db";

/// Metadata file for staleness checking
const META_FILE: &str = "apple_notes_meta.json";

/// Note metadata index (maps chunk IDs to note info)
const NOTE_METADATA_FILE: &str = "apple_notes_metadata.json";

/// Cached notes content (for faster iteration when tuning parameters)
const NOTES_CACHE_FILE: &str = "apple_notes_cache.json";

// ============================================================================
// Data Structures
// ============================================================================

/// Result of a semantic search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotesSearchResult {
    /// Note ID (x-coredata://...) for opening in Notes.app
    pub note_id: String,
    /// Note title
    pub title: String,
    /// Folder containing the note
    pub folder: String,
    /// Relevant text snippet from the matched chunk
    pub snippet: String,
    /// Semantic similarity score (0.0 - 1.0)
    pub score: f32,
}

/// Sync metadata for staleness detection
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMetadata {
    /// Number of notes when index was last built
    note_count: usize,
    /// ISO 8601 timestamp of last sync
    last_updated: String,
}

/// Statistics about the memvid index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    /// Whether the index files exist
    pub exists: bool,
    /// Whether the index is stale (note count changed)
    pub is_stale: bool,
    /// Number of notes in the index
    pub indexed_note_count: usize,
    /// Current note count in Notes.app
    pub current_note_count: usize,
    /// When the index was last updated
    pub last_updated: String,
    /// Size of the video file in bytes
    pub video_size_bytes: u64,
    /// Size of the index database in bytes
    pub index_size_bytes: u64,
}

// ============================================================================
// Path Utilities
// ============================================================================

/// Get the cache directory for memvid files
fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CACHE_SUBDIR)
}

/// Get the path to the video file
fn video_path() -> PathBuf {
    cache_dir().join(VIDEO_FILE)
}

/// Get the path to the index database
fn index_path() -> PathBuf {
    cache_dir().join(INDEX_FILE)
}

/// Get the path to the metadata file
fn meta_path() -> PathBuf {
    cache_dir().join(META_FILE)
}

/// Get the path to the note metadata index
fn note_metadata_path() -> PathBuf {
    cache_dir().join(NOTE_METADATA_FILE)
}

/// Get the path to the notes content cache
fn notes_cache_path() -> PathBuf {
    cache_dir().join(NOTES_CACHE_FILE)
}

/// Ensure the cache directory exists
#[cfg(feature = "memvid")]
fn ensure_cache_dir() -> Result<()> {
    let dir = cache_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| anyhow!("Failed to create cache directory {:?}: {}", dir, e))?;
    }
    Ok(())
}

// ============================================================================
// Metadata Persistence
// ============================================================================

/// Load sync metadata from disk
fn load_metadata() -> Option<SyncMetadata> {
    let path = meta_path();
    if !path.exists() {
        return None;
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Save sync metadata to disk
#[cfg(feature = "memvid")]
fn save_metadata(meta: &SyncMetadata) -> Result<()> {
    ensure_cache_dir()?;
    let path = meta_path();
    let content = serde_json::to_string_pretty(meta)?;
    fs::write(&path, content).map_err(|e| anyhow!("Failed to write metadata: {}", e))
}

/// Save note metadata index (maps numeric index to note info)
#[cfg(feature = "memvid")]
fn save_note_metadata(metadata: &HashMap<u32, NoteMetadataEntry>) -> Result<()> {
    ensure_cache_dir()?;
    let path = note_metadata_path();
    let content = serde_json::to_string_pretty(metadata)?;
    fs::write(&path, content).map_err(|e| anyhow!("Failed to write note metadata: {}", e))
}

/// Load note metadata index from disk
fn load_note_metadata() -> Option<HashMap<u32, NoteMetadataEntry>> {
    let path = note_metadata_path();
    if !path.exists() {
        return None;
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Save notes content cache to disk
#[cfg(feature = "memvid")]
fn save_notes_cache(cache: &HashMap<String, NoteContent>) -> Result<()> {
    ensure_cache_dir()?;
    let path = notes_cache_path();
    let content = serde_json::to_string(cache)?;  // Compact for performance
    fs::write(&path, content).map_err(|e| anyhow!("Failed to write notes cache: {}", e))
}

/// Load notes content cache from disk
#[cfg(feature = "memvid")]
fn load_notes_cache() -> Option<HashMap<String, NoteContent>> {
    let path = notes_cache_path();
    if !path.exists() {
        return None;
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

// ============================================================================
// Note Content Fetching
// ============================================================================

/// Fetch note content via AppleScript (returns plaintext body)
#[cfg(feature = "memvid")]
#[allow(dead_code)]
fn fetch_note_content(note_id: &str) -> Result<NoteContent> {
    let json_result = apple_notes::get_note(note_id)?;
    let parsed: serde_json::Value = serde_json::from_str(&json_result)?;

    if parsed["success"].as_bool() != Some(true) {
        return Err(anyhow!("Failed to get note: {}", json_result));
    }

    let note = &parsed["note"];
    Ok(NoteContent {
        id: note["id"].as_str().unwrap_or("").to_string(),
        title: note["title"].as_str().unwrap_or("").to_string(),
        folder: note["folder"].as_str().unwrap_or("").to_string(),
        modified: note["modified"].as_str().unwrap_or("").to_string(),
        body: note["body"].as_str().unwrap_or("").to_string(),
        open_cmd: note["open_cmd"].as_str().unwrap_or("").to_string(),
    })
}

/// Batch fetch note contents via AppleScript (much faster than individual fetches)
/// Returns a HashMap of note_id -> NoteContent for successfully fetched notes
#[cfg(feature = "memvid")]
fn fetch_notes_batch(note_ids: &[String]) -> Result<std::collections::HashMap<String, NoteContent>> {
    use std::io::Write;
    use std::process::Command;

    // Write note IDs to a temp file
    let temp_dir = cache_dir();
    ensure_cache_dir()?;
    let ids_file = temp_dir.join("batch_note_ids.txt");

    {
        let mut file = fs::File::create(&ids_file)
            .map_err(|e| anyhow!("Failed to create temp file: {}", e))?;
        for id in note_ids {
            writeln!(file, "{}", id)
                .map_err(|e| anyhow!("Failed to write to temp file: {}", e))?;
        }
    }

    // Run the batch AppleScript
    let script_path = std::env::current_dir()?.join("scripts/notes_get_batch.applescript");

    let output = Command::new("osascript")
        .arg(&script_path)
        .arg(&ids_file)
        .output()
        .map_err(|e| anyhow!("Failed to execute batch AppleScript: {}", e))?;

    // Clean up temp file
    let _ = fs::remove_file(&ids_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("AppleScript failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check for global error
    if stdout.starts_with("ERROR:") {
        return Err(anyhow!("AppleScript error: {}", stdout));
    }

    // Parse the output - format is RECORD_START ... RECORD_END blocks
    let mut results = std::collections::HashMap::new();

    for record in stdout.split("RECORD_START") {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        // Remove RECORD_END if present
        let record = record.strip_suffix("RECORD_END").unwrap_or(record).trim();

        // Check for per-note error
        if record.starts_with("error:") {
            // Skip this note, it wasn't found
            continue;
        }

        // Parse fields
        let mut id = String::new();
        let mut title = String::new();
        let mut folder = String::new();
        let mut modified = String::new();
        let mut body = String::new();
        let mut in_body = false;

        for line in record.lines() {
            if line == "BODY_START" {
                in_body = true;
                continue;
            }
            if line == "BODY_END" {
                in_body = false;
                continue;
            }

            if in_body {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(line);
            } else if let Some(val) = line.strip_prefix("id: ") {
                id = val.to_string();
            } else if let Some(val) = line.strip_prefix("title: ") {
                title = val.to_string();
            } else if let Some(val) = line.strip_prefix("folder: ") {
                folder = val.to_string();
            } else if let Some(val) = line.strip_prefix("modified: ") {
                modified = val.to_string();
            }
        }

        if !id.is_empty() {
            results.insert(
                id.clone(),
                NoteContent {
                    id: id.clone(),
                    title,
                    folder,
                    modified,
                    body,
                    open_cmd: format!("osascript scripts/notes_open.applescript \"{}\"", id),
                },
            );
        }
    }

    Ok(results)
}

// ============================================================================
// Index Building (memvid feature required)
// ============================================================================

/// Build the memvid index from all Apple Notes
///
/// This reads all notes from SQLite (metadata) and AppleScript (content),
/// then encodes them into a searchable video file with semantic embeddings.
#[cfg(feature = "memvid")]
pub async fn build_index() -> Result<IndexStats> {
    // Suppress FFmpeg swscaler warnings
    init_ffmpeg_quiet();

    ensure_cache_dir()?;

    // Load runtime config
    let app_config = load_config();
    eprintln!("Config: chunk_size={}, overlap={}, device={}, error_correction={}, metadata={}",
        app_config.chunking.chunk_size,
        app_config.chunking.overlap,
        app_config.ml.device,
        app_config.qr.error_correction,
        app_config.metadata.strategy);

    eprint!("Loading notes from database... ");
    let index = apple_notes::load_index().or_else(|_| {
        // Index doesn't exist, build it first
        apple_notes::build_index()?;
        apple_notes::load_index()
    })?;
    eprintln!("{} notes found", index.note_count);

    // Collect all note IDs for batch fetching
    let notes: Vec<&IndexedNote> = index.notes.values().collect();
    let total = notes.len();
    let note_ids: Vec<String> = notes.iter().map(|n| n.id.clone()).collect();

    // Try to load from cache first if enabled
    let fetched_notes = if app_config.cache.enable_notes_cache {
        if let Some(cached) = load_notes_cache() {
            eprintln!("Using cached notes content ({} notes)", cached.len());
            cached
        } else {
            // Batch fetch all note contents in a single AppleScript call
            eprint!("Fetching all note contents (batch, will cache)... ");
            let fetched = fetch_notes_batch(&note_ids)?;
            eprintln!("done ({} fetched, {} missing)", fetched.len(), total - fetched.len());
            // Save to cache for next time
            if let Err(e) = save_notes_cache(&fetched) {
                eprintln!("Warning: Failed to save notes cache: {}", e);
            } else {
                eprintln!("Notes content cached to {:?}", notes_cache_path());
            }
            fetched
        }
    } else {
        // Batch fetch all note contents in a single AppleScript call
        eprint!("Fetching all note contents (batch)... ");
        let fetched = fetch_notes_batch(&note_ids)?;
        eprintln!("done ({} fetched, {} missing)", fetched.len(), total - fetched.len());
        fetched
    };

    // Create encoder with config-driven settings
    eprint!("Initializing memvid encoder (device={}, qr_version={:?})... ",
        app_config.ml.device,
        app_config.qr.version.unwrap_or(0));
    let mut config = Config::default();
    config.ml.device = app_config.ml.device.clone();
    config.chunking.chunk_size = app_config.chunking.chunk_size;
    config.chunking.overlap = app_config.chunking.overlap;
    config.chunking.max_chunk_size = app_config.chunking.chunk_size;
    // QR error correction level
    config.qr.error_correction = match app_config.qr.error_correction.to_lowercase().as_str() {
        "low" => ErrorCorrectionLevel::Low,
        "medium" => ErrorCorrectionLevel::Medium,
        "quartile" => ErrorCorrectionLevel::Quartile,
        "high" => ErrorCorrectionLevel::High,
        _ => ErrorCorrectionLevel::Low,
    };
    // QR version (1-40, 40 = largest capacity)
    config.qr.version = app_config.qr.version;
    // Compression settings
    config.qr.enable_compression = app_config.qr.enable_compression;
    config.qr.compression_threshold = app_config.qr.compression_threshold;
    let mut encoder = MemvidEncoder::new(Some(config))
        .await
        .map_err(|e| anyhow!("Failed to create encoder: {}", e))?;
    eprintln!("done");

    // Build note metadata index for the "indexed" strategy
    let use_indexed = app_config.metadata.strategy == "indexed";
    let mut note_metadata_map: HashMap<u32, NoteMetadataEntry> = HashMap::new();
    let chunk_size = app_config.chunking.chunk_size;
    let overlap = app_config.chunking.overlap;

    // Pre-chunk all notes manually to have full control over chunk size
    eprintln!("Chunking {} notes (chunk_size={}, overlap={})...", fetched_notes.len(), chunk_size, overlap);
    let mut all_chunks: Vec<String> = Vec::new();
    let mut note_idx: u32 = 0;

    for (i, note) in notes.iter().enumerate() {
        if (i + 1) % 50 == 0 || i + 1 == total {
            eprint!("\r  Processing note {}/{}", i + 1, total);
        }

        // Look up content from batch results
        if let Some(content) = fetched_notes.get(&note.id) {
            // Store metadata separately
            if use_indexed {
                let short_id = note.id.strip_prefix("x-coredata://").unwrap_or(&note.id).to_string();
                note_metadata_map.insert(note_idx, NoteMetadataEntry {
                    note_id: short_id,
                    title: note.title.clone(),
                    folder: note.folder.clone(),
                    modified: note.modified.clone(),
                });
            }

            // Manual chunking with short prefix
            let body = &content.body;
            let prefix = format!("N:{}\n", note_idx);
            let prefix_len = prefix.len();
            let effective_chunk_size = chunk_size.saturating_sub(prefix_len);

            if effective_chunk_size == 0 {
                eprintln!("\nWARNING: chunk_size {} too small for prefix", chunk_size);
                continue;
            }

            // Split body into chunks
            let body_chars: Vec<char> = body.chars().collect();
            let mut pos = 0;
            while pos < body_chars.len() {
                let end = (pos + effective_chunk_size).min(body_chars.len());
                let chunk_text: String = body_chars[pos..end].iter().collect();
                all_chunks.push(format!("{}{}", prefix, chunk_text));

                // Move forward with overlap
                pos += effective_chunk_size.saturating_sub(overlap);
                if pos >= end && end < body_chars.len() {
                    pos = end; // Prevent infinite loop
                }
            }

            // Handle empty notes
            if body.is_empty() {
                all_chunks.push(format!("{}(empty note)", prefix));
            }

            note_idx += 1;
        }
    }
    let encoded_count = note_idx as usize;
    eprintln!("\r  Processing note {}/{}... done ({} notes -> {} chunks)",
        total, total, encoded_count, all_chunks.len());

    // Add pre-chunked text to encoder using add_chunks()
    eprint!("Adding {} chunks to encoder... ", all_chunks.len());
    encoder
        .add_chunks(all_chunks)
        .map_err(|e| anyhow!("Failed to add chunks: {}", e))?;
    eprintln!("done");

    // Save note metadata index if using indexed strategy
    if use_indexed {
        eprint!("Saving note metadata index... ");
        save_note_metadata(&note_metadata_map)?;
        eprintln!("done ({} entries)", note_metadata_map.len());
    }

    // Build video and index files
    let vpath = video_path();
    let ipath = index_path();

    eprint!("Building video memory (this may take a while)... ");
    encoder
        .build_video(
            vpath.to_str().ok_or_else(|| anyhow!("Invalid video path"))?,
            ipath.to_str().ok_or_else(|| anyhow!("Invalid index path"))?,
        )
        .await
        .map_err(|e| anyhow!("Failed to build video: {}", e))?;
    eprintln!("done");

    // Save sync metadata
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let meta = SyncMetadata {
        note_count: encoded_count,
        last_updated: now.clone(),
    };
    save_metadata(&meta)?;

    // Get file sizes
    let video_size = fs::metadata(&vpath).map(|m| m.len()).unwrap_or(0);
    let index_size = fs::metadata(&ipath).map(|m| m.len()).unwrap_or(0);

    Ok(IndexStats {
        exists: true,
        is_stale: false,
        indexed_note_count: encoded_count,
        current_note_count: index.note_count,
        last_updated: now,
        video_size_bytes: video_size,
        index_size_bytes: index_size,
    })
}

// ============================================================================
// Staleness Detection
// ============================================================================

/// Check if the memvid index exists
#[allow(dead_code)]
pub fn index_exists() -> bool {
    video_path().exists() && index_path().exists()
}

/// Check if the index is stale (note count changed)
#[allow(dead_code)]
pub fn is_stale() -> Result<bool> {
    let meta = match load_metadata() {
        Some(m) => m,
        None => return Ok(true), // No metadata means stale
    };

    // Load current index to get note count
    let current_index = apple_notes::load_index()?;
    Ok(current_index.note_count != meta.note_count)
}

/// Get statistics about the memvid index
pub fn get_stats() -> Result<IndexStats> {
    let vpath = video_path();
    let ipath = index_path();

    let exists = vpath.exists() && ipath.exists();

    if !exists {
        // Try to get current note count
        let current_count = apple_notes::load_index()
            .map(|i| i.note_count)
            .unwrap_or(0);

        return Ok(IndexStats {
            exists: false,
            is_stale: true,
            indexed_note_count: 0,
            current_note_count: current_count,
            last_updated: String::new(),
            video_size_bytes: 0,
            index_size_bytes: 0,
        });
    }

    let meta = load_metadata().unwrap_or(SyncMetadata {
        note_count: 0,
        last_updated: String::new(),
    });

    let current_index = apple_notes::load_index()?;
    let stale = current_index.note_count != meta.note_count;

    let video_size = fs::metadata(&vpath).map(|m| m.len()).unwrap_or(0);
    let index_size = fs::metadata(&ipath).map(|m| m.len()).unwrap_or(0);

    Ok(IndexStats {
        exists: true,
        is_stale: stale,
        indexed_note_count: meta.note_count,
        current_note_count: current_index.note_count,
        last_updated: meta.last_updated,
        video_size_bytes: video_size,
        index_size_bytes: index_size,
    })
}

// ============================================================================
// Semantic Search (memvid feature required)
// ============================================================================

/// Parse note metadata from the beginning of a chunk text
/// Supports both indexed format ("N:123\n...") and inline format ("NOTE_ID: ...\n...")
/// Returns (note_id, title, folder, remaining_text)
#[allow(dead_code)]
fn parse_chunk_metadata(text: &str) -> (String, String, String, String) {
    // Check for indexed format first: "N:123\n..."
    if let Some(rest) = text.strip_prefix("N:") {
        if let Some(newline_pos) = rest.find('\n') {
            let idx_str = &rest[..newline_pos];
            if let Ok(idx) = idx_str.parse::<u32>() {
                // Look up metadata from index
                if let Some(metadata_map) = load_note_metadata() {
                    if let Some(entry) = metadata_map.get(&idx) {
                        let remaining = rest[newline_pos + 1..].to_string();
                        // Restore "x-coredata://" prefix if not already present
                        let full_note_id = if entry.note_id.starts_with("x-coredata://") {
                            entry.note_id.clone()
                        } else {
                            format!("x-coredata://{}", entry.note_id)
                        };
                        return (
                            full_note_id,
                            entry.title.clone(),
                            entry.folder.clone(),
                            remaining,
                        );
                    }
                }
            }
        }
    }

    // Fall back to inline format for backwards compatibility
    let mut note_id = String::new();
    let mut title = String::new();
    let mut folder = String::new();
    let mut remaining = text.to_string();

    for line in text.lines() {
        if let Some(id) = line.strip_prefix("NOTE_ID: ") {
            note_id = id.to_string();
        } else if let Some(t) = line.strip_prefix("TITLE: ") {
            title = t.to_string();
        } else if let Some(f) = line.strip_prefix("FOLDER: ") {
            folder = f.to_string();
        } else if line.strip_prefix("MODIFIED: ").is_some() {
            // Skip but don't break
        } else if !line.is_empty() {
            // Found content, rest is the body
            if let Some(idx) = text.find(line) {
                remaining = text[idx..].to_string();
            }
            break;
        }
    }

    (note_id, title, folder, remaining)
}

/// Create a snippet from text (first N characters with word boundary)
#[allow(dead_code)]
fn create_snippet(text: &str, max_len: usize) -> String {
    let text = text.trim();
    if text.len() <= max_len {
        return text.to_string();
    }

    // Find last space before max_len
    let truncated = &text[..max_len];
    if let Some(last_space) = truncated.rfind(' ') {
        format!("{}...", &text[..last_space])
    } else {
        format!("{}...", truncated)
    }
}

/// Perform semantic search against the memvid index
///
/// Returns notes ranked by semantic similarity to the query.
#[cfg(feature = "memvid")]
pub async fn search(query: &str, top_k: usize) -> Result<Vec<NotesSearchResult>> {
    // Suppress FFmpeg swscaler warnings
    init_ffmpeg_quiet();

    let vpath = video_path();
    let ipath = index_path();

    if !vpath.exists() || !ipath.exists() {
        return Err(anyhow!(
            "Index not found. Run notes_rebuild_index first to create the semantic index."
        ));
    }

    // Create retriever
    let mut retriever = MemvidRetriever::new(
        vpath.to_str().ok_or_else(|| anyhow!("Invalid video path"))?,
        ipath.to_str().ok_or_else(|| anyhow!("Invalid index path"))?,
    )
    .await
    .map_err(|e| anyhow!("Failed to create retriever: {}", e))?;

    // Perform search (returns Vec<(score, text)>)
    let raw_results = retriever
        .search(query, top_k)
        .await
        .map_err(|e| anyhow!("Search failed: {}", e))?;

    // Parse results and extract note metadata
    let mut results: Vec<NotesSearchResult> = Vec::new();
    let mut seen_notes: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (score, text) in raw_results {
        let (note_id, title, folder, content) = parse_chunk_metadata(&text);

        // Skip duplicates (same note may have multiple matching chunks)
        if note_id.is_empty() || seen_notes.contains(&note_id) {
            continue;
        }
        seen_notes.insert(note_id.clone());

        results.push(NotesSearchResult {
            note_id,
            title,
            folder,
            snippet: create_snippet(&content, 200),
            score,
        });
    }

    Ok(results)
}

// ============================================================================
// Sync Wrappers (for non-async callers)
// ============================================================================

/// Synchronous wrapper for build_index
#[cfg(feature = "memvid")]
pub fn build_index_sync() -> Result<IndexStats> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow!("Failed to create tokio runtime: {}", e))?;
    rt.block_on(build_index())
}

/// Synchronous wrapper for search
#[cfg(feature = "memvid")]
pub fn search_sync(query: &str, top_k: usize) -> Result<Vec<NotesSearchResult>> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow!("Failed to create tokio runtime: {}", e))?;
    rt.block_on(search(query, top_k))
}

// ============================================================================
// JSON Output for CLI/Agent
// ============================================================================

/// Build index and return JSON result
#[cfg(feature = "memvid")]
pub fn rebuild_index_json() -> Result<String> {
    let stats = build_index_sync()?;
    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "action": "rebuild",
        "indexed_note_count": stats.indexed_note_count,
        "video_size_bytes": stats.video_size_bytes,
        "index_size_bytes": stats.index_size_bytes,
        "last_updated": stats.last_updated,
        "video_path": video_path().to_string_lossy(),
        "index_path": index_path().to_string_lossy()
    }))?)
}

/// Stub for rebuild_index_json when memvid is disabled
#[cfg(not(feature = "memvid"))]
pub fn rebuild_index_json() -> Result<String> {
    Err(anyhow!(
        "Semantic search requires the 'memvid' feature. Build with: cargo build --features memvid\n\
         Note: FFmpeg must be installed (brew install ffmpeg)"
    ))
}

/// Get index stats as JSON
pub fn stats_json() -> Result<String> {
    let stats = get_stats()?;

    #[cfg(feature = "memvid")]
    let memvid_enabled = true;
    #[cfg(not(feature = "memvid"))]
    let memvid_enabled = false;

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "memvid_enabled": memvid_enabled,
        "exists": stats.exists,
        "is_stale": stats.is_stale,
        "indexed_note_count": stats.indexed_note_count,
        "current_note_count": stats.current_note_count,
        "last_updated": stats.last_updated,
        "video_size_bytes": stats.video_size_bytes,
        "index_size_bytes": stats.index_size_bytes,
        "video_path": video_path().to_string_lossy(),
        "index_path": index_path().to_string_lossy()
    }))?)
}

/// Semantic search and return JSON result
#[cfg(feature = "memvid")]
pub fn search_json(query: &str, top_k: usize) -> Result<String> {
    let results = search_sync(query, top_k)?;
    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "query": query,
        "count": results.len(),
        "results": results.iter().map(|r| json!({
            "note_id": r.note_id,
            "title": r.title,
            "folder": r.folder,
            "snippet": r.snippet,
            "score": r.score,
            "open_cmd": format!("osascript scripts/notes_open.applescript \"{}\"", r.note_id)
        })).collect::<Vec<_>>()
    }))?)
}

/// Stub for search_json when memvid is disabled
#[cfg(not(feature = "memvid"))]
pub fn search_json(_query: &str, _top_k: usize) -> Result<String> {
    Err(anyhow!(
        "Semantic search requires the 'memvid' feature. Build with: cargo build --features memvid\n\
         Note: FFmpeg must be installed (brew install ffmpeg)"
    ))
}

// ============================================================================
// Smart Search (auto-select best method)
// ============================================================================

/// Smart search: uses semantic search if available and index exists, falls back to AppleScript
#[cfg(feature = "memvid")]
pub fn smart_search(query: &str) -> Result<String> {
    if index_exists() {
        search_json(query, 10)
    } else {
        // Fall back to AppleScript search
        apple_notes::search_notes(query, None)
    }
}

/// Smart search fallback when memvid is disabled
#[cfg(not(feature = "memvid"))]
pub fn smart_search(query: &str) -> Result<String> {
    // Always use AppleScript when memvid is not available
    apple_notes::search_notes(query, None)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chunk_metadata() {
        let chunk = "NOTE_ID: x-coredata://123/ICNote/p456\nTITLE: Test Note\nFOLDER: Notes\nMODIFIED: 2026-01-27\n\nThis is the actual content of the note.";
        let (id, title, folder, content) = parse_chunk_metadata(chunk);
        assert_eq!(id, "x-coredata://123/ICNote/p456");
        assert_eq!(title, "Test Note");
        assert_eq!(folder, "Notes");
        assert!(content.contains("actual content"));
    }

    #[test]
    fn test_parse_chunk_metadata_no_metadata() {
        let chunk = "Just some plain text without metadata headers.";
        let (id, title, folder, content) = parse_chunk_metadata(chunk);
        assert!(id.is_empty());
        assert!(title.is_empty());
        assert!(folder.is_empty());
        // Content should be the original text
        assert!(content.contains("plain text"));
    }

    #[test]
    fn test_create_snippet() {
        let text = "This is a short text.";
        assert_eq!(create_snippet(text, 100), "This is a short text.");

        let long_text = "This is a much longer text that should be truncated at a word boundary.";
        let snippet = create_snippet(long_text, 30);
        assert!(snippet.len() <= 33); // 30 + "..."
        assert!(snippet.ends_with("..."));
    }

    #[test]
    fn test_cache_paths() {
        let vpath = video_path();
        let ipath = index_path();
        let mpath = meta_path();

        assert!(vpath.to_string_lossy().contains("apple_notes.mp4"));
        assert!(ipath.to_string_lossy().contains("apple_notes_index.db"));
        assert!(mpath.to_string_lossy().contains("apple_notes_meta.json"));
    }
}
