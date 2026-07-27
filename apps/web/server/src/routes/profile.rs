use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter, ProfileQuery};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    LocalUsageDay, LocalUsageQuery, LocalUsageSnapshot, LocalUsageTotals, ProfileListQuery,
    ProfileLoginCancelResponse, ProfileLoginStartResponse, ProfileLoginStatusResponse,
    ProfileProjection,
};
use open_web_codex_platform_store::AppState;
use open_web_codex_provider_service::secured::{AuthorizedProviderOperations, ProviderActor};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::{require_runtime_profile, AuthenticatedUser};
use crate::routes::RuntimeProfileBinding;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<PlatformError>)>;

pub async fn account(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    query(adapter, ProfileQuery::Account).await
}

pub async fn rate_limits(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    query(adapter, ProfileQuery::RateLimits).await
}

#[derive(Clone, Copy, Default)]
struct UsageBreakdown {
    input: u64,
    cached: u64,
    output: u64,
    total: u64,
}

pub async fn usage(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(params): Query<LocalUsageQuery>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<LocalUsageSnapshot> {
    authorize_profile(&state, &auth, &profile).await?;
    let day_count = params.days.unwrap_or(30).clamp(1, 90);
    let mut rows = if let Some(workspace_id) = params.workspace_id {
        let is_project = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND organization_id = $2)",
        )
        .bind(workspace_id)
        .bind(auth.organization_id)
        .fetch_one(&state.db)
        .await
        .map_err(database_error)?;
        if is_project {
            sqlx::query(
                "SELECT event.run_id, event.event_type, event.created_at, event.payload \
                 FROM run_events event JOIN runs run ON run.id = event.run_id \
                 JOIN tasks task ON task.id = run.task_id \
                 WHERE run.organization_id = $1 AND run.requested_by = $2 \
                   AND task.project_id = $3 \
                   AND event.event_type IN ('codex.thread.token_usage.updated', 'codex.turn.completed') \
                 ORDER BY event.run_id, event.created_at LIMIT 100000",
            )
            .bind(auth.organization_id)
            .bind(auth.user_id)
            .bind(workspace_id)
            .fetch_all(&state.db)
            .await
            .map_err(database_error)?
        } else {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE id = $1 AND organization_id = $2 \
                 AND requested_by = $3 AND workspace_kind <> 'main')",
            )
            .bind(workspace_id)
            .bind(auth.organization_id)
            .bind(auth.user_id)
            .fetch_one(&state.db)
            .await
            .map_err(database_error)?;
            if !exists {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(PlatformError::not_found("Usage workspace was not found")),
                ));
            }
            sqlx::query(
                "SELECT event.run_id, event.event_type, event.created_at, event.payload \
                 FROM run_events event JOIN runs run ON run.id = event.run_id \
                 WHERE run.organization_id = $1 AND run.requested_by = $2 \
                   AND (run.id = $3 OR run.workspace_group_run_id = $3) \
                   AND event.event_type IN ('codex.thread.token_usage.updated', 'codex.turn.completed') \
                 ORDER BY event.run_id, event.created_at LIMIT 100000",
            )
            .bind(auth.organization_id)
            .bind(auth.user_id)
            .bind(workspace_id)
            .fetch_all(&state.db)
            .await
            .map_err(database_error)?
        }
    } else {
        sqlx::query(
            "SELECT event.run_id, event.event_type, event.created_at, event.payload \
             FROM run_events event JOIN runs run ON run.id = event.run_id \
             WHERE run.organization_id = $1 AND run.requested_by = $2 \
               AND event.event_type IN ('codex.thread.token_usage.updated', 'codex.turn.completed') \
             ORDER BY event.run_id, event.created_at LIMIT 100000",
        )
        .bind(auth.organization_id)
        .bind(auth.user_id)
        .fetch_all(&state.db)
        .await
        .map_err(database_error)?
    };

    let mut cumulative: HashMap<Uuid, BTreeMap<String, UsageBreakdown>> = HashMap::new();
    let mut run_counts: BTreeMap<String, u64> = BTreeMap::new();
    for row in rows.drain(..) {
        let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
        let day = created_at.date_naive().to_string();
        if row.get::<String, _>("event_type") == "codex.turn.completed" {
            *run_counts.entry(day).or_default() += 1;
            continue;
        }
        let payload: Value = row.get("payload");
        let Some(total) = payload.pointer("/data/tokenUsage/total") else {
            continue;
        };
        let breakdown = UsageBreakdown {
            input: unsigned(total.get("inputTokens")),
            cached: unsigned(total.get("cachedInputTokens")),
            output: unsigned(total.get("outputTokens")),
            total: unsigned(total.get("totalTokens")),
        };
        cumulative
            .entry(row.get("run_id"))
            .or_default()
            .insert(day, breakdown);
    }
    let mut daily: BTreeMap<String, UsageBreakdown> = BTreeMap::new();
    for buckets in cumulative.values() {
        let mut previous = UsageBreakdown::default();
        for (day, current) in buckets {
            let entry = daily.entry(day.clone()).or_default();
            entry.input += counter_delta(current.input, previous.input);
            entry.cached += counter_delta(current.cached, previous.cached);
            entry.output += counter_delta(current.output, previous.output);
            entry.total += counter_delta(current.total, previous.total);
            previous = *current;
        }
    }

    if daily.is_empty() && params.workspace_id.is_none() {
        if let Ok(fallback) = adapter.query_profile(ProfileQuery::Usage).await {
            for bucket in fallback
                .get("dailyUsageBuckets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let (Some(day), Some(tokens)) = (
                    bucket.get("startDate").and_then(Value::as_str),
                    bucket.get("tokens").and_then(Value::as_u64),
                ) {
                    daily.insert(
                        day.to_string(),
                        UsageBreakdown {
                            total: tokens,
                            ..UsageBreakdown::default()
                        },
                    );
                }
            }
        }
    }
    Ok(Json(build_usage_snapshot(day_count, daily, run_counts)))
}

fn unsigned(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn counter_delta(current: u64, previous: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn build_usage_snapshot(
    day_count: u32,
    daily: BTreeMap<String, UsageBreakdown>,
    run_counts: BTreeMap<String, u64>,
) -> LocalUsageSnapshot {
    let today = chrono::Utc::now().date_naive();
    let start = today - chrono::Duration::days(i64::from(day_count.saturating_sub(1)));
    let mut days = Vec::with_capacity(day_count as usize);
    for offset in 0..day_count {
        let date = start + chrono::Duration::days(i64::from(offset));
        let day = date.to_string();
        let usage = daily.get(&day).copied().unwrap_or_default();
        days.push(LocalUsageDay {
            day: day.clone(),
            input_tokens: usage.input,
            cached_input_tokens: usage.cached,
            output_tokens: usage.output,
            total_tokens: usage.total,
            agent_time_ms: 0,
            agent_runs: run_counts.get(&day).copied().unwrap_or(0),
        });
    }
    let last30_days_tokens = days.iter().map(|day| day.total_tokens).sum();
    let last7_days_tokens = days.iter().rev().take(7).map(|day| day.total_tokens).sum();
    let total_input: u64 = days.iter().map(|day| day.input_tokens).sum();
    let total_cached: u64 = days.iter().map(|day| day.cached_input_tokens).sum();
    let peak = days.iter().max_by_key(|day| day.total_tokens);
    LocalUsageSnapshot {
        updated_at: chrono::Utc::now().timestamp_millis(),
        totals: LocalUsageTotals {
            last7_days_tokens,
            last30_days_tokens,
            average_daily_tokens: last30_days_tokens / u64::from(day_count),
            cache_hit_rate_percent: if total_input == 0 {
                0.0
            } else {
                (total_cached as f64 / total_input as f64) * 100.0
            },
            peak_day: peak
                .filter(|day| day.total_tokens > 0)
                .map(|day| day.day.clone()),
            peak_day_tokens: peak.map(|day| day.total_tokens).unwrap_or(0),
        },
        days,
        top_models: Vec::new(),
    }
}

pub async fn start_login(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileLoginStartResponse> {
    authorize_profile(&state, &auth, &profile).await?;
    let login = adapter.start_profile_login().await.map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            Json(PlatformError::internal(
                "Codex Profile login could not be started",
            )),
        )
    })?;
    Ok(Json(ProfileLoginStartResponse {
        login_id: login.login_id,
        auth_url: login.auth_url,
    }))
}

pub async fn cancel_login(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileLoginCancelResponse> {
    authorize_profile(&state, &auth, &profile).await?;
    let cancellation = adapter.cancel_profile_login().await.map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            Json(PlatformError::internal(
                "Codex Profile login could not be canceled",
            )),
        )
    })?;
    Ok(Json(ProfileLoginCancelResponse {
        canceled: cancellation.canceled,
        status: cancellation.status,
    }))
}

pub async fn login_status(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(login_id): Path<String>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileLoginStatusResponse> {
    authorize_profile(&state, &auth, &profile).await?;
    let login_id = login_id.trim();
    if login_id.is_empty() || login_id.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("Invalid Profile login id")),
        ));
    }
    let status = adapter.profile_login_status(login_id).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Profile login was not found")),
        )
    })?;
    Ok(Json(ProfileLoginStatusResponse {
        completed: status.completed,
        success: status.success,
        error: status.error,
    }))
}

pub async fn collaboration_modes(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    query(adapter, ProfileQuery::CollaborationModes).await
}

pub async fn apps(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(params): Query<ProfileListQuery>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    let thread_id = optional_run_context(&state, &auth, params.run_id)
        .await?
        .and_then(|context| context.thread_id);
    query(
        adapter,
        ProfileQuery::Apps {
            cursor: params.cursor,
            limit: params.limit,
            thread_id,
        },
    )
    .await
}

pub async fn runtime_status(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(providers): Extension<Arc<dyn AuthorizedProviderOperations>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;

    let runtime = match adapter.health().await {
        Ok(status) => json!({
            "ok": status.ok,
            "name": status.name,
            "version": status.version,
        }),
        Err(_) => json!({
            "ok": false,
            "error": "runtime_health_unavailable",
        }),
    };
    let capabilities = profile.capabilities.get().await.map(|record| {
        json!({
            "serverBuild": record.server_build,
            "protocolVersion": record.protocol_version,
            "capabilityCount": record
                .manifest
                .get("capabilities")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        })
    });
    let mcp_servers = match adapter
        .query_profile(ProfileQuery::McpServers {
            cursor: None,
            limit: Some(100),
            thread_id: None,
        })
        .await
    {
        Ok(value) => json!({
            "ok": true,
            "data": sanitize_projection(value, None),
        }),
        Err(_) => json!({
            "ok": false,
            "error": "mcp_status_unavailable",
        }),
    };
    let provider_models = provider_model_diagnostics(&auth, providers).await;

    Ok(Json(ProfileProjection {
        data: json!({
            "profile": single_profile_summary(&profile),
            "runtime": runtime,
            "capabilities": capabilities,
            "mcpServers": mcp_servers,
            "providerModels": provider_models,
        }),
    }))
}

async fn provider_model_diagnostics(
    auth: &AuthenticatedUser,
    providers: Arc<dyn AuthorizedProviderOperations>,
) -> Value {
    match providers
        .list(ProviderActor {
            user_id: auth.user_id,
            organization_id: auth.organization_id,
        })
        .await
    {
        Ok(catalog) => {
            let current = catalog
                .data
                .iter()
                .find(|provider| provider.id == catalog.current_provider_id);
            json!({
                "ok": true,
                "currentProviderId": catalog.current_provider_id,
                "providerCount": catalog.data.len(),
                "current": current.map(|provider| json!({
                    "id": provider.id,
                    "name": provider.name,
                    "wireApi": provider.wire_api,
                    "kind": provider.kind,
                    "modelCount": provider.model_count,
                    "visibleModelCount": provider
                        .models
                        .iter()
                        .filter(|model| model.show_in_picker)
                        .count(),
                    "canFetchModels": provider.can_fetch_models,
                    "hasEnvKey": provider.env_key.is_some(),
                })),
                "providers": catalog
                    .data
                    .iter()
                    .map(|provider| json!({
                        "id": provider.id,
                        "name": provider.name,
                        "wireApi": provider.wire_api,
                        "kind": provider.kind,
                        "isCurrent": provider.is_current,
                        "modelCount": provider.model_count,
                        "visibleModelCount": provider
                            .models
                            .iter()
                            .filter(|model| model.show_in_picker)
                            .count(),
                        "canFetchModels": provider.can_fetch_models,
                        "hasEnvKey": provider.env_key.is_some(),
                    }))
                    .collect::<Vec<_>>(),
            })
        }
        Err(_) => json!({
            "ok": false,
            "error": "provider_model_status_unavailable",
        }),
    }
}

pub async fn mcp_servers(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(params): Query<ProfileListQuery>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    let Some(run_id) = params.run_id else {
        return Ok(Json(ProfileProjection {
            data: json!({ "data": [], "nextCursor": null }),
        }));
    };
    // Runtime startup notifications are already persisted as the durable,
    // tenant-filtered MCP status projection. Reading the latest status per
    // server avoids invoking `mcpServerStatus/list` during page hydration;
    // that inventory call also enumerates every tool/resource and can take
    // several seconds even though the sidebar only consumes startup state.
    let _context = run_context(&state, &auth, run_id).await?;
    let limit = i64::from(params.limit.unwrap_or(100).clamp(1, 100));
    let rows = sqlx::query(
        "SELECT latest.name, latest.status, latest.error, latest.failure_reason \
         FROM ( \
             SELECT DISTINCT ON (e.payload #>> '{data,name}') \
                    e.payload #>> '{data,name}' AS name, \
                    e.payload #>> '{data,status}' AS status, \
                    NULLIF(e.payload #>> '{data,error}', '') AS error, \
                    NULLIF(e.payload #>> '{data,failureReason}', '') AS failure_reason \
             FROM run_events e \
             WHERE e.run_id = $1 \
               AND e.payload #>> '{data,sourceType}' = 'mcpServer/startupStatus/updated' \
               AND COALESCE(e.payload #>> '{data,name}', '') <> '' \
             ORDER BY e.payload #>> '{data,name}', e.sequence DESC \
         ) latest ORDER BY latest.name LIMIT $2",
    )
    .bind(run_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(database_error)?;
    let data = rows
        .iter()
        .map(|row| {
            json!({
                "name": row.get::<String, _>("name"),
                "status": row.get::<String, _>("status"),
                "error": row.get::<Option<String>, _>("error"),
                "failureReason": row.get::<Option<String>, _>("failure_reason"),
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(ProfileProjection {
        data: json!({ "data": data, "nextCursor": null }),
    }))
}

fn single_profile_summary(profile: &RuntimeProfileBinding) -> Value {
    let codex_home_fingerprint = profile.codex_home.as_deref().map(|path| {
        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        format!("{digest:x}")[..16].to_string()
    });
    json!({
        "runtimeKey": profile.runtime_key,
        "name": profile.name,
        "hasCodexHome": profile.codex_home.is_some(),
        "codexHomeFingerprint": codex_home_fingerprint,
    })
}

pub async fn experimental_features(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(params): Query<ProfileListQuery>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    let thread_id = optional_run_context(&state, &auth, params.run_id)
        .await?
        .and_then(|context| context.thread_id);
    query(
        adapter,
        ProfileQuery::ExperimentalFeatures {
            cursor: params.cursor,
            limit: params.limit,
            thread_id,
        },
    )
    .await
}

pub async fn skills(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(params): Query<ProfileListQuery>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileProjection> {
    authorize_profile(&state, &auth, &profile).await?;
    let run_id = params.run_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(PlatformError::bad_request("runId is required")),
        )
    })?;
    let context = run_context(&state, &auth, run_id).await?;
    query(
        adapter,
        ProfileQuery::Skills {
            workspace: context.workspace,
            force_reload: params.force_reload.unwrap_or(false),
        },
    )
    .await
}

async fn authorize_profile(
    state: &AppState,
    auth: &AuthenticatedUser,
    profile: &RuntimeProfileBinding,
) -> Result<(), (StatusCode, Json<PlatformError>)> {
    require_runtime_profile(&state.db, auth, &profile.runtime_key).await
}

async fn query(
    adapter: Arc<dyn CodexAdapter>,
    request: ProfileQuery,
) -> ApiResult<ProfileProjection> {
    let value = adapter.query_profile(request).await.map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            Json(PlatformError::internal("Codex Profile query failed")),
        )
    })?;
    Ok(Json(ProfileProjection {
        data: sanitize_projection(value, None),
    }))
}

struct RunContext {
    thread_id: Option<String>,
    workspace: AuthorizedWorkspace,
}

async fn optional_run_context(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Option<Uuid>,
) -> Result<Option<RunContext>, (StatusCode, Json<PlatformError>)> {
    match run_id {
        Some(run_id) => run_context(state, auth, run_id).await.map(Some),
        None => Ok(None),
    }
}

async fn run_context(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<RunContext, (StatusCode, Json<PlatformError>)> {
    let row = sqlx::query(
        "SELECT r.codex_thread_id, r.workspace_id, r.requested_by, w.root_path, w.state \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
         WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Run workspace was not found")),
        )
    })?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(PlatformError::not_found("Run workspace was not found")),
        ));
    }
    if row.get::<String, _>("state") == "retired" {
        return Err((
            StatusCode::CONFLICT,
            Json(PlatformError::bad_request("Run workspace has been retired")),
        ));
    }
    let workspace_id: Uuid = row.get("workspace_id");
    Ok(RunContext {
        thread_id: row.get("codex_thread_id"),
        workspace: AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: row.get::<String, _>("root_path").into(),
        },
    })
}

fn sanitize_projection(value: Value, parent_name: Option<&str>) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| sanitize_projection(value, None))
                .collect(),
        ),
        Value::Object(values) => {
            let name = values
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut safe = Map::new();
            for (key, value) in values {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if matches!(
                    normalized.as_str(),
                    "authorization"
                        | "cookie"
                        | "credential"
                        | "password"
                        | "secret"
                        | "apikey"
                        | "accesstoken"
                        | "refreshtoken"
                ) {
                    continue;
                }
                if matches!(normalized.as_str(), "path" | "cwd" | "codexhome") {
                    let label = name.as_deref().or(parent_name).unwrap_or("resource");
                    safe.insert(key, Value::String(format!("profile://{label}")));
                } else {
                    safe.insert(
                        key,
                        sanitize_projection(value, name.as_deref().or(parent_name)),
                    );
                }
            }
            Value::Object(safe)
        }
        Value::String(value) if value.starts_with("data:") => {
            Value::String("[embedded-data]".to_string())
        }
        other => other,
    }
}

fn database_error(_error: sqlx::Error) -> (StatusCode, Json<PlatformError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        build_usage_snapshot, sanitize_projection, single_profile_summary, UsageBreakdown,
    };
    use crate::routes::{RuntimeCapabilityState, RuntimeProfileBinding};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn profile_projection_removes_secrets_and_server_paths() {
        let safe = sanitize_projection(
            json!({
                "data": [{
                    "name": "review",
                    "path": "/srv/profiles/user/skills/review/SKILL.md",
                    "apiKey": "secret",
                    "description": "Review code"
                }]
            }),
            None,
        );
        let text = serde_json::to_string(&safe).unwrap();
        assert!(!text.contains("/srv/profiles"));
        assert!(!text.contains("secret"));
        assert_eq!(safe["data"][0]["path"], "profile://review");
    }

    #[test]
    fn single_profile_summary_fingerprints_without_exposing_home_path() {
        let profile = RuntimeProfileBinding {
            runtime_key: "default-profile".to_string(),
            name: "Default".to_string(),
            codex_home: Some(Arc::new(PathBuf::from("/srv/private/codex-home"))),
            capabilities: RuntimeCapabilityState::default(),
        };

        let summary = single_profile_summary(&profile);
        let text = serde_json::to_string(&summary).unwrap();

        assert_eq!(summary["runtimeKey"], "default-profile");
        assert_eq!(summary["hasCodexHome"], true);
        assert!(summary["codexHomeFingerprint"].as_str().is_some());
        assert!(!text.contains("/srv/private/codex-home"));
    }

    #[test]
    fn usage_snapshot_preserves_daily_token_breakdown() {
        let today = chrono::Utc::now().date_naive().to_string();
        let mut daily = BTreeMap::new();
        daily.insert(
            today.clone(),
            UsageBreakdown {
                input: 100,
                cached: 25,
                output: 50,
                total: 150,
            },
        );
        let mut runs = BTreeMap::new();
        runs.insert(today.clone(), 2);
        let snapshot = build_usage_snapshot(7, daily, runs);
        let current = snapshot.days.last().unwrap();
        assert_eq!(current.day, today);
        assert_eq!(current.input_tokens, 100);
        assert_eq!(current.cached_input_tokens, 25);
        assert_eq!(current.output_tokens, 50);
        assert_eq!(current.agent_runs, 2);
        assert_eq!(snapshot.totals.last7_days_tokens, 150);
        assert_eq!(snapshot.totals.peak_day_tokens, 150);
    }
}
