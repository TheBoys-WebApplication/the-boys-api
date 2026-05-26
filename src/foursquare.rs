use std::{collections::HashMap, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// ── Public types ──────────────────────────────────────────────────────────────

/// Normalized place result returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct DiscoverResult {
    pub fsq_id: String,
    pub name: String,
    pub category: String,
    pub address: String,
    pub distance_m: u32,
    pub rating: Option<f32>,
    pub photo_url: Option<String>,
}

/// In-memory result cache. Key: `"{location}:{category}:{query}"`.
pub type DiscoverCache = Arc<Mutex<HashMap<String, (Vec<DiscoverResult>, Instant)>>>;

pub fn new_discover_cache() -> DiscoverCache {
    Arc::new(Mutex::new(HashMap::new()))
}

// ── Foursquare raw response shapes ────────────────────────────────────────────

#[derive(Deserialize)]
struct FsqSearchResponse {
    results: Vec<FsqPlace>,
}

#[derive(Deserialize)]
struct FsqPlace {
    fsq_id: String,
    name: String,
    #[serde(default)]
    categories: Vec<FsqCategory>,
    location: FsqLocation,
    distance: Option<u32>,
    rating: Option<f64>,
    #[serde(default)]
    photos: Vec<FsqPhoto>,
}

#[derive(Deserialize)]
struct FsqCategory {
    name: String,
}

#[derive(Deserialize)]
struct FsqLocation {
    formatted_address: Option<String>,
    locality: Option<String>,
}

#[derive(Deserialize)]
struct FsqPhoto {
    prefix: String,
    suffix: String,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct FoursquareClient {
    client: Client,
    api_key: String,
}

impl FoursquareClient {
    pub fn new(api_key: String) -> Self {
        Self { client: Client::new(), api_key }
    }

    /// Search for places near `location`.
    ///
    /// Returns an empty `Vec` (rather than an error) when Foursquare responds
    /// with 400, which typically means the location string was unrecognised.
    pub async fn search(
        &self,
        location: &str,
        categories: &str,
        query: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DiscoverResult>> {
        let mut params: Vec<(&str, String)> = vec![
            ("near", location.to_owned()),
            ("categories", categories.to_owned()),
            ("limit", limit.to_string()),
            ("sort", "POPULARITY".to_owned()),
            (
                "fields",
                "fsq_id,name,categories,location,distance,rating,photos".to_owned(),
            ),
        ];

        if let Some(q) = query {
            let q = q.trim();
            if !q.is_empty() {
                params.push(("query", q.to_owned()));
            }
        }

        let resp = self
            .client
            .get("https://api.foursquare.com/v3/places/search")
            .header("Authorization", &self.api_key)
            .query(&params)
            .send()
            .await
            .context("Foursquare HTTP request failed")?;

        // 400 typically means an unrecognised location — return empty gracefully.
        if resp.status() == reqwest::StatusCode::BAD_REQUEST {
            tracing::warn!(
                location = location,
                query = query,
                "Foursquare returned 400; returning empty results"
            );
            return Ok(vec![]);
        }

        if !resp.status().is_success() {
            anyhow::bail!("Foursquare returned HTTP {}", resp.status());
        }

        let fsq: FsqSearchResponse = resp
            .json()
            .await
            .context("Failed to deserialise Foursquare response")?;

        let results = fsq
            .results
            .into_iter()
            .filter(|p| !p.name.trim().is_empty())
            .map(|p| {
                let category = p
                    .categories
                    .into_iter()
                    .next()
                    .map(|c| c.name)
                    .unwrap_or_else(|| "Place".to_owned());

                let address = p
                    .location
                    .formatted_address
                    .or(p.location.locality)
                    .unwrap_or_else(|| "Unknown location".to_owned());

                let photo_url = p
                    .photos
                    .into_iter()
                    .next()
                    .map(|ph| format!("{}300x300{}", ph.prefix, ph.suffix));

                DiscoverResult {
                    fsq_id: p.fsq_id,
                    name: p.name,
                    category,
                    address,
                    distance_m: p.distance.unwrap_or(0),
                    rating: p.rating.map(|r| r as f32),
                    photo_url,
                }
            })
            .collect();

        Ok(results)
    }
}
