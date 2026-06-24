use std::{collections::HashMap, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// ── Constants ──────────────────────────────────────────────────────────────────

const BASE_URL: &str = "https://test.api.amadeus.com/v1";
/// Refresh 60 s before the token's 1799 s expiry.
const TOKEN_REFRESH_SECS: u64 = 1_740;
/// Search radius sent to the Activities API (kilometres).
const RADIUS_KM: u32 = 20;

// ── Public output type ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct DiscoverResult {
    pub place_id: String,
    pub name: String,
    pub category: String,
    pub address: String,
    pub distance_m: u32,
    pub rating: Option<f32>,
    pub photo_url: Option<String>,
}

/// Shared in-memory result cache keyed by `"{location}:{category}:{query}"`.
pub type DiscoverCache = Arc<Mutex<HashMap<String, (Vec<DiscoverResult>, Instant)>>>;

pub fn new_discover_cache() -> DiscoverCache {
    Arc::new(Mutex::new(HashMap::new()))
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Extract just the city name from a destination string.
/// Strips state/country qualifiers so Amadeus geocoding works reliably.
/// e.g. "Milwaukee Wisconsin" → "Milwaukee", "Paris, France" → "Paris"
fn extract_city(location: &str) -> String {
    let s = location.split(',').next().unwrap_or(location).trim();

    const US_STATES: &[&str] = &[
        "Alabama", "Alaska", "Arizona", "Arkansas", "California", "Colorado",
        "Connecticut", "Delaware", "Florida", "Georgia", "Hawaii", "Idaho",
        "Illinois", "Indiana", "Iowa", "Kansas", "Kentucky", "Louisiana",
        "Maine", "Maryland", "Massachusetts", "Michigan", "Minnesota",
        "Mississippi", "Missouri", "Montana", "Nebraska", "Nevada",
        "New Hampshire", "New Jersey", "New Mexico", "New York",
        "North Carolina", "North Dakota", "Ohio", "Oklahoma", "Oregon",
        "Pennsylvania", "Rhode Island", "South Carolina", "South Dakota",
        "Tennessee", "Texas", "Utah", "Vermont", "Virginia", "Washington",
        "West Virginia", "Wisconsin", "Wyoming",
    ];

    let s_lower = s.to_lowercase();
    for state in US_STATES {
        let state_lower = state.to_lowercase();
        if s_lower.ends_with(&state_lower) {
            let stripped = s[..s.len() - state.len()].trim();
            if !stripped.is_empty() {
                return stripped.to_owned();
            }
        }
    }

    s.to_owned()
}

/// Approximate distance in metres between two lat/lon points.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> u32 {
    const R: f64 = 6_371_000.0;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let dphi = (lat2 - lat1).to_radians();
    let dlambda = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    (R * c) as u32
}

/// Convert an Amadeus uppercase category tag to a readable label.
fn format_category(cat: &str) -> String {
    match cat {
        "SIGHTS"               => "Sights & Landmarks".into(),
        "TOURS"                => "Tours".into(),
        "SPORT"                => "Sports & Outdoors".into(),
        "FOOD_OR_DRINK"        => "Food & Drink".into(),
        "ENTERTAINMENT"        => "Entertainment".into(),
        "WELLNESS_OR_FITNESS"  => "Wellness & Fitness".into(),
        "NATURE_AND_PARKS"     => "Nature & Parks".into(),
        "SHOPPING"             => "Shopping".into(),
        other => {
            // Fallback: replace underscores, title-case each word.
            other
                .split('_')
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => {
                            first.to_uppercase().to_string()
                                + &chars.as_str().to_lowercase()
                        }
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// Return `true` if the activity's category list matches the UI slug.
fn matches_category(categories: &[String], slug: &str) -> bool {
    match slug {
        "outdoors" => categories
            .iter()
            .any(|c| c == "SPORT" || c == "NATURE_AND_PARKS"),
        "culture" => categories
            .iter()
            .any(|c| c == "SIGHTS" || c == "TOURS" || c == "ENTERTAINMENT"),
        "food" => categories.iter().any(|c| c == "FOOD_OR_DRINK"),
        "sports" => categories.iter().any(|c| c == "SPORT"),
        _ => true, // "all" — pass everything through
    }
}

// ── Raw Amadeus response shapes ────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct LocationResponse {
    #[serde(default)]
    data: Vec<LocationData>,
}

#[derive(Deserialize)]
struct LocationData {
    #[serde(rename = "geoCode")]
    geo_code: GeoCode,
}

#[derive(Deserialize, Clone, Copy)]
struct GeoCode {
    latitude: f64,
    longitude: f64,
}

#[derive(Deserialize)]
struct ActivitiesResponse {
    #[serde(default)]
    data: Vec<Activity>,
}

#[derive(Deserialize)]
struct Activity {
    id: String,
    name: String,
    #[serde(rename = "geoCode")]
    geo_code: Option<GeoCode>,
    rating: Option<String>,
    #[serde(default)]
    pictures: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
}

// ── Client ─────────────────────────────────────────────────────────────────────

pub struct AmadeusClient {
    client: Client,
    api_key: String,
    api_secret: String,
    /// Cached (token, fetched_at). Refreshed automatically before expiry.
    token: Mutex<Option<(String, Instant)>>,
    /// city_name_lowercase → GeoCode. Never expires (city coordinates don't change).
    geocode_cache: Mutex<HashMap<String, GeoCode>>,
}

impl AmadeusClient {
    pub fn new(api_key: String, api_secret: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            api_secret,
            token: Mutex::new(None),
            geocode_cache: Mutex::new(HashMap::new()),
        }
    }

    // ── Token management ───────────────────────────────────────────────────────

    async fn get_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        if let Some((token, fetched_at)) = guard.as_ref() {
            if fetched_at.elapsed().as_secs() < TOKEN_REFRESH_SECS {
                return Ok(token.clone());
            }
        }
        let token = self.fetch_token().await?;
        *guard = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    async fn fetch_token(&self) -> Result<String> {
        let resp = self
            .client
            .post(format!("{BASE_URL}/security/oauth2/token"))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.api_key.as_str()),
                ("client_secret", self.api_secret.as_str()),
            ])
            .send()
            .await
            .context("Amadeus token request failed")?;

        let status = resp.status();
        let body = resp.text().await.context("Failed to read token response")?;

        if !status.is_success() {
            tracing::error!(%status, %body, "Amadeus token endpoint returned error");
            anyhow::bail!("Failed to obtain Amadeus access token: HTTP {status}");
        }

        let parsed: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(%e, %body, "Failed to parse Amadeus token response");
            anyhow::anyhow!("Failed to parse Amadeus token response: {e}")
        })?;

        tracing::debug!("Amadeus access token refreshed");
        Ok(parsed.access_token)
    }

    // ── Geocoding ──────────────────────────────────────────────────────────────

    /// Resolve a location string to lat/lon via the Amadeus city search endpoint.
    /// Returns `None` when the city is not found.
    async fn geocode(&self, location: &str) -> Result<Option<GeoCode>> {
        let city = extract_city(location);
        let location = city.as_str();
        let key = location.to_lowercase();

        {
            let cache = self.geocode_cache.lock().await;
            if let Some(&geo) = cache.get(&key) {
                return Ok(Some(geo));
            }
        }

        let token = self.get_token().await?;
        let resp = self
            .client
            .get(format!("{BASE_URL}/reference-data/locations"))
            .bearer_auth(&token)
            .query(&[
                ("keyword", location.trim()),
                ("subType", "CITY"),
                ("page[limit]", "1"),
            ])
            .send()
            .await
            .context("Amadeus geocode request failed")?;

        let status = resp.status();
        let body = resp.text().await.context("Failed to read geocode response")?;

        if !status.is_success() {
            tracing::error!(%status, %body, "Amadeus geocode endpoint returned error");
            anyhow::bail!("Amadeus geocode returned HTTP {status}");
        }

        let parsed: LocationResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(%e, %body, "Failed to parse Amadeus geocode response");
            anyhow::anyhow!("Failed to parse Amadeus geocode response: {e}")
        })?;

        let geo = parsed.data.into_iter().next().map(|l| l.geo_code);

        if let Some(g) = geo {
            let mut cache = self.geocode_cache.lock().await;
            cache.insert(key, g);
        } else {
            tracing::warn!(location, "Amadeus geocode: city not found");
        }

        Ok(geo)
    }

    // ── Public search ──────────────────────────────────────────────────────────

    /// Search for activities near `location`.
    ///
    /// The `category` slug is applied client-side after the API returns results
    /// because the Amadeus Activities endpoint does not support category filters.
    /// The `query` parameter is accepted for API compatibility but ignored —
    /// Amadeus Activities does not expose keyword search.
    pub async fn search(
        &self,
        location: &str,
        category: &str,
        _query: Option<&str>,
    ) -> Result<Vec<DiscoverResult>> {
        let geo = match self.geocode(location).await? {
            Some(g) => g,
            None => return Ok(vec![]),
        };

        let token = self.get_token().await?;
        let resp = self
            .client
            .get(format!("{BASE_URL}/shopping/activities"))
            .bearer_auth(&token)
            .query(&[
                ("latitude", geo.latitude.to_string()),
                ("longitude", geo.longitude.to_string()),
                ("radius", RADIUS_KM.to_string()),
            ])
            .send()
            .await
            .context("Amadeus activities request failed")?;

        let status = resp.status();
        let body = resp.text().await.context("Failed to read activities response")?;

        if !status.is_success() {
            tracing::error!(%status, %body, "Amadeus activities endpoint returned error");
            anyhow::bail!("Amadeus activities returned HTTP {status}");
        }

        let parsed: ActivitiesResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(%e, %body, "Failed to parse Amadeus activities response");
            anyhow::anyhow!("Failed to parse Amadeus activities response: {e}")
        })?;

        let results = parsed
            .data
            .into_iter()
            .filter(|a| !a.name.trim().is_empty())
            .filter(|a| matches_category(&a.categories, category))
            .map(|a| {
                let distance_m = a.geo_code.map_or(0, |g| {
                    haversine_m(geo.latitude, geo.longitude, g.latitude, g.longitude)
                });

                let category_label = a
                    .categories
                    .first()
                    .map(|c| format_category(c))
                    .unwrap_or_else(|| "Activity".to_owned());

                let rating = a.rating.as_deref().and_then(|r| r.parse::<f32>().ok());

                let photo_url = a.pictures.into_iter().next();

                DiscoverResult {
                    place_id: a.id,
                    name: a.name,
                    category: category_label,
                    address: location.to_owned(),
                    distance_m,
                    rating,
                    photo_url,
                }
            })
            .collect();

        Ok(results)
    }
}
