use axum::{
    extract::State,
    http::{header, HeaderName, StatusCode},
    response::AppendHeaders,
    Json,
};
use open_web_codex_auth::{hash_password, needs_rehash, verify_password_or_dummy};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    LoginRequest, LoginResponse, Organization, SelectOrganizationRequest, SessionOrganization, User,
};
use open_web_codex_platform_store::AppState;
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;
type CookieApiResult<T> =
    Result<(AppendHeaders<[(HeaderName, String); 1]>, Json<T>), (StatusCode, Json<PlatformError>)>;

/// POST /api/sessions/local — issue a session for the first local owner.
///
/// The current single-Profile product bypasses interactive authentication while
/// retaining the same Session, Organization, Profile and authorization model
/// used by the rest of the platform.
pub async fn create_local_session(State(state): State<AppState>) -> CookieApiResult<LoginResponse> {
    let row = sqlx::query(
        "SELECT u.id, u.name, u.username, u.email, u.role, u.created_at, u.updated_at, \
                o.id AS organization_id, o.name AS organization_name, o.slug AS organization_slug, \
                o.created_at AS organization_created_at, o.updated_at AS organization_updated_at, \
                m.role AS membership_role \
         FROM users u \
         JOIN memberships m ON m.user_id = u.id \
         JOIN organizations o ON o.id = m.organization_id \
         ORDER BY CASE WHEN m.role = 'owner' THEN 0 ELSE 1 END, m.created_at, m.id \
         LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .map_err(internal_database_error)?
    .ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(PlatformError::internal("local owner is not initialized")),
        )
    })?;

    let session_token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let token_hash = hex::encode(Sha256::digest(session_token.as_bytes()));
    let user_id: Uuid = row.get("id");
    let organization_id: Uuid = row.get("organization_id");

    sqlx::query(
        "INSERT INTO sessions (user_id, organization_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '7 days')",
    )
    .bind(user_id)
    .bind(organization_id)
    .bind(&token_hash)
    .execute(&state.db)
    .await
    .map_err(internal_database_error)?;

    Ok((
        AppendHeaders([(header::SET_COOKIE, session_cookie(&session_token))]),
        Json(LoginResponse {
            user: User {
                id: user_id,
                name: row.get("name"),
                username: row.get("username"),
                email: row.get("email"),
                role: row.get("role"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            },
            organization: Organization {
                id: organization_id,
                name: row.get("organization_name"),
                slug: row.get("organization_slug"),
                created_at: row.get("organization_created_at"),
                updated_at: row.get("organization_updated_at"),
            },
            membership_role: row.get("membership_role"),
            session_token,
        }),
    ))
}

/// POST /api/sessions — Login with username + password into one Organization.
pub async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> CookieApiResult<LoginResponse> {
    let user = sqlx::query(
        "SELECT id, name, username, email, role, password_hash, created_at, updated_at \
         FROM users WHERE lower(username) = lower($1)",
    )
    .bind(req.username.trim())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_database_error)?;

    let encoded = user
        .as_ref()
        .map(|row| row.get::<String, _>("password_hash"));
    let password = req.password.clone();
    let valid = tokio::task::spawn_blocking(move || {
        verify_password_or_dummy(&password, encoded.as_deref())
    })
    .await
    .map_err(|_| internal_password_error())?;
    if !valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(PlatformError::unauthorized("invalid username or password")),
        ));
    }
    let user = user.expect("password verification requires an existing user");
    let user_id: Uuid = user.get("id");

    let organization = sqlx::query(
        "SELECT o.id, o.name, o.slug, o.created_at, o.updated_at, m.role AS membership_role \
         FROM memberships m JOIN organizations o ON o.id = m.organization_id \
         WHERE m.user_id = $1 AND ($2::uuid IS NULL OR o.id = $2) \
         ORDER BY m.created_at, m.id LIMIT 1",
    )
    .bind(user_id)
    .bind(req.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_database_error)?
    .ok_or_else(|| {
        (
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "user is not a member of the requested Organization",
            )),
        )
    })?;

    let existing_hash: String = user.get("password_hash");
    if needs_rehash(&existing_hash) {
        let password = req.password.clone();
        let replacement = tokio::task::spawn_blocking(move || hash_password(&password))
            .await
            .map_err(|_| internal_password_error())?
            .map_err(|_| internal_password_error())?;
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
            .bind(replacement)
            .bind(user_id)
            .execute(&state.db)
            .await
            .map_err(internal_database_error)?;
    }

    let session_token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let token_hash = hex::encode(Sha256::digest(session_token.as_bytes()));

    sqlx::query(
        "INSERT INTO sessions (user_id, organization_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '7 days')",
    )
    .bind(user_id)
    .bind(organization.get::<Uuid, _>("id"))
    .bind(&token_hash)
    .execute(&state.db)
    .await
    .map_err(internal_database_error)?;

    Ok((
        AppendHeaders([(header::SET_COOKIE, session_cookie(&session_token))]),
        Json(LoginResponse {
            user: User {
                id: user_id,
                name: user.get("name"),
                username: user.get("username"),
                email: user.get("email"),
                role: user.get("role"),
                created_at: user.get("created_at"),
                updated_at: user.get("updated_at"),
            },
            organization: organization_from_row(&organization),
            membership_role: organization.get("membership_role"),
            session_token,
        }),
    ))
}

/// DELETE /api/sessions/current — revoke the authenticated session and clear
/// the HttpOnly browser cookie used by same-origin resource requests.
pub async fn delete_session(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<(AppendHeaders<[(HeaderName, String); 1]>, StatusCode), (StatusCode, Json<PlatformError>)>
{
    sqlx::query(
        "UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(auth.session_id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await
    .map_err(internal_database_error)?;
    Ok((
        AppendHeaders([(header::SET_COOKIE, clear_session_cookie())]),
        StatusCode::NO_CONTENT,
    ))
}

pub(super) fn session_cookie(token: &str) -> String {
    format!(
        "session_token={token}; Path=/api/; HttpOnly; SameSite=Strict; Max-Age=604800{}",
        secure_cookie_attribute(),
    )
}

fn clear_session_cookie() -> String {
    format!(
        "session_token=; Path=/api/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        secure_cookie_attribute(),
    )
}

fn secure_cookie_attribute() -> &'static str {
    match std::env::var("OPEN_WEB_CODEX_SECURE_COOKIES") {
        Ok(value)
            if matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            ) =>
        {
            "; Secure"
        }
        _ => "",
    }
}

/// PUT /api/sessions/organization — change only the current session's
/// Organization after proving membership.
pub async fn select_organization(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(req): Json<SelectOrganizationRequest>,
) -> ApiResult<SessionOrganization> {
    let organization = sqlx::query(
        "SELECT o.id, o.name, o.slug, o.created_at, o.updated_at, m.role AS membership_role \
         FROM memberships m JOIN organizations o ON o.id = m.organization_id \
         WHERE m.user_id = $1 AND m.organization_id = $2",
    )
    .bind(auth.user_id)
    .bind(req.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_database_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Organization was not found")),
        )
    })?;

    let updated = sqlx::query(
        "UPDATE sessions SET organization_id = $1 \
         WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(req.organization_id)
    .bind(auth.session_id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await
    .map_err(internal_database_error)?;
    if updated.rows_affected() != 1 {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(PlatformError::unauthorized("session is no longer active")),
        ));
    }

    Ok(Json(SessionOrganization {
        organization: organization_from_row(&organization),
        role: organization.get("membership_role"),
    }))
}

fn organization_from_row(row: &sqlx::postgres::PgRow) -> Organization {
    Organization {
        id: row.get("id"),
        name: row.get("name"),
        slug: row.get("slug"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn internal_database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}

fn internal_password_error() -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("password verification failed")),
    )
}

#[cfg(test)]
mod tests {
    use super::{clear_session_cookie, session_cookie};

    #[test]
    fn browser_session_cookies_are_http_only_and_api_scoped() {
        let cookie = session_cookie("test-token");
        assert!(cookie.starts_with("session_token=test-token;"));
        assert!(cookie.contains("Path=/api/"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(!cookie.contains("Domain="));

        let cleared = clear_session_cookie();
        assert!(cleared.starts_with("session_token=;"));
        assert!(cleared.contains("Max-Age=0"));
    }
}
