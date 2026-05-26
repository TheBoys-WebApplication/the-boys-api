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
    amadeus::DiscoverResult,
    error::AppError,
    middleware::auth::AuthUser,
    AppState,
};

const CACHE_TTL_SECS: u64 = 3_600; // 1 hour — mirrors staleTime on the frontend

#[derive(Deserialize)]
pub struct DiscoverParams {
    /// UI category slug: all | outdoors | culture | food | sports
    pub category: Option<String>,
    /// Optional keyword filter (passed through; Amadeus ignores it).
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

    // Single query: verify group membership AND fetch the trip destination.
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

    // Use the caller-supplied location or fall back to the trip destination.
    let location = params
        .location
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&trip_destination)
        .to_owned();

    let category = params.category.as_deref().unwrap_or("all").to_owned();
    let query_text = params.query.clone();

    // Deterministic cache key.
    let cache_key = format!(
        "{}:{}:{}",
        location.to_lowercase(),
        category,
        query_text.as_deref().unwrap_or("").trim().to_lowercase(),
    );

    // Return cached result if still fresh.
    {
        let cache = state.discover_cache.lock().await;
        if let Some((results, inserted_at)) = cache.get(&cache_key) {
            if inserted_at.elapsed().as_secs() < CACHE_TTL_SECS {
                return Ok(Json(results.clone()));
            }
        }
    }

    // Cache miss — call Amadeus.
    let results = state
        .amadeus_client
        .search(&location, &category, query_text.as_deref())
        .await
        .map_err(|e| {
            tracing::error!("Amadeus search failed: {e:#}");
            AppError::Internal(anyhow::anyhow!("discover service unavailable"))
        })?;

    // Populate cache.
    {
        let mut cache = state.discover_cache.lock().await;
        cache.insert(cache_key, (results.clone(), Instant::now()));
    }

    Ok(Json(results))
}
