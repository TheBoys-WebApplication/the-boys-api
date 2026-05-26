use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use sqlx::Row;
use std::time::Instant;
use uuid::Uuid;

use crate::{
    error::AppError,
    foursquare::DiscoverResult,
    middleware::auth::AuthUser,
    AppState,
};

const CACHE_TTL_SECS: u64 = 3_600; // 1 hour — matches staleTime on the frontend

/// Map a UI category slug to the corresponding Foursquare category IDs.
fn category_ids(slug: &str) -> &'static str {
    match slug {
        "outdoors" => "17000",               // Landmarks & Outdoors
        "culture"  => "10000",               // Arts & Entertainment
        "food"     => "13000",               // Dining & Drinking
        "sports"   => "19000",               // Sports & Recreation
        _          => "10000,13000,17000,19000", // all
    }
}

#[derive(Deserialize)]
pub struct DiscoverParams {
    /// UI category slug: all | outdoors | culture | food | sports
    pub category: Option<String>,
    /// Optional free-text keyword filter.
    pub query: Option<String>,
    /// Overrides the trip's destination when provided.
    pub location: Option<String>,
}

pub async fn search(
    Path(trip_id): Path<Uuid>,
    Query(params): Query<DiscoverParams>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DiscoverResult>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    // Single query: verify membership AND fetch the trip destination.
    let row = sqlx::query(
        "SELECT t.destination
         FROM trips t
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE t.id = $1",
    )
    .bind(trip_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let trip_destination: String = row.get("destination");

    // Use the caller-supplied location or fall back to the trip's destination.
    let location = params
        .location
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&trip_destination)
        .to_owned();

    let category = params.category.as_deref().unwrap_or("all").to_owned();
    let query_text = params.query.clone();

    // Build a deterministic cache key.
    let cache_key = format!(
        "{}:{}:{}",
        location.to_lowercase(),
        category,
        query_text.as_deref().unwrap_or("").trim().to_lowercase(),
    );

    // Check cache.
    {
        let cache = state.discover_cache.lock().await;
        if let Some((results, inserted_at)) = cache.get(&cache_key) {
            if inserted_at.elapsed().as_secs() < CACHE_TTL_SECS {
                return Ok(Json(results.clone()));
            }
        }
    }

    // Cache miss — hit Foursquare.
    let categories = category_ids(&category);
    let results = state
        .fsq_client
        .search(&location, categories, query_text.as_deref(), 20)
        .await
        .map_err(|e| {
            tracing::error!("Foursquare search failed: {e:#}");
            AppError::Internal(anyhow::anyhow!("discover service unavailable"))
        })?;

    // Store result in cache.
    {
        let mut cache = state.discover_cache.lock().await;
        cache.insert(cache_key, (results.clone(), Instant::now()));
    }

    Ok(Json(results))
}
