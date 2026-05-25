use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, middleware::auth::AuthUser, AppState};

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateTripRequest {
    pub name: String,
    pub destination: String,
    pub description: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Deserialize)]
pub struct UpdateTripRequest {
    pub name: Option<String>,
    pub destination: Option<String>,
    pub description: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateActivityRequest {
    pub name: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub activity_date: Option<DateTime<Utc>>,
    pub estimated_cost: Option<f64>,
}

#[derive(Deserialize)]
pub struct UpdateActivityRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub activity_date: Option<DateTime<Utc>>,
    pub estimated_cost: Option<f64>,
    pub status: Option<String>,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ActivityCounts {
    pub idea: i64,
    pub confirmed: i64,
    pub done: i64,
}

#[derive(Serialize)]
pub struct TripResponse {
    pub id: Uuid,
    pub group_id: Uuid,
    pub created_by: Uuid,
    pub name: String,
    pub destination: String,
    pub description: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub status: String,
    pub activity_counts: ActivityCounts,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct ActivityResponse {
    pub id: Uuid,
    pub trip_id: Uuid,
    pub suggested_by: Uuid,
    pub suggested_by_name: String,
    pub name: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub activity_date: Option<DateTime<Utc>>,
    pub estimated_cost: Option<f64>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Fetch activity counts for a trip. Returns (idea, confirmed, done).
async fn activity_counts(db: &sqlx::PgPool, trip_id: Uuid) -> Result<ActivityCounts, AppError> {
    let row = sqlx::query(
        "SELECT
            COUNT(CASE WHEN status = 'idea'      THEN 1 END) AS idea_count,
            COUNT(CASE WHEN status = 'confirmed' THEN 1 END) AS confirmed_count,
            COUNT(CASE WHEN status = 'done'      THEN 1 END) AS done_count
         FROM activities
         WHERE trip_id = $1",
    )
    .bind(trip_id)
    .fetch_one(db)
    .await?;

    Ok(ActivityCounts {
        idea: row.get("idea_count"),
        confirmed: row.get("confirmed_count"),
        done: row.get("done_count"),
    })
}

/// Check caller is a member of the group that owns the trip.
/// Returns (group_id, trip_created_by) or NotFound.
async fn resolve_trip(
    db: &sqlx::PgPool,
    trip_id: Uuid,
    user_id: Uuid,
) -> Result<(Uuid, Uuid), AppError> {
    let row = sqlx::query(
        "SELECT t.group_id, t.created_by
         FROM trips t
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE t.id = $1",
    )
    .bind(trip_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok((row.get("group_id"), row.get("created_by")))
}

/// Check caller is a member of the group that owns the activity.
/// Returns (group_id, activity_suggested_by) or NotFound.
async fn resolve_activity(
    db: &sqlx::PgPool,
    activity_id: Uuid,
    user_id: Uuid,
) -> Result<(Uuid, Uuid), AppError> {
    let row = sqlx::query(
        "SELECT t.group_id, a.suggested_by
         FROM activities a
         JOIN trips t ON t.id = a.trip_id
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE a.id = $1",
    )
    .bind(activity_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok((row.get("group_id"), row.get("suggested_by")))
}

/// Check whether a user is the group leader.
async fn is_group_leader(db: &sqlx::PgPool, group_id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_one(db)
    .await?
    .get(0);

    Ok(is_leader)
}

// ── Trip handlers ─────────────────────────────────────────────────────────────

pub async fn list_trips(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
) -> Result<Json<Vec<TripResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    // Verify caller is a group member.
    let is_member: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = $1 AND user_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if !is_member {
        return Err(AppError::NotFound);
    }

    let rows = sqlx::query(
        "SELECT t.id, t.group_id, t.created_by, t.name, t.destination, t.description,
                t.start_date, t.end_date, t.status, t.created_at, t.updated_at,
                COUNT(CASE WHEN a.status = 'idea'      THEN 1 END) AS idea_count,
                COUNT(CASE WHEN a.status = 'confirmed' THEN 1 END) AS confirmed_count,
                COUNT(CASE WHEN a.status = 'done'      THEN 1 END) AS done_count
         FROM trips t
         LEFT JOIN activities a ON a.trip_id = t.id
         WHERE t.group_id = $1
         GROUP BY t.id
         ORDER BY t.created_at DESC",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;

    let trips = rows
        .iter()
        .map(|r| TripResponse {
            id: r.get("id"),
            group_id: r.get("group_id"),
            created_by: r.get("created_by"),
            name: r.get("name"),
            destination: r.get("destination"),
            description: r.get("description"),
            start_date: r.get("start_date"),
            end_date: r.get("end_date"),
            status: r.get("status"),
            activity_counts: ActivityCounts {
                idea: r.get("idea_count"),
                confirmed: r.get("confirmed_count"),
                done: r.get("done_count"),
            },
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();

    Ok(Json(trips))
}

pub async fn create_trip(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
    Json(body): Json<CreateTripRequest>,
) -> Result<Json<TripResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if body.destination.trim().is_empty() {
        return Err(AppError::BadRequest("destination is required".into()));
    }
    if let (Some(start), Some(end)) = (body.start_date, body.end_date) {
        if end < start {
            return Err(AppError::BadRequest("end_date must be on or after start_date".into()));
        }
    }

    let is_member: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = $1 AND user_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if !is_member {
        return Err(AppError::NotFound);
    }

    let row = sqlx::query(
        "INSERT INTO trips (group_id, created_by, name, destination, description, start_date, end_date)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, group_id, created_by, name, destination, description,
                   start_date, end_date, status, created_at, updated_at",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .bind(body.name.trim())
    .bind(body.destination.trim())
    .bind(&body.description)
    .bind(body.start_date)
    .bind(body.end_date)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(TripResponse {
        id: row.get("id"),
        group_id: row.get("group_id"),
        created_by: row.get("created_by"),
        name: row.get("name"),
        destination: row.get("destination"),
        description: row.get("description"),
        start_date: row.get("start_date"),
        end_date: row.get("end_date"),
        status: row.get("status"),
        activity_counts: ActivityCounts { idea: 0, confirmed: 0, done: 0 },
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn get_trip(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
) -> Result<Json<TripResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    // Check membership and get trip in one query.
    let row = sqlx::query(
        "SELECT t.id, t.group_id, t.created_by, t.name, t.destination, t.description,
                t.start_date, t.end_date, t.status, t.created_at, t.updated_at,
                COUNT(CASE WHEN a.status = 'idea'      THEN 1 END) AS idea_count,
                COUNT(CASE WHEN a.status = 'confirmed' THEN 1 END) AS confirmed_count,
                COUNT(CASE WHEN a.status = 'done'      THEN 1 END) AS done_count
         FROM trips t
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         LEFT JOIN activities a ON a.trip_id = t.id
         WHERE t.id = $1
         GROUP BY t.id",
    )
    .bind(trip_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(TripResponse {
        id: row.get("id"),
        group_id: row.get("group_id"),
        created_by: row.get("created_by"),
        name: row.get("name"),
        destination: row.get("destination"),
        description: row.get("description"),
        start_date: row.get("start_date"),
        end_date: row.get("end_date"),
        status: row.get("status"),
        activity_counts: ActivityCounts {
            idea: row.get("idea_count"),
            confirmed: row.get("confirmed_count"),
            done: row.get("done_count"),
        },
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn update_trip(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
    Json(body): Json<UpdateTripRequest>,
) -> Result<Json<TripResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    let (group_id, created_by) = resolve_trip(&state.db, trip_id, auth.user_id).await?;

    // Only trip creator or group leader may update.
    let leader = is_group_leader(&state.db, group_id, auth.user_id).await?;
    if auth.user_id != created_by && !leader {
        return Err(AppError::Unauthorized);
    }

    // Validate fields if provided.
    if let Some(ref n) = body.name {
        if n.trim().is_empty() {
            return Err(AppError::BadRequest("name cannot be empty".into()));
        }
    }
    if let Some(ref d) = body.destination {
        if d.trim().is_empty() {
            return Err(AppError::BadRequest("destination cannot be empty".into()));
        }
    }
    if let Some(ref s) = body.status {
        let valid = ["planning", "upcoming", "active", "completed", "cancelled"];
        if !valid.contains(&s.as_str()) {
            return Err(AppError::BadRequest(format!("invalid status '{s}'")));
        }
    }
    // Only validate date order when both dates are explicitly provided in this request.
    if let (Some(start), Some(end)) = (body.start_date, body.end_date) {
        if end < start {
            return Err(AppError::BadRequest("end_date must be on or after start_date".into()));
        }
    }

    let row = sqlx::query(
        "UPDATE trips
         SET name        = COALESCE($1, name),
             destination = COALESCE($2, destination),
             description = COALESCE($3, description),
             start_date  = COALESCE($4, start_date),
             end_date    = COALESCE($5, end_date),
             status      = COALESCE($6, status),
             updated_at  = NOW()
         WHERE id = $7
         RETURNING id, group_id, created_by, name, destination, description,
                   start_date, end_date, status, created_at, updated_at",
    )
    .bind(&body.name)
    .bind(&body.destination)
    .bind(&body.description)
    .bind(body.start_date)
    .bind(body.end_date)
    .bind(&body.status)
    .bind(trip_id)
    .fetch_one(&state.db)
    .await?;

    let counts = activity_counts(&state.db, trip_id).await?;

    Ok(Json(TripResponse {
        id: row.get("id"),
        group_id: row.get("group_id"),
        created_by: row.get("created_by"),
        name: row.get("name"),
        destination: row.get("destination"),
        description: row.get("description"),
        start_date: row.get("start_date"),
        end_date: row.get("end_date"),
        status: row.get("status"),
        activity_counts: counts,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn delete_trip(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    let (group_id, _) = resolve_trip(&state.db, trip_id, auth.user_id).await?;

    // Only the group leader can delete a trip.
    let leader = is_group_leader(&state.db, group_id, auth.user_id).await?;
    if !leader {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("DELETE FROM trips WHERE id = $1")
        .bind(trip_id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Activity handlers ─────────────────────────────────────────────────────────

pub async fn list_activities(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
) -> Result<Json<Vec<ActivityResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    resolve_trip(&state.db, trip_id, auth.user_id).await?; // membership check

    let rows = sqlx::query(
        "SELECT a.id, a.trip_id, a.suggested_by, u.display_name AS suggested_by_name,
                a.name, a.description, a.location, a.activity_date, a.estimated_cost,
                a.status, a.created_at, a.updated_at
         FROM activities a
         JOIN users u ON u.id = a.suggested_by
         WHERE a.trip_id = $1
         ORDER BY
             CASE a.status
                 WHEN 'idea'      THEN 1
                 WHEN 'confirmed' THEN 2
                 WHEN 'done'      THEN 3
             END,
             a.created_at ASC",
    )
    .bind(trip_id)
    .fetch_all(&state.db)
    .await?;

    let activities = rows
        .iter()
        .map(|r| ActivityResponse {
            id: r.get("id"),
            trip_id: r.get("trip_id"),
            suggested_by: r.get("suggested_by"),
            suggested_by_name: r.get("suggested_by_name"),
            name: r.get("name"),
            description: r.get("description"),
            location: r.get("location"),
            activity_date: r.get("activity_date"),
            estimated_cost: r.get("estimated_cost"),
            status: r.get("status"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();

    Ok(Json(activities))
}

pub async fn create_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
    Json(body): Json<CreateActivityRequest>,
) -> Result<Json<ActivityResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    resolve_trip(&state.db, trip_id, auth.user_id).await?; // membership check

    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if let Some(cost) = body.estimated_cost {
        if cost < 0.0 {
            return Err(AppError::BadRequest("estimated_cost cannot be negative".into()));
        }
    }

    let row = sqlx::query(
        "INSERT INTO activities
            (trip_id, suggested_by, name, description, location, activity_date, estimated_cost)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, trip_id, suggested_by, name, description, location,
                   activity_date, estimated_cost, status, created_at, updated_at",
    )
    .bind(trip_id)
    .bind(auth.user_id)
    .bind(body.name.trim())
    .bind(&body.description)
    .bind(&body.location)
    .bind(body.activity_date)
    .bind(body.estimated_cost)
    .fetch_one(&state.db)
    .await?;

    // Fetch display name separately (could also JOIN in the INSERT's RETURNING but that's not standard SQL).
    let display_name: String =
        sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(auth.user_id)
            .fetch_one(&state.db)
            .await?
            .get("display_name");

    Ok(Json(ActivityResponse {
        id: row.get("id"),
        trip_id: row.get("trip_id"),
        suggested_by: row.get("suggested_by"),
        suggested_by_name: display_name,
        name: row.get("name"),
        description: row.get("description"),
        location: row.get("location"),
        activity_date: row.get("activity_date"),
        estimated_cost: row.get("estimated_cost"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn update_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(activity_id): Path<Uuid>,
    Json(body): Json<UpdateActivityRequest>,
) -> Result<Json<ActivityResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    resolve_activity(&state.db, activity_id, auth.user_id).await?; // membership check

    // Validate fields if provided.
    if let Some(ref n) = body.name {
        if n.trim().is_empty() {
            return Err(AppError::BadRequest("name cannot be empty".into()));
        }
    }
    if let Some(cost) = body.estimated_cost {
        if cost < 0.0 {
            return Err(AppError::BadRequest("estimated_cost cannot be negative".into()));
        }
    }
    if let Some(ref s) = body.status {
        let valid = ["idea", "confirmed", "done"];
        if !valid.contains(&s.as_str()) {
            return Err(AppError::BadRequest(format!("invalid status '{s}'")));
        }
    }

    let row = sqlx::query(
        "UPDATE activities
         SET name           = COALESCE($1, name),
             description    = COALESCE($2, description),
             location       = COALESCE($3, location),
             activity_date  = COALESCE($4, activity_date),
             estimated_cost = COALESCE($5, estimated_cost),
             status         = COALESCE($6, status),
             updated_at     = NOW()
         WHERE id = $7
         RETURNING id, trip_id, suggested_by, name, description, location,
                   activity_date, estimated_cost, status, created_at, updated_at",
    )
    .bind(&body.name)
    .bind(&body.description)
    .bind(&body.location)
    .bind(body.activity_date)
    .bind(body.estimated_cost)
    .bind(&body.status)
    .bind(activity_id)
    .fetch_one(&state.db)
    .await?;

    let suggested_by: Uuid = row.get("suggested_by");
    let display_name: String =
        sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(suggested_by)
            .fetch_one(&state.db)
            .await?
            .get("display_name");

    Ok(Json(ActivityResponse {
        id: row.get("id"),
        trip_id: row.get("trip_id"),
        suggested_by,
        suggested_by_name: display_name,
        name: row.get("name"),
        description: row.get("description"),
        location: row.get("location"),
        activity_date: row.get("activity_date"),
        estimated_cost: row.get("estimated_cost"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn delete_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(activity_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    let (group_id, suggested_by) = resolve_activity(&state.db, activity_id, auth.user_id).await?;

    // Only activity creator or group leader may delete.
    let leader = is_group_leader(&state.db, group_id, auth.user_id).await?;
    if auth.user_id != suggested_by && !leader {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("DELETE FROM activities WHERE id = $1")
        .bind(activity_id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
