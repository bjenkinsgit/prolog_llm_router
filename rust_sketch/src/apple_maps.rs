//! Apple Maps Server API Client
//!
//! Handles JWT authentication and geocoding via Apple Maps Server API.
//! Requires a .p8 private key with MapKit JS capability from Apple Developer Portal.

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ============================================================================
// Configuration
// ============================================================================

/// Apple Maps configuration
#[derive(Debug, Clone)]
pub struct AppleMapsConfig {
    /// Apple Developer Team ID
    pub team_id: String,

    /// Maps ID (e.g., maps.com.example.app)
    pub maps_id: String,

    /// Key ID from the .p8 filename
    pub key_id: String,

    /// Path to the .p8 private key file
    pub private_key_path: String,
}

impl AppleMapsConfig {
    /// Create config from environment variables
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            team_id: std::env::var("APPLE_TEAM_ID")
                .context("Missing APPLE_TEAM_ID environment variable")?,
            maps_id: std::env::var("APPLE_MAPS_ID")
                .context("Missing APPLE_MAPS_ID environment variable")?,
            key_id: std::env::var("APPLE_MAPS_KEY")
                .context("Missing APPLE_MAPS_KEY environment variable")?,
            private_key_path: std::env::var("APPLE_MAPS_KEY_PATH")
                .context("Missing APPLE_MAPS_KEY_PATH environment variable")?,
        })
    }
}

// ============================================================================
// JWT Token Generation
// ============================================================================

/// JWT claims for Maps auth token
#[derive(Debug, Serialize)]
struct MapsAuthClaims {
    /// Issuer - Apple Developer Team ID
    iss: String,

    /// Issued at timestamp
    iat: u64,

    /// Expiration timestamp
    exp: u64,
}

/// Generate a Maps auth token (JWT) for exchanging to access token
fn generate_maps_auth_token(config: &AppleMapsConfig) -> Result<String> {
    let private_key_pem = fs::read_to_string(&config.private_key_path)
        .with_context(|| format!("Failed to read private key: {}", config.private_key_path))?;

    let encoding_key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
        .context("Failed to parse EC private key from PEM")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System time error")?
        .as_secs();

    let claims = MapsAuthClaims {
        iss: config.team_id.clone(),
        iat: now,
        exp: now + 3600, // 1 hour
    };

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(config.key_id.clone());
    header.typ = Some("JWT".to_string());

    encode(&header, &claims, &encoding_key).context("Failed to encode Maps auth JWT")
}

// ============================================================================
// Access Token Management
// ============================================================================

/// Cached access token with expiration tracking
struct CachedAccessToken {
    token: String,
    obtained_at: Instant,
}

impl CachedAccessToken {
    fn is_valid(&self) -> bool {
        // Refresh 5 minutes before expiry (tokens last 30 min)
        self.obtained_at.elapsed() < Duration::from_secs(25 * 60)
    }
}

/// Response from /v1/token endpoint
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    expires_in_seconds: u64,
}

// ============================================================================
// Geocoding Response Structures
// ============================================================================

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeocodeResponse {
    pub results: Vec<GeocodeResult>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeocodeResult {
    pub coordinate: Coordinate,
    pub display_map_region: Option<MapRegion>,
    pub name: String,
    #[serde(default)]
    pub formatted_address_lines: Vec<String>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct MapRegion {
    pub south_latitude: f64,
    pub west_longitude: f64,
    pub north_latitude: f64,
    pub east_longitude: f64,
}

// ============================================================================
// Apple Maps Client
// ============================================================================

/// Apple Maps Server API client with token caching
pub struct AppleMapsClient {
    config: AppleMapsConfig,
    client: Client,
    cached_token: Mutex<Option<CachedAccessToken>>,
}

impl AppleMapsClient {
    pub fn new(config: AppleMapsConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            config,
            client,
            cached_token: Mutex::new(None),
        })
    }

    /// Create client from environment variables
    pub fn from_env() -> Result<Self> {
        Self::new(AppleMapsConfig::from_env()?)
    }

    /// Get a valid access token, refreshing if necessary
    fn get_access_token(&self) -> Result<String> {
        let mut cached = self.cached_token.lock().unwrap();

        // Return cached token if still valid
        if let Some(ref token) = *cached {
            if token.is_valid() {
                return Ok(token.token.clone());
            }
        }

        // Generate new auth token and exchange for access token
        eprintln!("DEBUG: Obtaining new Apple Maps access token");
        let auth_token = generate_maps_auth_token(&self.config)?;

        let response = self
            .client
            .get("https://maps-api.apple.com/v1/token")
            .header("Authorization", format!("Bearer {}", auth_token))
            .send()
            .context("Failed to request Maps access token")?;

        let status = response.status();
        let body = response.text().context("Failed to read token response")?;

        if !status.is_success() {
            return Err(anyhow!("Maps token API error {}: {}", status, body));
        }

        let token_resp: TokenResponse =
            serde_json::from_str(&body).context("Failed to parse token response")?;

        // Cache the new token
        *cached = Some(CachedAccessToken {
            token: token_resp.access_token.clone(),
            obtained_at: Instant::now(),
        });

        Ok(token_resp.access_token)
    }

    /// Geocode a location string to coordinates
    pub fn geocode(&self, query: &str) -> Result<Coordinate> {
        let access_token = self.get_access_token()?;

        let url = format!(
            "https://maps-api.apple.com/v1/geocode?q={}",
            urlencoding::encode(query)
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .context("Geocode request failed")?;

        let status = response.status();
        let body = response.text().context("Failed to read geocode response")?;

        if !status.is_success() {
            return Err(anyhow!("Geocode API error {}: {}", status, body));
        }

        let geocode_resp: GeocodeResponse =
            serde_json::from_str(&body).context("Failed to parse geocode response")?;

        geocode_resp
            .results
            .first()
            .map(|r| r.coordinate)
            .ok_or_else(|| anyhow!("No geocoding results for '{}'", query))
    }

    /// Geocode and return full result with address info
    pub fn geocode_full(&self, query: &str) -> Result<GeocodeResult> {
        let access_token = self.get_access_token()?;

        let url = format!(
            "https://maps-api.apple.com/v1/geocode?q={}",
            urlencoding::encode(query)
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .context("Geocode request failed")?;

        let status = response.status();
        let body = response.text().context("Failed to read geocode response")?;

        if !status.is_success() {
            return Err(anyhow!("Geocode API error {}: {}", status, body));
        }

        let geocode_resp: GeocodeResponse =
            serde_json::from_str(&body).context("Failed to parse geocode response")?;

        geocode_resp
            .results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No geocoding results for '{}'", query))
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Check if Apple Maps is configured
pub fn is_configured() -> bool {
    std::env::var("APPLE_TEAM_ID").is_ok()
        && std::env::var("APPLE_MAPS_ID").is_ok()
        && std::env::var("APPLE_MAPS_KEY").is_ok()
        && std::env::var("APPLE_MAPS_KEY_PATH").is_ok()
}

/// Geocode a location string to (latitude, longitude)
pub fn geocode(query: &str) -> Result<(f64, f64)> {
    let client = AppleMapsClient::from_env()?;
    let coord = client.geocode(query)?;
    Ok((coord.latitude, coord.longitude))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_missing() {
        // This should fail if env vars aren't set
        std::env::remove_var("APPLE_MAPS_ID");
        let result = AppleMapsConfig::from_env();
        assert!(result.is_err());
    }

    #[test]
    fn test_is_configured_false() {
        std::env::remove_var("APPLE_MAPS_ID");
        assert!(!is_configured());
    }
}
