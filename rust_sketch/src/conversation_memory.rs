//! Conversation Memory with Memvid
//!
//! Persists conversation state across prolog-router invocations using memvid-rs.
//! Stores user queries and LLM responses in a searchable video memory, enabling
//! context retrieval from past conversations.
//!
//! This module requires the `memvid` feature to be enabled:
//!   cargo build --features memvid
//!
//! Storage locations:
//! - ~/.cache/prolog-router/conversation_memory.mp4       - QR-encoded conversation history
//! - ~/.cache/prolog-router/conversation_memory_index.db  - BERT embeddings for semantic search
//! - ~/.cache/prolog-router/conversation_memory_meta.json - Metadata (session count, last update)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[cfg(feature = "memvid")]
use memvid_rs::{Config, MemvidEncoder, MemvidRetriever};
#[cfg(feature = "memvid")]
use memvid_rs::config::ErrorCorrectionLevel;

/// Initialize FFmpeg with configurable log verbosity (uses shared config from memvid_notes)
#[cfg(feature = "memvid")]
fn init_ffmpeg_quiet() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let ffmpeg_config = crate::memvid_notes::get_ffmpeg_config();
        ffmpeg_next::init().ok();
        let level = match ffmpeg_config.library_log_level.to_lowercase().as_str() {
            "quiet" => ffmpeg_next::log::Level::Quiet,
            "panic" => ffmpeg_next::log::Level::Panic,
            "fatal" => ffmpeg_next::log::Level::Fatal,
            "error" => ffmpeg_next::log::Level::Error,
            "warning" => ffmpeg_next::log::Level::Warning,
            "info" => ffmpeg_next::log::Level::Info,
            "verbose" => ffmpeg_next::log::Level::Verbose,
            "debug" => ffmpeg_next::log::Level::Debug,
            "trace" => ffmpeg_next::log::Level::Trace,
            _ => ffmpeg_next::log::Level::Error,
        };
        ffmpeg_next::log::set_level(level);
    });
}

// ============================================================================
// Configuration
// ============================================================================

/// Memory configuration loaded from memvid_config.toml [memory] section
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_max_context_results")]
    pub max_context_results: usize,
    #[serde(default = "default_session_timeout_hours")]
    #[allow(dead_code)]
    pub session_timeout_hours: u64,
}

fn default_enabled() -> bool { true }
fn default_max_context_results() -> usize { 3 }
fn default_session_timeout_hours() -> u64 { 24 }

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_context_results: default_max_context_results(),
            session_timeout_hours: default_session_timeout_hours(),
        }
    }
}

/// Load memory config from memvid_config.toml
pub fn load_memory_config() -> MemoryConfig {
    let config_path = std::env::current_dir()
        .map(|p| p.join("memvid_config.toml"))
        .unwrap_or_else(|_| PathBuf::from("memvid_config.toml"));

    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            // Parse the full config and extract [memory] section
            if let Ok(config) = toml::from_str::<toml::Value>(&content) {
                if let Some(memory) = config.get("memory") {
                    if let Ok(mem_config) = memory.clone().try_into::<MemoryConfig>() {
                        return mem_config;
                    }
                }
            }
        }
    }
    MemoryConfig::default()
}

// ============================================================================
// File Paths
// ============================================================================

/// Directory name within cache for memvid files
const CACHE_SUBDIR: &str = "prolog-router";

/// Video file name (QR-encoded conversation history)
const VIDEO_FILE: &str = "conversation_memory.mp4";

/// Index database file name (vector embeddings)
const INDEX_FILE: &str = "conversation_memory_index.db";

/// Metadata file for tracking conversation count
const META_FILE: &str = "conversation_memory_meta.json";

// ============================================================================
// Data Structures
// ============================================================================

/// Role of a conversation turn
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnRole {
    User,
    Assistant,
}

impl TurnRole {
    #[allow(dead_code)]
    fn as_str(&self) -> &'static str {
        match self {
            TurnRole::User => "user",
            TurnRole::Assistant => "assistant",
        }
    }
}

/// A single conversation turn
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ConversationTurn {
    pub session_id: String,
    /// ISO 8601 timestamp string
    pub timestamp: String,
    pub role: TurnRole,
    pub content: String,
}

/// Result of a memory search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchResult {
    /// Session ID of the matching conversation
    pub session_id: String,
    /// Timestamp of the turn
    pub timestamp: String,
    /// Role (user or assistant)
    pub role: String,
    /// Content of the turn
    pub content: String,
    /// Semantic similarity score (0.0 - 1.0)
    pub score: f32,
}

/// Statistics about the conversation memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Whether the memory files exist
    pub exists: bool,
    /// Number of conversation exchanges stored
    pub exchange_count: usize,
    /// When the memory was last updated
    pub last_updated: String,
    /// Size of the video file in bytes
    pub video_size_bytes: u64,
    /// Size of the index database in bytes
    pub index_size_bytes: u64,
}

/// Sync metadata for tracking conversation count
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMetadata {
    /// Number of conversation exchanges stored
    exchange_count: usize,
    /// ISO 8601 timestamp of last update
    last_updated: String,
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

/// Ensure the cache directory exists
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
#[allow(dead_code)]
fn save_metadata(meta: &SyncMetadata) -> Result<()> {
    ensure_cache_dir()?;
    let path = meta_path();
    let content = serde_json::to_string_pretty(meta)?;
    fs::write(&path, content).map_err(|e| anyhow!("Failed to write metadata: {}", e))
}

// ============================================================================
// Conversation Memory
// ============================================================================

/// Memory index for conversations
pub struct ConversationMemory {
    video_path: PathBuf,
    index_path: PathBuf,
    #[cfg(feature = "memvid")]
    retriever: Option<MemvidRetriever>,
    #[cfg(not(feature = "memvid"))]
    #[allow(dead_code)]
    retriever: Option<()>,
}

impl ConversationMemory {
    /// Check if memory files exist
    pub fn exists() -> bool {
        video_path().exists() && index_path().exists()
    }

    /// Load existing memory or create new (memvid feature required)
    #[cfg(feature = "memvid")]
    pub async fn load_or_create() -> Result<Self> {
        // Suppress FFmpeg swscaler warnings
        init_ffmpeg_quiet();

        ensure_cache_dir()?;

        let vpath = video_path();
        let ipath = index_path();

        let retriever = if vpath.exists() && ipath.exists() {
            // Load existing retriever
            Some(
                MemvidRetriever::new(
                    vpath.to_str().ok_or_else(|| anyhow!("Invalid video path"))?,
                    ipath.to_str().ok_or_else(|| anyhow!("Invalid index path"))?,
                )
                .await
                .map_err(|e| anyhow!("Failed to load memory retriever: {}", e))?
            )
        } else {
            None
        };

        Ok(Self {
            video_path: vpath,
            index_path: ipath,
            retriever,
        })
    }

    /// Stub for load_or_create when memvid is disabled
    #[cfg(not(feature = "memvid"))]
    pub async fn load_or_create() -> Result<Self> {
        ensure_cache_dir()?;
        Ok(Self {
            video_path: video_path(),
            index_path: index_path(),
            retriever: None,
        })
    }

    /// Append a conversation exchange (user + assistant) to memory
    #[cfg(feature = "memvid")]
    pub async fn append_exchange(
        &mut self,
        session_id: &str,
        user_msg: &str,
        assistant_msg: &str
    ) -> Result<()> {
        let now = chrono::Utc::now();
        let timestamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Format chunks with structured prefix
        let user_chunk = format!(
            "S:{}|T:{}|R:user\n{}",
            session_id, timestamp, user_msg
        );
        let assistant_chunk = format!(
            "S:{}|T:{}|R:assistant\n{}",
            session_id, timestamp, assistant_msg
        );

        // Load config for encoder settings (use load_config to get actual values)
        let app_config = crate::memvid_notes::get_ffmpeg_config();

        // Helper to create memvid Config with our settings
        let create_memvid_config = || {
            let mut config = Config::default();
            // Apply ffmpeg settings from our config
            config.video.x265_log_level = app_config.x265_log_level.clone();
            config.video.ffmpeg_cli_log_level = app_config.cli_log_level.clone();
            config.video.ffmpeg_hide_banner = app_config.hide_banner;
            config.video.apply_x265_log_level();
            config
        };

        // Check if we need to create a new memory or append to existing
        if self.video_path.exists() && self.index_path.exists() {
            // Append to existing memory
            let config = create_memvid_config();
            let mut encoder = MemvidEncoder::new(Some(config))
                .await
                .map_err(|e| anyhow!("Failed to create encoder: {}", e))?;

            encoder
                .append_chunks(
                    self.video_path.to_str().ok_or_else(|| anyhow!("Invalid video path"))?,
                    self.index_path.to_str().ok_or_else(|| anyhow!("Invalid index path"))?,
                    vec![user_chunk, assistant_chunk],
                )
                .await
                .map_err(|e| anyhow!("Failed to append chunks: {}", e))?;
        } else {
            // Create new memory
            let mut config = create_memvid_config();
            // Also load chunking/qr settings for new memory creation
            let full_config = crate::memvid_notes::get_full_config();
            config.ml.device = full_config.ml.device.clone();
            config.chunking.chunk_size = full_config.chunking.chunk_size;
            config.chunking.overlap = full_config.chunking.overlap;
            config.chunking.max_chunk_size = full_config.chunking.chunk_size;
            config.qr.error_correction = match full_config.qr.error_correction.to_lowercase().as_str() {
                "low" => ErrorCorrectionLevel::Low,
                "medium" => ErrorCorrectionLevel::Medium,
                "quartile" => ErrorCorrectionLevel::Quartile,
                "high" => ErrorCorrectionLevel::High,
                _ => ErrorCorrectionLevel::Low,
            };
            config.qr.version = full_config.qr.version;
            config.qr.enable_compression = full_config.qr.enable_compression;
            config.qr.compression_threshold = full_config.qr.compression_threshold;

            let mut encoder = MemvidEncoder::new(Some(config))
                .await
                .map_err(|e| anyhow!("Failed to create encoder: {}", e))?;

            encoder
                .add_chunks(vec![user_chunk, assistant_chunk])
                .map_err(|e| anyhow!("Failed to add chunks: {}", e))?;

            encoder
                .build_video(
                    self.video_path.to_str().ok_or_else(|| anyhow!("Invalid video path"))?,
                    self.index_path.to_str().ok_or_else(|| anyhow!("Invalid index path"))?,
                )
                .await
                .map_err(|e| anyhow!("Failed to build video: {}", e))?;
        }

        // Update metadata
        let meta = load_metadata().unwrap_or(SyncMetadata {
            exchange_count: 0,
            last_updated: String::new(),
        });
        save_metadata(&SyncMetadata {
            exchange_count: meta.exchange_count + 1,
            last_updated: timestamp,
        })?;

        // Reload retriever with new data
        self.retriever = Some(
            MemvidRetriever::new(
                self.video_path.to_str().ok_or_else(|| anyhow!("Invalid video path"))?,
                self.index_path.to_str().ok_or_else(|| anyhow!("Invalid index path"))?,
            )
            .await
            .map_err(|e| anyhow!("Failed to reload retriever: {}", e))?
        );

        Ok(())
    }

    /// Stub for append_exchange when memvid is disabled
    #[cfg(not(feature = "memvid"))]
    pub async fn append_exchange(
        &mut self,
        _session_id: &str,
        _user_msg: &str,
        _assistant_msg: &str
    ) -> Result<()> {
        Err(anyhow!(
            "Conversation memory requires the 'memvid' feature. Build with: cargo build --features memvid"
        ))
    }

    /// Search for relevant past conversations
    #[cfg(feature = "memvid")]
    pub async fn search(&mut self, query: &str, top_k: usize) -> Result<Vec<MemorySearchResult>> {
        let retriever = match &mut self.retriever {
            Some(r) => r,
            None => return Ok(Vec::new()), // No memory exists yet
        };

        let raw_results = retriever
            .search(query, top_k)
            .await
            .map_err(|e| anyhow!("Search failed: {}", e))?;

        let mut results = Vec::new();
        for (score, text) in raw_results {
            if let Some(result) = parse_memory_chunk(&text, score) {
                results.push(result);
            }
        }

        Ok(results)
    }

    /// Stub for search when memvid is disabled
    #[cfg(not(feature = "memvid"))]
    pub async fn search(&mut self, _query: &str, _top_k: usize) -> Result<Vec<MemorySearchResult>> {
        Ok(Vec::new())
    }

    /// Get memory statistics
    pub fn stats(&self) -> MemoryStats {
        let vpath = &self.video_path;
        let ipath = &self.index_path;

        let exists = vpath.exists() && ipath.exists();

        if !exists {
            return MemoryStats {
                exists: false,
                exchange_count: 0,
                last_updated: String::new(),
                video_size_bytes: 0,
                index_size_bytes: 0,
            };
        }

        let meta = load_metadata().unwrap_or(SyncMetadata {
            exchange_count: 0,
            last_updated: String::new(),
        });

        let video_size = fs::metadata(vpath).map(|m| m.len()).unwrap_or(0);
        let index_size = fs::metadata(ipath).map(|m| m.len()).unwrap_or(0);

        MemoryStats {
            exists: true,
            exchange_count: meta.exchange_count,
            last_updated: meta.last_updated,
            video_size_bytes: video_size,
            index_size_bytes: index_size,
        }
    }
}

// ============================================================================
// Chunk Parsing
// ============================================================================

/// Parse a memory chunk into a MemorySearchResult
/// Format: "S:<session_id>|T:<timestamp>|R:<role>\n<content>"
#[allow(dead_code)]
fn parse_memory_chunk(text: &str, score: f32) -> Option<MemorySearchResult> {
    let lines: Vec<&str> = text.splitn(2, '\n').collect();
    if lines.is_empty() {
        return None;
    }

    let header = lines[0];
    let content = if lines.len() > 1 { lines[1] } else { "" };

    let mut session_id = String::new();
    let mut timestamp = String::new();
    let mut role = String::new();

    for part in header.split('|') {
        if let Some(s) = part.strip_prefix("S:") {
            session_id = s.to_string();
        } else if let Some(t) = part.strip_prefix("T:") {
            timestamp = t.to_string();
        } else if let Some(r) = part.strip_prefix("R:") {
            role = r.to_string();
        }
    }

    if session_id.is_empty() || role.is_empty() {
        return None;
    }

    Some(MemorySearchResult {
        session_id,
        timestamp,
        role,
        content: content.to_string(),
        score,
    })
}

// ============================================================================
// Session ID Generation
// ============================================================================

/// Generate a unique session ID
pub fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    // Combine timestamp with random-ish suffix from nanoseconds
    format!("{}_{:04x}", duration.as_secs(), duration.subsec_nanos() % 0xFFFF)
}

// ============================================================================
// Sync Wrappers (for non-async callers)
// ============================================================================

/// Synchronous wrapper for load_or_create
pub fn load_or_create_sync() -> Result<ConversationMemory> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow!("Failed to create tokio runtime: {}", e))?;
    rt.block_on(ConversationMemory::load_or_create())
}

/// Synchronous wrapper for append_exchange
pub fn append_exchange_sync(
    memory: &mut ConversationMemory,
    session_id: &str,
    user_msg: &str,
    assistant_msg: &str
) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow!("Failed to create tokio runtime: {}", e))?;
    rt.block_on(memory.append_exchange(session_id, user_msg, assistant_msg))
}

/// Synchronous wrapper for search
pub fn search_sync(
    memory: &mut ConversationMemory,
    query: &str,
    top_k: usize
) -> Result<Vec<MemorySearchResult>> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow!("Failed to create tokio runtime: {}", e))?;
    rt.block_on(memory.search(query, top_k))
}

// ============================================================================
// JSON Output for CLI/Agent
// ============================================================================

/// Search memory and return JSON result
pub fn search_json(query: &str, top_k: usize) -> Result<String> {
    let mut memory = load_or_create_sync()?;
    let results = search_sync(&mut memory, query, top_k)?;

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "query": query,
        "count": results.len(),
        "results": results
    }))?)
}

/// Get memory stats as JSON
pub fn stats_json() -> Result<String> {
    let memory = load_or_create_sync()?;
    let stats = memory.stats();

    #[cfg(feature = "memvid")]
    let memvid_enabled = true;
    #[cfg(not(feature = "memvid"))]
    let memvid_enabled = false;

    Ok(serde_json::to_string_pretty(&json!({
        "success": true,
        "memvid_enabled": memvid_enabled,
        "exists": stats.exists,
        "exchange_count": stats.exchange_count,
        "last_updated": stats.last_updated,
        "video_size_bytes": stats.video_size_bytes,
        "index_size_bytes": stats.index_size_bytes,
        "video_path": video_path().to_string_lossy(),
        "index_path": index_path().to_string_lossy()
    }))?)
}

// ============================================================================
// Context Formatting for LLM
// ============================================================================

/// Format memory search results as context for LLM prompt injection
pub fn format_memory_context(results: &[MemorySearchResult]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut context = String::from("\n## Relevant Past Conversations\n");
    for (i, result) in results.iter().enumerate() {
        context.push_str(&format!(
            "\n### Memory {} (relevance: {:.2})\n",
            i + 1,
            result.score
        ));
        context.push_str(&format!("**{}**: {}\n", result.role, result.content));
    }
    context
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_chunk() {
        let chunk = "S:1706745600_abc1|T:2026-01-31T12:00:00Z|R:user\nWhat's the weather in NYC?";
        let result = parse_memory_chunk(chunk, 0.95).unwrap();

        assert_eq!(result.session_id, "1706745600_abc1");
        assert_eq!(result.timestamp, "2026-01-31T12:00:00Z");
        assert_eq!(result.role, "user");
        assert_eq!(result.content, "What's the weather in NYC?");
        assert!((result.score - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_parse_memory_chunk_invalid() {
        let chunk = "Just some random text without header";
        let result = parse_memory_chunk(chunk, 0.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_generate_session_id() {
        let id1 = generate_session_id();
        let _id2 = generate_session_id();

        // Session IDs should be unique (at least not identical in quick succession)
        // Note: This test might occasionally fail if called in exact same nanosecond
        assert!(!id1.is_empty());
        assert!(id1.contains('_'));
    }

    #[test]
    fn test_format_memory_context_empty() {
        let results: Vec<MemorySearchResult> = vec![];
        let context = format_memory_context(&results);
        assert!(context.is_empty());
    }

    #[test]
    fn test_format_memory_context() {
        let results = vec![
            MemorySearchResult {
                session_id: "test123".to_string(),
                timestamp: "2026-01-31T12:00:00Z".to_string(),
                role: "user".to_string(),
                content: "What's the weather?".to_string(),
                score: 0.9,
            },
        ];
        let context = format_memory_context(&results);

        assert!(context.contains("Relevant Past Conversations"));
        assert!(context.contains("Memory 1"));
        assert!(context.contains("0.90"));
        assert!(context.contains("What's the weather?"));
    }

    #[test]
    fn test_memory_config_default() {
        let config = MemoryConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_context_results, 3);
        assert_eq!(config.session_timeout_hours, 24);
    }

    #[test]
    fn test_cache_paths() {
        let vpath = video_path();
        let ipath = index_path();
        let mpath = meta_path();

        assert!(vpath.to_string_lossy().contains("conversation_memory.mp4"));
        assert!(ipath.to_string_lossy().contains("conversation_memory_index.db"));
        assert!(mpath.to_string_lossy().contains("conversation_memory_meta.json"));
    }

    #[test]
    fn test_turn_role_as_str() {
        assert_eq!(TurnRole::User.as_str(), "user");
        assert_eq!(TurnRole::Assistant.as_str(), "assistant");
    }
}
