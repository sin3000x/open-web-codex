use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    Json,
};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_store::AppState;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

/// Authenticated user extracted from the session token
/// (Authorization: Bearer <token> header or session_token cookie).
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub organization_id: Uuid,
    pub organization_role: String,
}

pub type AuthRejection = (StatusCode, Json<PlatformError>);

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .or_else(|| {
                parts
                    .headers
                    .get("cookie")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|c| {
                        c.split(';')
                            .find(|part| part.trim().starts_with("session_token="))
                            .and_then(|part| part.trim().strip_prefix("session_token="))
                    })
            });

        let token = token.ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(PlatformError::unauthorized("missing session token")),
            )
        })?;

        authenticate_token(&state.db, token).await
    }
}

pub async fn authenticate_token(
    db: &sqlx::PgPool,
    token: &str,
) -> Result<AuthenticatedUser, AuthRejection> {
    if token.trim().is_empty() || token.len() > 512 {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(PlatformError::unauthorized("invalid session token")),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());

    let row = sqlx::query(
        "SELECT s.id AS session_id, u.id, u.name, u.username, u.email, u.role, \
                    m.organization_id, m.role AS organization_role \
             FROM sessions s \
             JOIN users u ON u.id = s.user_id \
             JOIN memberships m ON m.organization_id = s.organization_id AND m.user_id = s.user_id \
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(
                "authentication database operation failed",
            )),
        )
    })?;

    let row = row.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(PlatformError::unauthorized("invalid or expired session")),
        )
    })?;

    Ok(AuthenticatedUser {
        session_id: row.get("session_id"),
        user_id: row.get("id"),
        name: row.get("name"),
        username: row.get("username"),
        email: row.get("email"),
        role: row.get("role"),
        organization_id: row.get("organization_id"),
        organization_role: row.get("organization_role"),
    })
}

pub async fn require_runtime_profile(
    db: &sqlx::PgPool,
    auth: &AuthenticatedUser,
    runtime_key: &str,
) -> Result<(), AuthRejection> {
    let authorized: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
             SELECT 1 FROM profiles \
             WHERE runtime_key = $1 AND organization_id = $2 AND owner_user_id = $3 \
               AND status = 'active' \
         )",
    )
    .bind(runtime_key)
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .fetch_one(db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(
                "Profile authorization database operation failed",
            )),
        )
    })?;
    if authorized {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Profile was not found")),
        ))
    }
}
