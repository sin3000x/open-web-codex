use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_store::AppState;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;

// This is a server memory-safety boundary for all cached MCP Resources, not a
// map-card contract limit. Cards themselves carry only refs for large data.
const MAX_MCP_RESOURCE_BYTES: usize = 128 * 1024 * 1024;

pub async fn read(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((run_id, artifact_id)): Path<(Uuid, Uuid)>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
) -> ApiResult<Value> {
    let row = sqlx::query(
        "SELECT a.thread_id, a.source_server, a.source_uri, a.mime_type,
                a.expected_size, a.content,
                r.requested_by, w.id AS workspace_id, w.root_path, w.state AS workspace_state
         FROM reply_artifacts a
         JOIN runs r ON r.id = a.run_id
         JOIN workspaces w ON w.id = r.workspace_id
         WHERE a.id = $1 AND a.run_id = $2
           AND a.organization_id = $3 AND r.organization_id = $3
           AND w.organization_id = $3 AND a.state IN ('pending', 'ready')",
    )
    .bind(artifact_id)
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;

    let requested_by: Option<Uuid> = row.get("requested_by");
    if row.get::<String, _>("workspace_state") == "retired"
        || (requested_by != Some(auth.user_id)
            && !matches!(auth.organization_role.as_str(), "owner" | "admin"))
    {
        return Err(not_found());
    }

    let content: Option<Vec<u8>> = row.get("content");
    let bytes = if let Some(content) = content {
        content
    } else {
        if row
            .get::<Option<i64>, _>("expected_size")
            .is_some_and(|size| {
                usize::try_from(size)
                    .map(|size| size > MAX_MCP_RESOURCE_BYTES)
                    .unwrap_or(true)
            })
        {
            return Err(payload_too_large());
        }
        let thread_id: String = row.get("thread_id");
        let source_server: String = row.get("source_server");
        let source_uri: String = row.get("source_uri");
        let workspace = AuthorizedWorkspace {
            id: row.get::<Uuid, _>("workspace_id").to_string(),
            root: row.get::<String, _>("root_path").into(),
        };
        let response = adapter
            .read_mcp_resource(&workspace, &thread_id, &source_server, &source_uri)
            .await
            .map_err(runtime_error)?;
        let bytes = resource_bytes(&response, &source_uri)?;
        if bytes.len() > MAX_MCP_RESOURCE_BYTES {
            return Err(payload_too_large());
        }
        let digest = hex::encode(Sha256::digest(&bytes));
        sqlx::query(
            "UPDATE reply_artifacts
             SET content = $1, content_sha256 = $2, state = 'ready', updated_at = now()
             WHERE id = $3 AND run_id = $4 AND state = 'pending'",
        )
        .bind(&bytes)
        .bind(digest)
        .bind(artifact_id)
        .bind(run_id)
        .execute(&state.db)
        .await
        .map_err(database_error)?;
        bytes
    };

    if bytes.len() > MAX_MCP_RESOURCE_BYTES {
        return Err(payload_too_large());
    }
    let mime_type = row
        .get::<Option<String>, _>("mime_type")
        .unwrap_or_else(|| "application/geo+json".to_string());
    if !matches!(
        mime_type.as_str(),
        "application/geo+json" | "application/json"
    ) {
        return Err(bad_gateway("MCP Resource is not GeoJSON"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| bad_gateway("MCP Resource did not contain valid JSON"))?;
    if !valid_geojson_root(&value) {
        return Err(bad_gateway("MCP Resource did not contain GeoJSON"));
    }
    Ok(Json(value))
}

fn resource_bytes(response: &Value, expected_uri: &str) -> Result<Vec<u8>, ApiError> {
    let contents = response
        .get("contents")
        .and_then(Value::as_array)
        .ok_or_else(|| bad_gateway("mcpServer/resource/read omitted contents"))?;
    let content = contents
        .iter()
        .find(|content| content.get("uri").and_then(Value::as_str) == Some(expected_uri))
        .ok_or_else(|| bad_gateway("MCP Resource response did not match the requested URI"))?;
    if !matches!(
        content.get("mimeType").and_then(Value::as_str),
        None | Some("application/geo+json" | "application/json")
    ) {
        return Err(bad_gateway("MCP Resource content type was unsupported"));
    }
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(blob) = content.get("blob").and_then(Value::as_str) {
        return BASE64
            .decode(blob)
            .map_err(|_| bad_gateway("MCP Resource blob was not valid base64"));
    }
    Err(bad_gateway("MCP Resource content was unsupported"))
}

fn valid_geojson_root(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some(
            "FeatureCollection"
                | "Feature"
                | "GeometryCollection"
                | "Point"
                | "MultiPoint"
                | "LineString"
                | "MultiLineString"
                | "Polygon"
                | "MultiPolygon"
        )
    )
}

fn not_found() -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(PlatformError::not_found("Reply Artifact was not found")),
    )
}

fn payload_too_large() -> ApiError {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        Json(PlatformError::bad_request(
            "MCP Resource exceeds the server safety limit",
        )),
    )
}

fn bad_gateway(message: &str) -> ApiError {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal(message)),
    )
}

fn runtime_error(_: open_web_codex_adapter::AdapterError) -> ApiError {
    bad_gateway("MCP Resource read failed")
}

fn database_error(_: sqlx::Error) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("Database operation failed")),
    )
}

#[cfg(test)]
mod tests {
    use super::{resource_bytes, valid_geojson_root};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use serde_json::json;

    #[test]
    fn reads_text_and_blob_mcp_resource_contents() {
        let text = resource_bytes(
            &json!({
                "contents": [{
                    "uri": "maps-data://geojson/one",
                    "mimeType": "application/geo+json",
                    "text": "{\"type\":\"FeatureCollection\",\"features\":[]}"
                }]
            }),
            "maps-data://geojson/one",
        )
        .expect("text Resource");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&text).unwrap()["type"],
            "FeatureCollection"
        );

        let encoded = BASE64.encode(b"{\"type\":\"Point\",\"coordinates\":[1,2]}");
        let blob = resource_bytes(
            &json!({
                "contents": [{
                    "uri": "maps-data://geojson/two",
                    "blob": encoded
                }]
            }),
            "maps-data://geojson/two",
        )
        .expect("blob Resource");
        assert!(valid_geojson_root(
            &serde_json::from_slice(&blob).expect("valid JSON")
        ));
    }

    #[test]
    fn rejects_non_geojson_json_roots() {
        assert!(!valid_geojson_root(&json!({ "type": "table" })));
        assert!(valid_geojson_root(&json!({
            "type": "FeatureCollection",
            "features": []
        })));
    }
}
