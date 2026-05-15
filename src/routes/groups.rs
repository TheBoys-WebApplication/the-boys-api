use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, middleware::auth::AuthUser, AppState};

#[derive(Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct JoinGroupRequest {
    pub invite_code: String,
}

#[derive(Serialize)]
pub struct GroupResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub leader_id: Uuid,
    pub invite_code: String,
    pub member_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct MemberResponse {
    pub user_id: Uuid,
    pub display_name: String,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

pub async fn create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateGroupRequest>,
) -> Result<Json<GroupResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    let mut tx = state.db.begin().await?;

    let row = sqlx::query(
        "INSERT INTO groups (name, description, leader_id)
         VALUES ($1, $2, $3)
         RETURNING id, name, description, leader_id, invite_code, created_at",
    )
    .bind(&body.name)
    .bind(&body.description)
    .bind(auth.user_id)
    .fetch_one(&mut *tx)
    .await?;

    let group_id: Uuid = row.get("id");

    sqlx::query(
        "INSERT INTO group_members (group_id, user_id, role) VALUES ($1, $2, 'leader')",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(GroupResponse {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        leader_id: row.get("leader_id"),
        invite_code: row.get("invite_code"),
        member_count: 1,
        created_at: row.get("created_at"),
    }))
}

pub async fn list_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<GroupResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let rows = sqlx::query(
        "SELECT g.id, g.name, g.description, g.leader_id, g.invite_code, g.created_at,
                COUNT(gm_all.id) AS member_count
         FROM groups g
         JOIN group_members gm ON gm.group_id = g.id AND gm.user_id = $1
         LEFT JOIN group_members gm_all ON gm_all.group_id = g.id
         GROUP BY g.id, g.name, g.description, g.leader_id, g.invite_code, g.created_at
         ORDER BY g.created_at DESC",
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await?;

    let groups = rows
        .iter()
        .map(|r| GroupResponse {
            id: r.get("id"),
            name: r.get("name"),
            description: r.get("description"),
            leader_id: r.get("leader_id"),
            invite_code: r.get("invite_code"),
            member_count: r.get("member_count"),
            created_at: r.get("created_at"),
        })
        .collect();

    Ok(Json(groups))
}

pub async fn get_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
) -> Result<Json<GroupResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

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
        "SELECT g.id, g.name, g.description, g.leader_id, g.invite_code, g.created_at,
                COUNT(gm.id) AS member_count
         FROM groups g
         LEFT JOIN group_members gm ON gm.group_id = g.id
         WHERE g.id = $1
         GROUP BY g.id",
    )
    .bind(group_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(GroupResponse {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        leader_id: row.get("leader_id"),
        invite_code: row.get("invite_code"),
        member_count: row.get("member_count"),
        created_at: row.get("created_at"),
    }))
}

pub async fn update_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
    Json(body): Json<UpdateGroupRequest>,
) -> Result<Json<GroupResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if !is_leader {
        return Err(AppError::Unauthorized);
    }

    let row = sqlx::query(
        "UPDATE groups
         SET name        = COALESCE($1, name),
             description = COALESCE($2, description),
             updated_at  = NOW()
         WHERE id = $3
         RETURNING id, name, description, leader_id, invite_code, created_at",
    )
    .bind(&body.name)
    .bind(&body.description)
    .bind(group_id)
    .fetch_one(&state.db)
    .await?;

    let member_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM group_members WHERE group_id = $1")
            .bind(group_id)
            .fetch_one(&state.db)
            .await?
            .get(0);

    Ok(Json(GroupResponse {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        leader_id: row.get("leader_id"),
        invite_code: row.get("invite_code"),
        member_count,
        created_at: row.get("created_at"),
    }))
}

pub async fn delete_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if !is_leader {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(group_id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn join_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<JoinGroupRequest>,
) -> Result<Json<GroupResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let group_row = sqlx::query(
        "SELECT id, name, description, leader_id, invite_code, created_at
         FROM groups WHERE invite_code = $1",
    )
    .bind(&body.invite_code)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let group_id: Uuid = group_row.get("id");

    let already_member: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = $1 AND user_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if already_member {
        return Err(AppError::Conflict("already a member of this group".into()));
    }

    sqlx::query(
        "INSERT INTO group_members (group_id, user_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await?;

    let member_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM group_members WHERE group_id = $1")
            .bind(group_id)
            .fetch_one(&state.db)
            .await?
            .get(0);

    Ok(Json(GroupResponse {
        id: group_row.get("id"),
        name: group_row.get("name"),
        description: group_row.get("description"),
        leader_id: group_row.get("leader_id"),
        invite_code: group_row.get("invite_code"),
        member_count,
        created_at: group_row.get("created_at"),
    }))
}

pub async fn regenerate_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if !is_leader {
        return Err(AppError::Unauthorized);
    }

    let new_code: String = sqlx::query(
        "UPDATE groups
         SET invite_code = encode(gen_random_bytes(6), 'hex'), updated_at = NOW()
         WHERE id = $1
         RETURNING invite_code",
    )
    .bind(group_id)
    .fetch_one(&state.db)
    .await?
    .get("invite_code");

    Ok(Json(serde_json::json!({ "invite_code": new_code })))
}

pub async fn list_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
) -> Result<Json<Vec<MemberResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

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
        "SELECT gm.user_id, u.display_name, gm.role, gm.joined_at
         FROM group_members gm
         JOIN users u ON u.id = gm.user_id
         WHERE gm.group_id = $1
         ORDER BY gm.joined_at ASC",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;

    let members = rows
        .iter()
        .map(|r| MemberResponse {
            user_id: r.get("user_id"),
            display_name: r.get("display_name"),
            role: r.get("role"),
            joined_at: r.get("joined_at"),
        })
        .collect();

    Ok(Json(members))
}

pub async fn remove_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((group_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if !is_leader {
        return Err(AppError::Unauthorized);
    }

    if user_id == auth.user_id {
        return Err(AppError::BadRequest(
            "leader cannot remove themselves; delete the group instead".into(),
        ));
    }

    let result = sqlx::query(
        "DELETE FROM group_members WHERE group_id = $1 AND user_id = $2",
    )
    .bind(group_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}
