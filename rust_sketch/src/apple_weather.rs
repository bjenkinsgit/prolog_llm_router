//! Apple WeatherKit REST API Client
//!
//! Handles JWT authentication and weather data fetching from Apple's WeatherKit API.
//! Requires a .p8 private key from Apple Developer Portal with WeatherKit capability.

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Temperature Unit Conversion
// ============================================================================

/// Temperature unit preference
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

impl TemperatureUnit {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "f" | "fahrenheit" => TemperatureUnit::Fahrenheit,
            _ => TemperatureUnit::Celsius,
        }
    }

    /// Get from TEMPERATURE_UNIT environment variable
    pub fn from_env() -> Self {
        std::env::var("TEMPERATURE_UNIT")
            .map(|s| Self::from_str(&s))
            .unwrap_or_default()
    }

    /// Unit suffix for display
    pub fn suffix(&self) -> &'static str {
        match self {
            TemperatureUnit::Celsius => "C",
            TemperatureUnit::Fahrenheit => "F",
        }
    }
}

/// Convert Celsius to Fahrenheit
pub fn celsius_to_fahrenheit(celsius: f64) -> f64 {
    celsius * 9.0 / 5.0 + 32.0
}

/// Convert temperature based on unit preference
pub fn convert_temp(celsius: f64, unit: TemperatureUnit) -> f64 {
    match unit {
        TemperatureUnit::Celsius => celsius,
        TemperatureUnit::Fahrenheit => celsius_to_fahrenheit(celsius),
    }
}

// ============================================================================
// Configuration
// ============================================================================

/// Apple WeatherKit configuration
#[derive(Debug, Clone)]
pub struct WeatherKitConfig {
    /// Apple Developer Team ID (10 characters)
    pub team_id: String,

    /// WeatherKit Service ID (bundle identifier)
    pub service_id: String,

    /// Key ID from the .p8 filename (e.g., CX7YXVQARR from AuthKey_CX7YXVQARR.p8)
    pub key_id: String,

    /// Path to the .p8 private key file
    pub private_key_path: String,
}

impl WeatherKitConfig {
    /// Create config from environment variables
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            team_id: std::env::var("APPLE_TEAM_ID")
                .context("Missing APPLE_TEAM_ID environment variable")?,
            service_id: std::env::var("APPLE_SERVICE_ID")
                .context("Missing APPLE_SERVICE_ID environment variable")?,
            key_id: std::env::var("APPLE_KEY_ID")
                .context("Missing APPLE_KEY_ID environment variable")?,
            private_key_path: std::env::var("APPLE_PRIVATE_KEY_PATH")
                .context("Missing APPLE_PRIVATE_KEY_PATH environment variable")?,
        })
    }

    /// Create config with explicit values
    pub fn new(team_id: &str, service_id: &str, key_id: &str, private_key_path: &str) -> Self {
        Self {
            team_id: team_id.to_string(),
            service_id: service_id.to_string(),
            key_id: key_id.to_string(),
            private_key_path: private_key_path.to_string(),
        }
    }
}

// ============================================================================
// JWT Token Generation
// ============================================================================

/// JWT claims for WeatherKit authentication
#[derive(Debug, Serialize)]
struct WeatherKitClaims {
    /// Issuer - Apple Developer Team ID
    iss: String,

    /// Issued at timestamp
    iat: u64,

    /// Expiration timestamp (max 1 hour from iat)
    exp: u64,

    /// Subject - WeatherKit Service ID
    sub: String,
}

/// Generate JWT for WeatherKit API authentication
pub fn generate_jwt(config: &WeatherKitConfig) -> Result<String> {
    let private_key_pem = fs::read_to_string(&config.private_key_path)
        .with_context(|| format!("Failed to read private key: {}", config.private_key_path))?;

    let encoding_key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
        .context("Failed to parse EC private key from PEM")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System time error")?
        .as_secs();

    let claims = WeatherKitClaims {
        iss: config.team_id.clone(),
        iat: now,
        exp: now + 3600,
        sub: config.service_id.clone(),
    };

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(config.key_id.clone());

    encode(&header, &claims, &encoding_key).context("Failed to encode JWT")
}

// ============================================================================
// WeatherKit API Client
// ============================================================================

/// WeatherKit API response structures
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherResponse {
    pub current_weather: Option<CurrentWeather>,
    #[serde(default)]
    pub forecast_daily: Option<DailyForecast>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CurrentWeather {
    pub temperature: f64,
    pub temperature_apparent: f64,
    pub condition_code: String,
    pub humidity: f64,
    pub wind_speed: f64,
    #[serde(default)]
    pub uv_index: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyForecast {
    pub days: Vec<DayWeather>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct DayWeather {
    pub forecast_start: String,
    pub condition_code: String,
    pub temperature_max: f64,
    pub temperature_min: f64,
    #[serde(default)]
    pub precipitation_chance: f64,
}

/// Apple WeatherKit client
pub struct WeatherKitClient {
    config: WeatherKitConfig,
    client: Client,
}

impl WeatherKitClient {
    pub fn new(config: WeatherKitConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { config, client })
    }

    /// Create client from environment variables
    pub fn from_env() -> Result<Self> {
        Self::new(WeatherKitConfig::from_env()?)
    }

    /// Generate a fresh JWT token
    fn get_token(&self) -> Result<String> {
        generate_jwt(&self.config)
    }

    /// Get weather for a location by coordinates
    pub fn get_weather(&self, lat: f64, lon: f64, language: &str) -> Result<WeatherResponse> {
        let token = self.get_token()?;

        let url = format!(
            "https://weatherkit.apple.com/api/v1/weather/{}/{}/{}?dataSets=currentWeather,forecastDaily",
            language, lat, lon
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .context("WeatherKit API request failed")?;

        let status = response.status();
        let body = response.text().context("Failed to read response")?;

        if !status.is_success() {
            return Err(anyhow!("WeatherKit API error {}: {}", status, body));
        }

        serde_json::from_str(&body).context("Failed to parse WeatherKit response")
    }

    /// Get weather by city name (uses geocoding first)
    /// For simplicity, this uses a basic lookup - in production you'd use Apple Maps or similar
    pub fn get_weather_by_city(&self, city: &str) -> Result<String> {
        // Simple geocoding lookup for common cities
        // In production, use Apple Maps Geocoding API
        let (lat, lon) = geocode_city(city)?;

        let weather = self.get_weather(lat, lon, "en")?;

        // Get temperature unit preference from environment
        let unit = TemperatureUnit::from_env();
        let suffix = unit.suffix();

        // Format response
        if let Some(current) = weather.current_weather {
            let temp = convert_temp(current.temperature, unit);
            let feels_like = convert_temp(current.temperature_apparent, unit);

            Ok(format!(
                "{}: {:.1}{} (feels like {:.1}{}), {}, humidity {:.0}%, wind {:.1} km/h",
                city,
                temp,
                suffix,
                feels_like,
                suffix,
                format_condition(&current.condition_code),
                current.humidity * 100.0,
                current.wind_speed
            ))
        } else if let Some(forecast) = weather.forecast_daily {
            if let Some(day) = forecast.days.first() {
                let high = convert_temp(day.temperature_max, unit);
                let low = convert_temp(day.temperature_min, unit);

                Ok(format!(
                    "{}: {} with high of {:.1}{}, low of {:.1}{}, {:.0}% chance of precipitation",
                    city,
                    format_condition(&day.condition_code),
                    high,
                    suffix,
                    low,
                    suffix,
                    day.precipitation_chance * 100.0
                ))
            } else {
                Err(anyhow!("No forecast data available"))
            }
        } else {
            Err(anyhow!("No weather data in response"))
        }
    }
}

/// Format Apple's condition codes to human-readable strings
fn format_condition(code: &str) -> &str {
    match code {
        "Clear" => "clear",
        "Cloudy" => "cloudy",
        "MostlyClear" => "mostly clear",
        "MostlyCloudy" => "mostly cloudy",
        "PartlyCloudy" => "partly cloudy",
        "Rain" => "rain",
        "Drizzle" => "drizzle",
        "HeavyRain" => "heavy rain",
        "Snow" => "snow",
        "Flurries" => "flurries",
        "HeavySnow" => "heavy snow",
        "Sleet" => "sleet",
        "FreezingRain" => "freezing rain",
        "Thunderstorms" => "thunderstorms",
        "Windy" => "windy",
        "Foggy" => "foggy",
        "Haze" => "haze",
        "Hot" => "hot",
        "Cold" => "cold",
        _ => code,
    }
}

/// Simple geocoding for common cities
/// In production, use Apple Maps Geocoding API or similar service
fn geocode_city(city: &str) -> Result<(f64, f64)> {
    let city_lower = city.to_lowercase();

    // Common cities lookup table
    let coords = match city_lower.as_str() {
        "new york" | "nyc" | "new york city" => (40.7128, -74.0060),
        "los angeles" | "la" => (34.0522, -118.2437),
        "chicago" => (41.8781, -87.6298),
        "houston" => (29.7604, -95.3698),
        "phoenix" => (33.4484, -112.0740),
        "philadelphia" => (39.9526, -75.1652),
        "san antonio" => (29.4241, -98.4936),
        "san diego" => (32.7157, -117.1611),
        "dallas" => (32.7767, -96.7970),
        "san jose" => (37.3382, -121.8863),
        "austin" => (30.2672, -97.7431),
        "seattle" => (47.6062, -122.3321),
        "denver" => (39.7392, -104.9903),
        "boston" => (42.3601, -71.0589),
        "san francisco" | "sf" => (37.7749, -122.4194),
        "miami" => (25.7617, -80.1918),
        "atlanta" => (33.7490, -84.3880),
        "portland" => (45.5152, -122.6784),
        "las vegas" => (36.1699, -115.1398),
        "detroit" => (42.3314, -83.0458),
        "minneapolis" => (44.9778, -93.2650),
        "london" => (51.5074, -0.1278),
        "paris" => (48.8566, 2.3522),
        "tokyo" => (35.6762, 139.6503),
        "sydney" => (-33.8688, 151.2093),
        "toronto" => (43.6532, -79.3832),
        "berlin" => (52.5200, 13.4050),
        "madrid" => (40.4168, -3.7038),
        "rome" => (41.9028, 12.4964),
        "amsterdam" => (52.3676, 4.9041),
        "singapore" => (1.3521, 103.8198),
        "hong kong" => (22.3193, 114.1694),
        "seoul" => (37.5665, 126.9780),
        "mumbai" => (19.0760, 72.8777),
        "dubai" => (25.2048, 55.2708),
        "mexico city" => (19.4326, -99.1332),
        "sao paulo" => (-23.5505, -46.6333),
        "buenos aires" => (-34.6037, -58.3816),
        "cairo" => (30.0444, 31.2357),
        "moscow" => (55.7558, 37.6173),
        _ => return Err(anyhow!(
            "Unknown city: '{}'. Please provide coordinates or use a known city name.",
            city
        )),
    };

    Ok(coords)
}

// ============================================================================
// Integration with Tool Executor
// ============================================================================

/// Execute Apple Weather tool - to be called from tools.rs
pub fn execute_apple_weather(location: &str, _date: &str) -> Result<String> {
    let config = WeatherKitConfig::from_env()?;
    let client = WeatherKitClient::new(config)?;
    client.get_weather_by_city(location)
}

/// Check if Apple WeatherKit is configured
pub fn is_configured() -> bool {
    std::env::var("APPLE_TEAM_ID").is_ok()
        && std::env::var("APPLE_SERVICE_ID").is_ok()
        && std::env::var("APPLE_KEY_ID").is_ok()
        && std::env::var("APPLE_PRIVATE_KEY_PATH").is_ok()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geocode_known_cities() {
        let (lat, lon) = geocode_city("NYC").unwrap();
        assert!((lat - 40.7128).abs() < 0.01);
        assert!((lon - (-74.0060)).abs() < 0.01);

        let (lat, lon) = geocode_city("London").unwrap();
        assert!((lat - 51.5074).abs() < 0.01);
    }

    #[test]
    fn test_geocode_unknown_city() {
        let result = geocode_city("Nonexistent City XYZ");
        assert!(result.is_err());
    }

    #[test]
    fn test_format_condition() {
        assert_eq!(format_condition("Clear"), "clear");
        assert_eq!(format_condition("PartlyCloudy"), "partly cloudy");
        assert_eq!(format_condition("Thunderstorms"), "thunderstorms");
    }

    #[test]
    fn test_config_creation() {
        let config = WeatherKitConfig::new(
            "TEAM123",
            "com.example.weather",
            "KEY456",
            "/path/to/key.p8",
        );
        assert_eq!(config.team_id, "TEAM123");
        assert_eq!(config.service_id, "com.example.weather");
    }

    #[test]
    fn test_celsius_to_fahrenheit() {
        // Freezing point: 0C = 32F
        assert!((celsius_to_fahrenheit(0.0) - 32.0).abs() < 0.01);
        // Boiling point: 100C = 212F
        assert!((celsius_to_fahrenheit(100.0) - 212.0).abs() < 0.01);
        // Body temp: 37C = 98.6F
        assert!((celsius_to_fahrenheit(37.0) - 98.6).abs() < 0.1);
        // -40 is the same in both scales
        assert!((celsius_to_fahrenheit(-40.0) - (-40.0)).abs() < 0.01);
    }

    #[test]
    fn test_convert_temp() {
        assert!((convert_temp(20.0, TemperatureUnit::Celsius) - 20.0).abs() < 0.01);
        assert!((convert_temp(20.0, TemperatureUnit::Fahrenheit) - 68.0).abs() < 0.01);
    }

    #[test]
    fn test_temperature_unit_from_str() {
        assert_eq!(TemperatureUnit::from_str("F"), TemperatureUnit::Fahrenheit);
        assert_eq!(TemperatureUnit::from_str("fahrenheit"), TemperatureUnit::Fahrenheit);
        assert_eq!(TemperatureUnit::from_str("FAHRENHEIT"), TemperatureUnit::Fahrenheit);
        assert_eq!(TemperatureUnit::from_str("C"), TemperatureUnit::Celsius);
        assert_eq!(TemperatureUnit::from_str("celsius"), TemperatureUnit::Celsius);
        assert_eq!(TemperatureUnit::from_str("anything"), TemperatureUnit::Celsius);
    }
}
