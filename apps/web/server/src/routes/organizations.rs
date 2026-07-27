use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    AddMemberRequest, CreateOrganizationRequest, MemberInfo, Membership, Organization,
};
use open_web_codex_platform_store::AppState;
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

/// GET /api/organizations — list orgs the current user belongs to.
pub async fn list_organizations(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<Vec<Organization>> {
    let rows = sqlx::query(
        "SELECT o.id, o.name, o.slug, o.created_at, o.updated_at \
         FROM organizations o JOIN memberships m ON m.organization_id = o.id \
         WHERE m.user_id = $1 ORDER BY o.created_at",
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("db error: {e}"))),
        )
    })?;

    let orgs: Vec<Organization> = rows
        .iter()
        .map(|r| Organization {
            id: r.get("id"),
            name: r.get("name"),
            slug: r.get("slug"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();

    Ok(Json(orgs))
}

/// POST /api/organizations — create a new org (creator becomes owner).
pub async fn create_organization(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Json(req): Json<CreateOrganizationRequest>,
) -> ApiResult<Organization> {
    if req.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("name must not be empty")),
        ));
    }

    let slug = req.slug.unwrap_or_else(|| {
        req.name
            .to_lowercase()
            .replace(' ', "-")
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "")
    });

    let mut transaction = state.db.begin().await.map_err(internal_database_error)?;
    let org = sqlx::query(
        "INSERT INTO organizations (name, slug) VALUES ($1, $2) \
         RETURNING id, name, slug, created_at, updated_at",
    )
    .bind(&req.name)
    .bind(&slug)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(format!(
                "slug '{slug}' is unavailable"
            ))),
        )
    })?;

    // Add creator as owner
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(org.get::<Uuid, _>("id"))
    .bind(auth.user_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_database_error)?;
    transaction
        .commit()
        .await
        .map_err(internal_database_error)?;

    Ok(Json(Organization {
        id: org.get("id"),
        name: org.get("name"),
        slug: org.get("slug"),
        created_at: org.get("created_at"),
        updated_at: org.get("updated_at"),
    }))
}

/// GET /api/organizations/:id — get org details (member-only).
pub async fn get_organization(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Organization> {
    // Verify membership
    let member: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memberships WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?;

    if member == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("organization not found")),
        ));
    }

    let row = sqlx::query(
        "SELECT id, name, slug, created_at, updated_at FROM organizations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("organization not found")),
        )
    })?;

    Ok(Json(Organization {
        id: row.get("id"),
        name: row.get("name"),
        slug: row.get("slug"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

/// GET /api/organizations/:id/members — list members.
pub async fn list_members(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<MemberInfo>> {
    // Verify membership
    let member: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memberships WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?;

    if member == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("organization not found")),
        ));
    }

    let rows = sqlx::query(
        "SELECT u.id, u.name, u.email, m.role \
         FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.organization_id = $1 ORDER BY m.created_at",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?;

    let members: Vec<MemberInfo> = rows
        .iter()
        .map(|r| MemberInfo {
            user_id: r.get("id"),
            name: r.get("name"),
            email: r.get("email"),
            role: r.get("role"),
        })
        .collect();

    Ok(Json(members))
}

/// POST /api/organizations/:id/members — add a member (owner/admin only).
pub async fn add_member(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<AddMemberRequest>,
) -> ApiResult<Membership> {
    // Check caller's role
    let caller_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM memberships WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PlatformError::internal(format!("{e}"))),
        )
    })?;

    let caller_role = caller_role.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("organization not found")),
        )
    })?;

    if caller_role != "owner" && caller_role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "only owners and admins can add members",
            )),
        ));
    }

    // Find user by email
    let target_user: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&req.email)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(PlatformError::internal(format!("{e}"))),
            )
        })?;

    let (target_id,) = target_user.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("user not found")),
        )
    })?;

    let target_role = req.role.unwrap_or_else(|| "member".to_string());
    let allowed = match target_role.as_str() {
        "member" => caller_role == "owner" || caller_role == "admin",
        "admin" | "owner" => caller_role == "owner",
        _ => false,
    };
    if !allowed {
        return Err((
            StatusCode::FORBIDDEN,
            Json(PlatformError::forbidden(
                "caller cannot grant the requested Organization role",
            )),
        ));
    }

    let row = sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, $3) \
         RETURNING id, organization_id, user_id, role, created_at",
    )
    .bind(id)
    .bind(target_id)
    .bind(&target_role)
    .fetch_one(&state.db)
    .await
    .map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request(
                "user is already a member or the role is invalid",
            )),
        )
    })?;

    Ok(Json(Membership {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        user_id: row.get("user_id"),
        role: row.get("role"),
        created_at: row.get("created_at"),
    }))
}

fn internal_database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}
