use crate::middleware::auth::AuthenticatedUser;
use axum::Json;
use open_web_codex_platform_contracts::MeResponse;

/// GET /api/me
pub async fn me(auth: AuthenticatedUser) -> Json<MeResponse> {
    Json(MeResponse {
        id: auth.user_id,
        name: auth.name,
        username: auth.username,
        email: auth.email,
        role: auth.role,
        organization_id: auth.organization_id,
        organization_role: auth.organization_role,
    })
}
