use open_web_codex_platform_store::migrate;
use open_web_codex_secret_store::{PostgresSecretStore, SecretCipher, SecretValue};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn blank_migrations_are_idempotent_and_provider_secrets_are_encrypted() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect disposable PostgreSQL database");

    migrate::run(&pool).await.expect("first migration run");
    migrate::run(&pool).await.expect("second migration run");

    let organization_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let profile_id = Uuid::now_v7();
    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, 'Test', $2)")
        .bind(organization_id)
        .bind(format!("test-{organization_id}"))
        .execute(&pool)
        .await
        .expect("insert organization");
    sqlx::query(
        "INSERT INTO users (id, username, name, email, password_hash, role) \
         VALUES ($1, $2, 'Test', $3, 'not-a-real-password-hash', 'owner')",
    )
    .bind(user_id)
    .bind(format!("test-{user_id}"))
    .bind(format!("{user_id}@example.invalid"))
    .execute(&pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO profiles (id, organization_id, owner_user_id, runtime_key, name) \
         VALUES ($1, $2, $3, $4, 'Test Profile')",
    )
    .bind(profile_id)
    .bind(organization_id)
    .bind(user_id)
    .bind(format!("test-{profile_id}"))
    .execute(&pool)
    .await
    .expect("insert Profile");

    let store = PostgresSecretStore::new(
        pool.clone(),
        SecretCipher::generate("test-v1").expect("generate cipher"),
    );
    let plaintext = "provider-secret-that-must-not-be-stored";
    let secret = SecretValue::new(plaintext).expect("Secret value");
    let environment_key = store
        .put_provider_key(organization_id, profile_id, "provider-a", &secret)
        .await
        .expect("store Provider Secret");

    let row = sqlx::query(
        "SELECT nonce, ciphertext FROM profile_secrets \
         WHERE profile_id = $1 AND provider_id = 'provider-a'",
    )
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .expect("read encrypted row");
    let nonce: Vec<u8> = row.get("nonce");
    let ciphertext: Vec<u8> = row.get("ciphertext");
    assert_eq!(nonce.len(), 12);
    assert!(!ciphertext
        .windows(plaintext.len())
        .any(|window| window == plaintext.as_bytes()));

    let restored = store
        .get_provider_key(organization_id, profile_id, "provider-a")
        .await
        .expect("decrypt Provider Secret")
        .expect("stored Provider Secret");
    assert_eq!(restored.expose(), plaintext);
    assert_eq!(
        store
            .list_provider_environment(organization_id, profile_id)
            .await
            .expect("list Profile environment")[0]
            .environment_key,
        environment_key
    );
}
