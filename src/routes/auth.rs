use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, jwt, AppState};

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: Uuid,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let exists: bool = sqlx::query("SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)")
        .bind(&body.email)
        .fetch_one(&state.db)
        .await?
        .get(0);

    if exists {
        return Err(AppError::Conflict("email already registered".into()));
    }

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash error: {}", e)))?
        .to_string();

    let user_id: Uuid = sqlx::query(
        "INSERT INTO users (email, password_hash, display_name) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&body.email)
    .bind(&password_hash)
    .bind(&body.display_name)
    .fetch_one(&state.db)
    .await?
    .get("id");

    let token = jwt::create_token(user_id, &state.jwt_secret)?;
    Ok(Json(AuthResponse { token, user_id }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let row = sqlx::query("SELECT id, password_hash FROM users WHERE email = $1")
        .bind(&body.email)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let user_id: Uuid = row.get("id");
    let stored_hash: String = row.get("password_hash");

    let parsed_hash = PasswordHash::new(&stored_hash).map_err(|_| AppError::Unauthorized)?;

    Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized)?;

    let token = jwt::create_token(user_id, &state.jwt_secret)?;
    Ok(Json(AuthResponse { token, user_id }))
}
