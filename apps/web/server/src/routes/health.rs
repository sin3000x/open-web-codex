use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use open_web_codex_adapter::CodexAdapter;
use open_web_codex_platform_contracts::HealthResponse;
use open_web_codex_platform_store::AppState;
use std::sync::Arc;

/// GET /api/health — returns server status and uptime.
pub async fn health_check(
    State(state): State<AppState>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> (StatusCode, Json<HealthResponse>) {
    let uptime = state.started_at.elapsed().as_secs();
    let runtime_ok = adapter.health().await.is_ok_and(|status| status.ok);

    let status = if runtime_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(HealthResponse {
            ok: runtime_ok,
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: state.started_at_utc,
            uptime_seconds: uptime,
        }),
    )
}
