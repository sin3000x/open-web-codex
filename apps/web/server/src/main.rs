mod event_projection;
mod middleware;
mod routes;
#[cfg(test)]
mod security_integration;

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use clap::Parser;
use open_web_codex_approval_service::{ApprovalService, ResolvedApproval};
use open_web_codex_git_runtime::{GitRuntime, GitRuntimeConfig};
use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig};
use open_web_codex_profile_registry::ProfileRegistry;
use open_web_codex_provider_service::secured::{
    AuthorizedProviderOperations, InMemoryAuthorizedProviderService, SecuredProviderService,
};
use open_web_codex_run_orchestrator::RunOrchestrator;
use open_web_codex_secret_store::{MasterKey, PostgresSecretStore, SecretCipher};
use rand::{distributions::Alphanumeric, Rng};
use sqlx::Row;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use open_web_codex_adapter::{fake::FakeCodexAdapter, real::RealCodexAdapter, CodexAdapter};
use open_web_codex_auth::hash_password;
use open_web_codex_platform_store::AppState;

#[derive(Parser, Debug)]
#[command(
    name = "open-web-codex-server",
    about = "open-web-codex platform server"
)]
struct Cli {
    /// Address to bind the HTTP server.
    #[arg(long, default_value = "127.0.0.1:4800")]
    bind: SocketAddr,

    /// PostgreSQL connection string.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    /// Maximum PostgreSQL connections.
    #[arg(long, env = "DATABASE_MAX_CONNECTIONS", default_value_t = 10)]
    database_max_connections: u32,

    /// Run database migrations on startup.
    #[arg(long, default_value_t = true)]
    migrate: bool,
    /// Codex adapter mode: "fake" (in-memory) or "real" (native Profile Host).
    #[arg(long, env = "CODEX_MODE", default_value = "real")]
    codex_mode: String,
    /// Persistent Profile home used by the native Codex app-server.
    #[arg(long, env = "CODEX_HOME")]
    codex_home: Option<PathBuf>,
    /// Single-Profile transition: import file-backed Codex login from this home when the Profile has no auth.json.
    #[arg(long, env = "OPEN_WEB_CODEX_IMPORT_CODEX_AUTH_FROM")]
    import_codex_auth_from: Option<PathBuf>,
    /// Codex executable used by the native Profile Host.
    #[arg(long, env = "CODEX_BIN", default_value = "codex")]
    codex_bin: PathBuf,
    /// Private root for server-owned repository mirrors and Run workspaces.
    #[arg(
        long,
        env = "OPEN_WEB_CODEX_RUNNER_ROOT",
        default_value = ".open-web-codex/runner"
    )]
    runner_root: PathBuf,
    /// Permit local filesystem Git sources. Intended only for isolated tests.
    #[arg(long, env = "OPEN_WEB_CODEX_ALLOW_LOCAL_GIT_SOURCES", default_value_t = false, action = clap::ArgAction::SetTrue)]
    allow_local_git_sources: bool,
    /// Built browser assets served by this process. API-only mode is used
    /// when the directory does not exist.
    #[arg(long, env = "OPEN_WEB_CODEX_WEB_DIST", default_value = "dist")]
    web_dist: PathBuf,
    /// Stable Profile identity for the transitional single-Profile flow.
    #[arg(long, env = "CODEX_PROFILE_ID", default_value = "default-profile")]
    profile_id: String,
    /// Stable workspace identity for event and Thread routing.
    #[arg(long, env = "CODEX_WORKSPACE_ID", default_value = "default-workspace")]
    workspace_id: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    tracing::info!(bind = %cli.bind, "starting open-web-codex server");

    // Connect to PostgreSQL
    let pool = open_web_codex_platform_store::connect(
        &cli.database_url.clone().unwrap_or_else(|| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
            format!("postgres://{user}@localhost:5432/open_web_codex")
        }),
        cli.database_max_connections,
    )
    .await?;
    tracing::info!("connected to PostgreSQL");

    // Run migrations
    if cli.migrate {
        open_web_codex_platform_store::migrate::run(&pool).await?;
        tracing::info!("database migrations complete");
    }

    let state = AppState::new(pool);
    let mut git_config = GitRuntimeConfig::new(cli.runner_root.clone());
    if cli.allow_local_git_sources {
        tracing::warn!("local filesystem Git sources are enabled");
        git_config = git_config.with_local_sources();
    }
    let git = Arc::new(GitRuntime::new(git_config)?);
    let profile_binding = routes::RuntimeProfileBinding {
        runtime_key: cli.profile_id.clone(),
        name: cli.profile_id.clone(),
        codex_home: cli.codex_home.clone().map(Arc::new),
        capabilities: routes::RuntimeCapabilityState::default(),
    };
    ensure_local_owner(&state.db).await?;
    ensure_transitional_profile_binding(
        &state.db,
        &profile_binding.runtime_key,
        &profile_binding.name,
    )
    .await?;
    let master_key = match std::env::var("OPEN_WEB_CODEX_MASTER_KEY") {
        Ok(value) => MasterKey::from_base64(&value)?,
        Err(_) if cli.codex_mode == "real" => {
            return Err(anyhow::anyhow!(
                "OPEN_WEB_CODEX_MASTER_KEY is required in real Codex mode"
            ));
        }
        Err(_) => MasterKey::generate()?,
    };
    let key_version =
        std::env::var("OPEN_WEB_CODEX_MASTER_KEY_VERSION").unwrap_or_else(|_| "v1".to_string());
    let configuration_secrets = Arc::new(PostgresSecretStore::new(
        state.db.clone(),
        SecretCipher::new(master_key, key_version)?,
    ));
    let (adapter, providers): (Arc<dyn CodexAdapter>, Arc<dyn AuthorizedProviderOperations>) =
        match cli.codex_mode.as_str() {
            "fake" => {
                tracing::info!("starting in fake codex mode");
                (
                    Arc::new(FakeCodexAdapter::new().with_demo_workspace().await),
                    Arc::new(InMemoryAuthorizedProviderService::default()),
                )
            }
            "real" => {
                let codex_home = cli.codex_home.clone().ok_or_else(|| {
                    anyhow::anyhow!("--codex-home / CODEX_HOME is required in real Codex mode")
                })?;
                prepare_single_profile_auth_import(
                    cli.import_codex_auth_from.as_deref(),
                    &codex_home,
                )?;
                let workspace_root = git.workspace_root().to_path_buf();
                tracing::info!(
                    profile_id = %cli.profile_id,
                    workspace_id = %cli.workspace_id,
                    workspace_root = %workspace_root.display(),
                    "starting native Codex Profile Host"
                );
                let registry = ProfileRegistry::new();
                let providers = SecuredProviderService::new(
                    state.db.clone(),
                    profile_binding.runtime_key.clone(),
                    registry.clone(),
                    configuration_secrets.as_ref().clone(),
                );
                let secret_environment = providers.startup_secret_environment().await?;
                let host_config =
                    ProfileHostConfig::new(cli.profile_id.clone(), codex_home, workspace_root)
                        .with_codex_bin(cli.codex_bin.clone());
                let workspace_root = host_config.workspace_root.clone();
                let host = registry
                    .register_with_secret_environment(host_config, secret_environment)
                    .await?;
                let capabilities = profile_capability_record(&host).await?;
                profile_binding.capabilities.set(capabilities.clone()).await;
                persist_profile_capabilities(
                    &state.db,
                    &profile_binding.runtime_key,
                    &capabilities,
                )
                .await?;
                let real =
                    RealCodexAdapter::from_host(host, cli.workspace_id.clone(), workspace_root)?;
                (Arc::new(real), Arc::new(providers))
            }
            other => anyhow::bail!("unknown --codex-mode '{other}'; expected 'fake' or 'real'"),
        };
    let approvals = Arc::new(ApprovalService::new(
        state.db.clone(),
        profile_binding.runtime_key.clone(),
    ));
    let orchestrator = Arc::new(RunOrchestrator::new(
        state.db.clone(),
        git.clone(),
        adapter.clone(),
        profile_binding.runtime_key.clone(),
        format!("server-{}", uuid::Uuid::now_v7()),
        std::time::Duration::from_secs(30),
    )?);
    let (runner_shutdown, runner_shutdown_rx) = tokio::sync::watch::channel(false);
    let runner_task = tokio::spawn(orchestrator.clone().run_worker(runner_shutdown_rx));

    // ── Event Bus ───────────────────────────────────────────────────
    // Bridge adapter events into durable projections and the live event bus.
    let event_bus = state.event_bus.clone();
    {
        let event_bus = event_bus.clone();
        let adapter = adapter.clone();
        let approvals = approvals.clone();
        let projection_db = state.db.clone();
        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

            // Subscribe to adapter events
            let adapter_clone = adapter.clone();
            let sub = tokio::spawn(async move {
                if let Err(e) = adapter_clone.subscribe_events(tx).await {
                    tracing::warn!("adapter event subscription ended: {e}");
                }
            });

            // Persist a safe, versioned projection before exposing the event
            // to connected browsers. The database cursor is therefore always
            // available when a client reconnects after seeing an event.
            let mut reconciled_runtime_instance_id = None;
            while let Some(data) = rx.recv().await {
                let runtime_instance_id = match frame_runtime_instance_id(&data) {
                    Ok(runtime_instance_id) => runtime_instance_id,
                    Err(error) => {
                        tracing::warn!("invalid Runtime instance event frame: {error}");
                        continue;
                    }
                };
                if reconciled_runtime_instance_id != Some(runtime_instance_id) {
                    if let Err(error) = approvals
                        .cancel_stale_runtime_requests(runtime_instance_id)
                        .await
                    {
                        tracing::warn!("stale approval reconciliation failed: {error}");
                        continue;
                    }
                    reconciled_runtime_instance_id = Some(runtime_instance_id);
                }
                let runtime_resolution = match runtime_resolved_request(&data) {
                    Ok(runtime_resolution) => runtime_resolution,
                    Err(error) => {
                        tracing::warn!("invalid server-request resolution frame: {error}");
                        continue;
                    }
                };
                if let Some((thread_id, runtime_request_id)) = runtime_resolution {
                    let resolved = match approvals
                        .resolve_runtime_request(
                            runtime_instance_id,
                            &thread_id,
                            &runtime_request_id,
                        )
                        .await
                    {
                        Ok(Some(resolved)) => resolved,
                        Ok(None) => {
                            tracing::warn!(
                                "server-request resolution had no persisted platform approval"
                            );
                            continue;
                        }
                        Err(error) => {
                            tracing::warn!("server-request resolution failed: {error}");
                            continue;
                        }
                    };
                    let public_data = match public_resolved_approval_frame(&data, &resolved) {
                        Ok(frame) => frame,
                        Err(error) => {
                            tracing::warn!("approval resolution projection failed: {error}");
                            continue;
                        }
                    };
                    persist_and_broadcast(&public_data, &projection_db, &event_bus).await;
                    continue;
                }
                let captured = match approvals.capture_event_frame(&data).await {
                    Ok(captured) => captured,
                    Err(error) => {
                        tracing::warn!("app-server request was not persisted: {error}");
                        continue;
                    }
                };
                let public_data = match captured {
                    Some(approval_id) => match public_approval_frame(&data, approval_id) {
                        Ok(frame) => frame,
                        Err(error) => {
                            tracing::warn!("approval projection failed: {error}");
                            continue;
                        }
                    },
                    None => data,
                };
                persist_and_broadcast(&public_data, &projection_db, &event_bus).await;
            }

            let _ = sub.await;
            tracing::info!("event bridge task exiting");
        });
    }

    let mut app = Router::new().nest(
        "/api",
        routes::router(
            adapter,
            providers,
            approvals,
            git,
            orchestrator,
            configuration_secrets,
            profile_binding,
        ),
    );
    if cli.web_dist.is_dir() {
        let index = cli.web_dist.join("index.html");
        app = app.fallback_service(ServeDir::new(&cli.web_dist).fallback(ServeFile::new(index)));
        tracing::info!(web_dist = %cli.web_dist.display(), "serving browser application");
    } else {
        tracing::warn!(web_dist = %cli.web_dist.display(), "browser assets not found; serving API only");
    }
    let app = app.layer(TraceLayer::new_for_http()).with_state(state);

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    tracing::info!("listening on {}", cli.bind);

    let serve_result = axum::serve(listener, app).await;
    let _ = runner_shutdown.send(true);
    let _ = runner_task.await;
    serve_result?;

    Ok(())
}

async fn persist_and_broadcast(
    data: &[u8],
    projection_db: &sqlx::PgPool,
    event_bus: &tokio::sync::broadcast::Sender<open_web_codex_platform_store::LiveEvent>,
) {
    match event_projection::persist_frame(data, projection_db).await {
        Ok(Some(projected)) => {
            let live = open_web_codex_platform_store::LiveEvent {
                organization_id: projected.organization_id,
                payload: projected.payload,
            };
            if event_bus.send(live).is_err() {
                tracing::debug!("event bus: no active receivers, dropping event");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::warn!("event projection failed: {error}"),
    }
}

fn runtime_resolved_request(frame: &[u8]) -> anyhow::Result<Option<(String, serde_json::Value)>> {
    let payload = frame
        .strip_prefix(b"data: ")
        .and_then(|value| value.strip_suffix(b"\n\n"))
        .ok_or_else(|| anyhow::anyhow!("invalid app-server event frame"))?;
    let envelope: serde_json::Value = serde_json::from_slice(payload)?;
    let Some(message) = envelope
        .pointer("/params/message")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };
    if message.get("method").and_then(serde_json::Value::as_str) != Some("serverRequest/resolved") {
        return Ok(None);
    }
    let params = message
        .get("params")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("server-request resolution omitted params"))?;
    let thread_id = params
        .get("threadId")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("server-request resolution omitted threadId"))?
        .to_string();
    let request_id = params
        .get("requestId")
        .filter(|value| value.is_u64() || value.is_i64() || value.is_string())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("server-request resolution omitted a valid requestId"))?;
    Ok(Some((thread_id, request_id)))
}

fn frame_runtime_instance_id(frame: &[u8]) -> anyhow::Result<uuid::Uuid> {
    let payload = frame
        .strip_prefix(b"data: ")
        .and_then(|value| value.strip_suffix(b"\n\n"))
        .ok_or_else(|| anyhow::anyhow!("invalid app-server event frame"))?;
    let envelope: serde_json::Value = serde_json::from_slice(payload)?;
    let value = envelope
        .pointer("/params/runtime_instance_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("event omitted Runtime instance id"))?;
    Ok(uuid::Uuid::parse_str(value)?)
}

fn public_resolved_approval_frame(
    frame: &[u8],
    resolved: &ResolvedApproval,
) -> anyhow::Result<Vec<u8>> {
    let payload = frame
        .strip_prefix(b"data: ")
        .and_then(|value| value.strip_suffix(b"\n\n"))
        .ok_or_else(|| anyhow::anyhow!("invalid app-server event frame"))?;
    let envelope: serde_json::Value = serde_json::from_slice(payload)?;
    let workspace_id = envelope.pointer("/params/workspace_id").cloned();
    let mut public_params = serde_json::Map::from_iter([
        (
            "threadId".to_string(),
            serde_json::Value::String(resolved.thread_id.clone()),
        ),
        (
            "approvalStatus".to_string(),
            serde_json::Value::String(resolved.outcome.as_str().to_string()),
        ),
        (
            "requestId".to_string(),
            serde_json::Value::String(resolved.approval_id.to_string()),
        ),
    ]);
    if let Some(turn_id) = &resolved.turn_id {
        public_params.insert(
            "turnId".to_string(),
            serde_json::Value::String(turn_id.clone()),
        );
    }
    if let Some(item_id) = &resolved.item_id {
        public_params.insert(
            "itemId".to_string(),
            serde_json::Value::String(item_id.clone()),
        );
    }
    let public = serde_json::json!({
        "method": "app-server-event",
        "params": {
            "workspace_id": workspace_id,
            "message": {
                "method": "serverRequest/resolved",
                "params": public_params,
            }
        }
    });
    let mut projected = b"data: ".to_vec();
    serde_json::to_writer(&mut projected, &public)?;
    projected.extend_from_slice(b"\n\n");
    Ok(projected)
}

fn public_approval_frame(frame: &[u8], approval_id: uuid::Uuid) -> anyhow::Result<Vec<u8>> {
    let payload = frame
        .strip_prefix(b"data: ")
        .and_then(|value| value.strip_suffix(b"\n\n"))
        .ok_or_else(|| anyhow::anyhow!("invalid app-server event frame"))?;
    let envelope: serde_json::Value = serde_json::from_slice(payload)?;
    let workspace_id = envelope.pointer("/params/workspace_id").cloned();
    let message = envelope
        .pointer("/params/message")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("approval omitted message"))?;
    let request_method = message
        .get("method")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("approval omitted method"))?;
    let params = message
        .get("params")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("approval omitted params"))?;
    let thread_id = params
        .get("threadId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("approval omitted threadId"))?;
    let mut request_params = serde_json::Map::new();
    for key in ["threadId", "turnId", "itemId", "reason", "startedAtMs"] {
        if let Some(value) = params.get(key) {
            request_params.insert(key.to_string(), value.clone());
        }
    }
    if request_method == "item/commandExecution/requestApproval" {
        if let Some(command) = params.get("command") {
            request_params.insert("command".to_string(), command.clone());
        }
    }
    if request_method == "mcpServer/elicitation/request" {
        for key in ["serverName", "mode", "message", "requestedSchema"] {
            if let Some(value) = params.get(key) {
                request_params.insert(key.to_string(), value.clone());
            }
        }
        if let Some(url) = params
            .get("url")
            .and_then(serde_json::Value::as_str)
            .and_then(routes::configuration::safe_maps_credential_url)
        {
            request_params.insert(
                "url".to_string(),
                serde_json::Value::String(url.to_string()),
            );
        }
    }
    if request_method == "item/tool/requestUserInput" {
        if let Some(questions) = params.get("questions") {
            request_params.insert("questions".to_string(), questions.clone());
        }
        if let Some(timeout) = params.get("autoResolutionMs") {
            request_params.insert("autoResolutionMs".to_string(), timeout.clone());
        }
    }
    let turn_id = request_params.get("turnId").cloned();
    let item_id = request_params.get("itemId").cloned();
    let public = serde_json::json!({
        "method": "app-server-event",
        "params": {
            "workspace_id": workspace_id,
            "message": {
                "method": "platform/approvalRequested",
                "params": {
                    "approvalId": approval_id,
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "itemId": item_id,
                    "requestMethod": request_method,
                    "requestParams": request_params,
                }
            }
        }
    });
    let mut projected = b"data: ".to_vec();
    serde_json::to_writer(&mut projected, &public)?;
    projected.extend_from_slice(b"\n\n");
    Ok(projected)
}

async fn profile_capability_record(
    host: &ProfileHost,
) -> anyhow::Result<routes::RuntimeCapabilityRecord> {
    let snapshot = host.snapshot().await;
    let manifest = host
        .capability_manifest()
        .await
        .ok_or_else(|| anyhow::anyhow!("initialized Profile omitted its Capability Manifest"))?;
    let server_build = snapshot
        .server_build
        .ok_or_else(|| anyhow::anyhow!("initialized Profile omitted its server build"))?;
    let protocol_version = snapshot
        .protocol_version
        .ok_or_else(|| anyhow::anyhow!("initialized Profile omitted its protocol version"))?;
    Ok(routes::RuntimeCapabilityRecord {
        server_build,
        protocol_version,
        manifest: serde_json::to_value(manifest)?,
    })
}

async fn persist_profile_capabilities(
    db: &sqlx::PgPool,
    runtime_key: &str,
    capabilities: &routes::RuntimeCapabilityRecord,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO profile_capabilities \
         (profile_id, server_build, protocol_version, manifest, observed_at) \
         SELECT id, $1, $2, $3, now() FROM profiles WHERE runtime_key = $4 \
         ON CONFLICT (profile_id) DO UPDATE SET server_build = EXCLUDED.server_build, \
         protocol_version = EXCLUDED.protocol_version, manifest = EXCLUDED.manifest, \
         observed_at = now()",
    )
    .bind(&capabilities.server_build)
    .bind(&capabilities.protocol_version)
    .bind(&capabilities.manifest)
    .bind(runtime_key)
    .execute(db)
    .await?;
    Ok(())
}

fn prepare_single_profile_auth_import(
    explicit_source_home: Option<&Path>,
    profile_home: &Path,
) -> anyhow::Result<()> {
    let Some(source_home) = explicit_source_home
        .map(Path::to_path_buf)
        .or_else(default_cli_codex_home)
    else {
        return Ok(());
    };

    import_file_backed_codex_auth_if_missing(&source_home, profile_home)
}

fn default_cli_codex_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".codex"))
}

fn import_file_backed_codex_auth_if_missing(
    source_home: &Path,
    profile_home: &Path,
) -> anyhow::Result<()> {
    let source_auth = source_home.join("auth.json");
    if !source_auth.is_file() {
        tracing::debug!(
            source_home = %source_home.display(),
            "single Profile auth import skipped because source auth.json is absent"
        );
        return Ok(());
    }

    let target_auth = profile_home.join("auth.json");
    if target_auth.exists() {
        tracing::debug!(
            profile_home = %profile_home.display(),
            "single Profile auth import skipped because Profile auth.json already exists"
        );
        return Ok(());
    }

    let source_home = source_home
        .canonicalize()
        .unwrap_or_else(|_| source_home.to_path_buf());
    let profile_home = profile_home
        .canonicalize()
        .unwrap_or_else(|_| profile_home.to_path_buf());
    if source_home == profile_home {
        return Ok(());
    }

    let bytes = fs::read(&source_auth).map_err(|error| {
        anyhow::anyhow!(
            "failed to read single Profile Codex auth from {}: {error}",
            source_auth.display()
        )
    })?;
    if let Some(parent) = target_auth.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            anyhow::anyhow!(
                "failed to prepare Profile auth directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(
        &mut options.open(&target_auth).map_err(|error| {
            anyhow::anyhow!(
                "failed to create Profile auth file {}: {error}",
                target_auth.display()
            )
        })?,
        &bytes,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "failed to write Profile auth file {}: {error}",
            target_auth.display()
        )
    })?;

    tracing::info!(
        "imported file-backed Codex auth into the single Profile home for transitional login sharing"
    );
    Ok(())
}

async fn ensure_transitional_profile_binding(
    db: &sqlx::PgPool,
    runtime_key: &str,
    name: &str,
) -> anyhow::Result<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM profiles WHERE runtime_key = $1)")
            .bind(runtime_key)
            .fetch_one(db)
            .await?;
    if exists {
        return Ok(());
    }

    let owners = sqlx::query(
        "SELECT m.organization_id, m.user_id FROM memberships m \
         WHERE m.role = 'owner' ORDER BY m.created_at, m.id LIMIT 2",
    )
    .fetch_all(db)
    .await?;
    match owners.as_slice() {
        [] => Ok(()),
        [owner] => {
            sqlx::query(
                "INSERT INTO profiles (organization_id, owner_user_id, runtime_key, name) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT (runtime_key) DO NOTHING",
            )
            .bind(owner.get::<uuid::Uuid, _>("organization_id"))
            .bind(owner.get::<uuid::Uuid, _>("user_id"))
            .bind(runtime_key)
            .bind(name)
            .execute(db)
            .await?;
            Ok(())
        }
        _ => anyhow::bail!(
            "Profile '{runtime_key}' has no ownership binding and multiple owner memberships exist"
        ),
    }
}

async fn ensure_local_owner(db: &sqlx::PgPool) -> anyhow::Result<()> {
    let user_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)")
        .fetch_one(db)
        .await?;
    if user_exists {
        return Ok(());
    }

    let password: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|error| anyhow::anyhow!("local owner password task failed: {error}"))?
        .map_err(|error| anyhow::anyhow!("local owner password hashing failed: {error}"))?;

    let mut transaction = db.begin().await?;
    sqlx::query("LOCK TABLE users IN EXCLUSIVE MODE")
        .execute(&mut *transaction)
        .await?;
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *transaction)
        .await?;
    if user_count > 0 {
        transaction.commit().await?;
        return Ok(());
    }

    let user_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (name, username, email, password_hash, role) \
         VALUES ('Local User', 'local', 'local@localhost.invalid', $1, 'owner') \
         RETURNING id",
    )
    .bind(password_hash)
    .fetch_one(&mut *transaction)
    .await?;
    let organization_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (name, slug) VALUES ('Local Workspace', 'local-workspace') \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id",
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    tracing::info!("created the implicit local owner for authentication-free startup");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        import_file_backed_codex_auth_if_missing, public_approval_frame,
        public_resolved_approval_frame, runtime_resolved_request,
    };
    use crate::routes::configuration::safe_maps_credential_url;
    use open_web_codex_approval_service::ResolvedApproval;
    use serde_json::Value;
    use std::fs;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[test]
    fn single_profile_auth_import_copies_missing_file_backed_login() {
        let source = TempDir::new().expect("source home");
        let profile = TempDir::new().expect("profile home");
        let auth = r#"{"auth_mode":"chatgpt","tokens":{"id_token":"id","access_token":"access","refresh_token":"refresh","account_id":"acct"}}"#;
        fs::write(source.path().join("auth.json"), auth).expect("write source auth");

        import_file_backed_codex_auth_if_missing(source.path(), profile.path())
            .expect("import auth");

        assert_eq!(
            fs::read_to_string(profile.path().join("auth.json")).expect("read profile auth"),
            auth
        );
    }

    #[test]
    fn single_profile_auth_import_preserves_existing_profile_auth() {
        let source = TempDir::new().expect("source home");
        let profile = TempDir::new().expect("profile home");
        fs::write(
            source.path().join("auth.json"),
            r#"{"auth_mode":"chatgpt"}"#,
        )
        .expect("write source auth");
        fs::write(
            profile.path().join("auth.json"),
            r#"{"auth_mode":"apikey"}"#,
        )
        .expect("write profile auth");

        import_file_backed_codex_auth_if_missing(source.path(), profile.path())
            .expect("import auth");

        assert_eq!(
            fs::read_to_string(profile.path().join("auth.json")).expect("read profile auth"),
            r#"{"auth_mode":"apikey"}"#
        );
    }

    #[test]
    fn public_approval_events_omit_runtime_request_ids_and_server_paths() {
        let mut raw = br#"data: {"method":"app-server-event","params":{"workspace_id":"workspace-1","message":{"id":77,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","itemId":"item-1","cwd":"/private/server/path","command":"git status"}}}}"#.to_vec();
        raw.extend_from_slice(b"\n\n");
        let approval_id = Uuid::now_v7();
        let projected = public_approval_frame(&raw, approval_id).expect("approval projection");
        let text = String::from_utf8(projected.clone()).unwrap();
        assert!(!text.contains("/private/server/path"));
        assert!(!text.contains("\"id\":77"));

        let payload = projected
            .strip_prefix(b"data: ")
            .and_then(|value| value.strip_suffix(b"\n\n"))
            .unwrap();
        let value: Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(
            value.pointer("/params/message/method").unwrap(),
            "platform/approvalRequested"
        );
        assert_eq!(
            value.pointer("/params/message/params/approvalId").unwrap(),
            &Value::String(approval_id.to_string())
        );
        assert_eq!(
            value
                .pointer("/params/message/params/requestMethod")
                .unwrap(),
            "item/commandExecution/requestApproval"
        );
        assert_eq!(
            value
                .pointer("/params/message/params/requestParams/command")
                .unwrap(),
            "git status"
        );
    }

    #[test]
    fn public_mcp_elicitation_events_are_safe_approval_projections() {
        let mut raw = br#"data: {"method":"app-server-event","params":{"workspace_id":"workspace-1","message":{"id":88,"method":"mcpServer/elicitation/request","params":{"threadId":"thread-1","turnId":"turn-1","serverName":"map_utils","mode":"url","_meta":{"secret":"do-not-project","path":"/private/server/path"},"message":"A maps provider and API key are required. Configure Mapbox or Google in this app; the selected provider will be saved globally and reused.","url":"http://127.0.0.1:43123/one-time-token","elicitationId":"runtime-secret-id"}}}}"#.to_vec();
        raw.extend_from_slice(b"\n\n");
        let approval_id = Uuid::now_v7();
        let projected = public_approval_frame(&raw, approval_id).expect("approval projection");
        let text = String::from_utf8(projected.clone()).unwrap();
        assert!(!text.contains("do-not-project"));
        assert!(!text.contains("/private/server/path"));
        assert!(!text.contains("\"id\":88"));

        let payload = projected
            .strip_prefix(b"data: ")
            .and_then(|value| value.strip_suffix(b"\n\n"))
            .unwrap();
        let value: Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(
            value
                .pointer("/params/message/params/requestMethod")
                .unwrap(),
            "mcpServer/elicitation/request"
        );
        assert_eq!(
            value
                .pointer("/params/message/params/requestParams/serverName")
                .unwrap(),
            "map_utils"
        );
        assert_eq!(
            value
                .pointer("/params/message/params/requestParams/message")
                .unwrap(),
            "A maps provider and API key are required. Configure Mapbox or Google in this app; the selected provider will be saved globally and reused."
        );
        assert_eq!(
            value
                .pointer("/params/message/params/requestParams/url")
                .unwrap(),
            "http://127.0.0.1:43123/one-time-token"
        );
        assert!(!text.contains("runtime-secret-id"));
    }

    #[test]
    fn public_mcp_elicitation_rejects_non_loopback_configuration_urls() {
        assert_eq!(
            safe_maps_credential_url("http://127.0.0.1:43123/one-time-token"),
            Some("http://127.0.0.1:43123/one-time-token")
        );
        assert_eq!(
            safe_maps_credential_url("https://example.com/steal-token"),
            None
        );
        assert_eq!(
            safe_maps_credential_url("http://localhost:43123/one-time-token"),
            None
        );
        assert_eq!(safe_maps_credential_url("http://127.0.0.1/"), None);
    }

    #[test]
    fn public_resolution_uses_platform_identity_and_omits_runtime_request_id() {
        let mut raw = br#"data: {"method":"app-server-event","params":{"workspace_id":"workspace-1","message":{"method":"serverRequest/resolved","params":{"threadId":"runtime-thread","requestId":"runtime-secret-77"}}}}"#.to_vec();
        raw.extend_from_slice(b"\n\n");
        assert_eq!(
            runtime_resolved_request(&raw).unwrap(),
            Some((
                "runtime-thread".to_string(),
                Value::String("runtime-secret-77".to_string())
            ))
        );

        let approval_id = Uuid::now_v7();
        let projected = public_resolved_approval_frame(
            &raw,
            &ResolvedApproval {
                approval_id,
                thread_id: "platform-thread".to_string(),
                turn_id: Some("platform-turn".to_string()),
                item_id: Some("platform-item".to_string()),
                outcome: open_web_codex_approval_service::ApprovalOutcome::Accepted,
            },
        )
        .expect("approval resolution projection");
        let text = String::from_utf8(projected.clone()).unwrap();
        assert!(!text.contains("runtime-secret-77"));
        assert!(!text.contains("runtime-thread"));

        let payload = projected
            .strip_prefix(b"data: ")
            .and_then(|value| value.strip_suffix(b"\n\n"))
            .unwrap();
        let value: Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(
            value.pointer("/params/message/method").unwrap(),
            "serverRequest/resolved"
        );
        assert_eq!(
            value.pointer("/params/message/params/requestId").unwrap(),
            &Value::String(approval_id.to_string())
        );
        assert_eq!(
            value.pointer("/params/message/params/threadId").unwrap(),
            "platform-thread"
        );
        assert_eq!(
            value.pointer("/params/message/params/turnId").unwrap(),
            "platform-turn"
        );
        assert_eq!(
            value.pointer("/params/message/params/itemId").unwrap(),
            "platform-item"
        );
        assert_eq!(
            value
                .pointer("/params/message/params/approvalStatus")
                .unwrap(),
            "accepted"
        );
    }
}
