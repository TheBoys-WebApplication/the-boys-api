use axum::http::HeaderMap;
use uuid::Uuid;

use crate::{error::AppError, jwt};

pub struct AuthUser {
    pub user_id: Uuid,
}

impl AuthUser {
    /// Extract and validate a Bearer JWT from request headers.
    /// Protected handlers call this directly instead of using a FromRequestParts impl.
    pub fn from_headers(headers: &HeaderMap, jwt_secret: &str) -> Result<Self, AppError> {
        let token = headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or(AppError::Unauthorized)?;

        let claims = jwt::decode_token(token, jwt_secret)?;
        let user_id = claims.sub.parse::<Uuid>().map_err(|_| AppError::Unauthorized)?;

        Ok(AuthUser { user_id })
    }
}
