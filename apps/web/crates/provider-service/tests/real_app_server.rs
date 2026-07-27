use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use open_web_codex_platform_contracts::{
    ProviderCredentialInput, UpdateProviderModelRequest, UpsertProviderRequest,
};
use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig};
use open_web_codex_profile_registry::ProfileRegistry;
use open_web_codex_provider_service::secured::{
    AuthorizedProviderOperations, ProviderActor, SecuredProviderService,
};
use open_web_codex_provider_service::ProviderService;
use open_web_codex_secret_store::{PostgresSecretStore, SecretCipher};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

fn temporary_profile_home() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "open-web-codex-real-provider-service-{}-{timestamp}",
        std::process::id()
    ))
}

async fn spawn_host(codex_bin: &Path, home: &Path, workspace: &Path) -> ProfileHost {
    ProfileHost::spawn(
        ProfileHostConfig::new("provider-smoke-profile", home, workspace).with_codex_bin(codex_bin),
    )
    .await
    .expect("spawn native Profile Host")
}

async fn start_mock_models_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock model server");
    let address = listener.local_addr().expect("mock server address");
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = vec![0_u8; 32 * 1024];
                let count = stream.read(&mut request).await.unwrap_or_default();
                let request_line = String::from_utf8_lossy(&request[..count])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let model_id = if request_line.contains("/provider-two/") {
                    "provider-two-model"
                } else {
                    "provider-one-model"
                };
                let body = format!(
                    r#"{{"data":[{{"id":"{model_id}","object":"model","owned_by":"smoke"}}]}}"#
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{address}"), task)
}

fn upsert_request(base_url: String, api_key: &str) -> UpsertProviderRequest {
    UpsertProviderRequest {
        name: "Provider smoke".to_string(),
        base_url,
        wire_api: "chat".to_string(),
        credentials: ProviderCredentialInput::Direct {
            api_key: api_key.to_string(),
        },
        select: true,
    }
}

/// Exercises Provider CRUD, provider-scoped model refresh and cache isolation
/// against the checked-out Codex app-server binary. Run with:
///
/// `CODEX_BIN=/absolute/path/to/codex cargo test -p open-web-codex-provider-service --test real_app_server -- --ignored`
#[tokio::test]
#[ignore = "requires a real Codex CLI binary"]
async fn switches_and_refreshes_isolated_provider_catalogs() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/web workspace root")
        .canonicalize()
        .expect("canonical workspace");
    let home = temporary_profile_home();
    std::fs::create_dir_all(&home).expect("create Profile home");
    std::fs::write(home.join("config.toml"), "").expect("write empty Profile config");
    let (models_uri, models_server) = start_mock_models_server().await;
    let host = spawn_host(&codex_bin, &home, &workspace).await;
    let service = ProviderService::for_profile_host(host.clone());

    let first_secret = "provider-one-secret-value";
    let first = service
        .upsert(
            "provider-one",
            upsert_request(format!("{models_uri}/provider-one/v1"), first_secret),
        )
        .await
        .expect("create first Provider");
    assert_eq!(first.current_provider_id, "provider-one");
    assert!(!serde_json::to_string(&first)
        .expect("serialize first catalog")
        .contains(first_secret));
    let first = service
        .refresh_models("provider-one")
        .await
        .expect("refresh first Provider");
    let first_provider = first
        .data
        .iter()
        .find(|provider| provider.id == "provider-one")
        .expect("first Provider in catalog");
    assert_eq!(first_provider.models[0].model_id, "provider-one-model");

    let second_secret = "provider-two-secret-value";
    service
        .upsert(
            "provider-two",
            upsert_request(format!("{models_uri}/provider-two/v1"), second_secret),
        )
        .await
        .expect("create second Provider");
    let second = service
        .refresh_models("provider-two")
        .await
        .expect("refresh second Provider");
    assert_eq!(second.current_provider_id, "provider-two");
    assert!(!serde_json::to_string(&second)
        .expect("serialize second catalog")
        .contains(second_secret));
    let first_provider = second
        .data
        .iter()
        .find(|provider| provider.id == "provider-one")
        .expect("first Provider remains in catalog");
    let second_provider = second
        .data
        .iter()
        .find(|provider| provider.id == "provider-two")
        .expect("second Provider in catalog");
    assert_eq!(first_provider.models[0].model_id, "provider-one-model");
    assert_eq!(second_provider.models[0].model_id, "provider-two-model");

    let selected = service
        .select("provider-one")
        .await
        .expect("select first Provider again");
    assert_eq!(selected.current_provider_id, "provider-one");
    assert_eq!(
        selected
            .data
            .iter()
            .find(|provider| provider.id == "provider-two")
            .expect("second Provider remains after switch")
            .models[0]
            .model_id,
        "provider-two-model"
    );

    host.shutdown().await.expect("shutdown Profile Host");
    drop(host);
    models_server.abort();
    std::fs::remove_dir_all(home).expect("remove smoke Profile home");
}

/// Proves that the production Provider path persists ciphertext, writes only a
/// generated environment key to Codex config, and restarts the Profile with
/// the decrypted value in its private child environment.
#[tokio::test]
#[ignore = "requires CODEX_BIN and a disposable TEST_DATABASE_URL"]
async fn secured_provider_credentials_never_enter_codex_config() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is set");
    let db = PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await
        .expect("connect disposable PostgreSQL database");
    open_web_codex_platform_store::migrate::run(&db)
        .await
        .expect("migrate database");

    let organization_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let profile_id = Uuid::now_v7();
    let runtime_key = format!("secured-provider-{profile_id}");
    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, 'Test', $2)")
        .bind(organization_id)
        .bind(format!("test-{organization_id}"))
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, name, email, password_hash, role) \
         VALUES ($1, $2, 'Test', $3, 'test-only', 'owner')",
    )
    .bind(user_id)
    .bind(format!("test-{user_id}"))
    .bind(format!("{user_id}@example.invalid"))
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO profiles (id, organization_id, owner_user_id, runtime_key, name) \
         VALUES ($1, $2, $3, $4, 'Test Profile')",
    )
    .bind(profile_id)
    .bind(organization_id)
    .bind(user_id)
    .bind(&runtime_key)
    .execute(&db)
    .await
    .unwrap();

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/web workspace root")
        .canonicalize()
        .expect("canonical workspace");
    let home = temporary_profile_home();
    std::fs::create_dir_all(&home).expect("create Profile home");
    std::fs::write(home.join("config.toml"), "").expect("write empty Profile config");
    let (models_uri, models_server) = start_mock_models_server().await;

    let registry = ProfileRegistry::new();
    let service = SecuredProviderService::new(
        db.clone(),
        runtime_key.clone(),
        registry.clone(),
        PostgresSecretStore::new(
            db.clone(),
            SecretCipher::generate("test-v1").expect("generate cipher"),
        ),
    );
    let initial_environment = service.startup_secret_environment().await.unwrap();
    assert!(initial_environment.is_empty());
    let host = registry
        .register_with_secret_environment(
            ProfileHostConfig::new(&runtime_key, &home, &workspace).with_codex_bin(codex_bin),
            initial_environment,
        )
        .await
        .expect("register Profile");
    let initial_process_id = host.snapshot().await.process_id;
    let actor = ProviderActor {
        user_id,
        organization_id,
    };
    let secret = "secured-provider-secret-value";
    let catalog = service
        .upsert(
            actor,
            "secured-provider",
            upsert_request(format!("{models_uri}/provider-one/v1"), secret),
        )
        .await
        .expect("create secured Provider");
    let provider = catalog
        .data
        .iter()
        .find(|provider| provider.id == "secured-provider")
        .expect("secured Provider catalog entry");
    let environment_key = provider
        .env_key
        .as_deref()
        .expect("generated environment key");
    assert!(environment_key.starts_with("OPEN_WEB_CODEX_PROVIDER_"));
    assert!(!serde_json::to_string(&catalog).unwrap().contains(secret));
    let config = std::fs::read_to_string(home.join("config.toml")).expect("read Codex config");
    assert!(config.contains(environment_key));
    assert!(!config.contains(secret));
    assert_ne!(
        registry
            .host(&runtime_key)
            .await
            .unwrap()
            .snapshot()
            .await
            .process_id,
        initial_process_id
    );
    assert_eq!(
        registry
            .secret_environment_keys(&runtime_key)
            .await
            .unwrap(),
        vec![environment_key.to_string()]
    );

    service
        .refresh_models(actor, "secured-provider")
        .await
        .expect("refresh secured Provider models");
    assert!(host
        .apply_scheduled_restart()
        .await
        .expect("apply refreshed Provider catalog"));
    service
        .update_model(
            actor,
            "secured-provider",
            "provider-one-model",
            UpdateProviderModelRequest {
                context_window: 256_000,
            },
        )
        .await
        .expect("update secured Provider model context");
    assert!(host
        .apply_scheduled_restart()
        .await
        .expect("apply updated Provider model context"));

    let row = sqlx::query(
        "SELECT ciphertext FROM profile_secrets WHERE profile_id = $1 AND provider_id = $2",
    )
    .bind(profile_id)
    .bind("secured-provider")
    .fetch_one(&db)
    .await
    .expect("encrypted Secret row");
    let ciphertext: Vec<u8> = row.get("ciphertext");
    assert!(!ciphertext
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));

    service
        .select(actor, "openai")
        .await
        .expect("select built-in Provider");
    service
        .delete(actor, "secured-provider")
        .await
        .expect("delete secured Provider");
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM profile_secrets WHERE profile_id = $1")
            .bind(profile_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    assert!(registry
        .secret_environment_keys(&runtime_key)
        .await
        .unwrap()
        .is_empty());

    registry
        .shutdown(&runtime_key)
        .await
        .expect("shutdown Profile");
    models_server.abort();
    std::fs::remove_dir_all(home).expect("remove smoke Profile home");
}
