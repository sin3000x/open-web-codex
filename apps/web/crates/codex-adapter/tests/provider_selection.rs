//! Exercise the actual adapter/JSONL boundary, not a permissive FakeCodexAdapter.
use open_web_codex_adapter::real::RealCodexAdapter;
use open_web_codex_adapter::{
    require_thread_provider, AdapterError, AuthorizedWorkspace, CodexAdapter, TurnOptions,
};
use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig};
use serde_json::{json, Value};

#[test]
fn provider_validation_requires_authoritative_identity() {
    for response in [
        json!({}),
        json!({"thread":{"id":"t"}}),
        json!({"thread":{"id":"other","modelProvider":"internal"}}),
    ] {
        assert!(matches!(
            require_thread_provider(&response, "t", "internal"),
            Err(AdapterError::Rpc(_))
        ));
    }
    let thread = json!({"thread":{"id":"t","modelProvider":"internal"}});
    assert!(require_thread_provider(&thread, "t", "internal").is_ok());
    assert!(matches!(
        require_thread_provider(&thread, "t", "official"),
        Err(AdapterError::ThreadProviderMismatch)
    ));
}

#[tokio::test]
async fn rejects_cross_provider_before_turn_start_in_loaded_and_cold_threads() {
    let temp = tempfile::tempdir().unwrap();
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let capture = temp.path().join("requests.jsonl");
    let script = temp.path().join("app_server.py");
    std::fs::write(&script, r#"
import json, os, sys
thread = {"id": "thread-1", "modelProvider": "internal", "historyMode": "paginated"}
for line in sys.stdin:
    req = json.loads(line)
    if "id" not in req:
        continue
    with open(os.environ["TEST_CAPTURE"], "a") as f:
        f.write(json.dumps(req) + "\n")
    method = req["method"]
    if method == "initialize":
        result = {"userAgent": "test", "platformFamily": "unix", "platformOs": "linux", "codexHome": os.environ["CODEX_HOME"]}
    elif method in ("thread/start", "thread/resume", "thread/read"):
        result = {"thread": thread}
    elif method == "turn/start":
        result = {"turn": {"id": "turn-1"}}
    else:
        raise RuntimeError("unexpected method: " + method)
    print(json.dumps({"id": req["id"], "result": result}), flush=True)
"#).unwrap();
    let mut config = ProfileHostConfig::new(
        "provider-test",
        temp.path().join("profile"),
        &workspace_root,
    )
    .with_codex_bin("python3")
    .with_environment("TEST_CAPTURE", &capture);
    config.codex_args = vec!["-u".into(), script.into_os_string()];
    let host = ProfileHost::spawn(config).await.unwrap();
    let workspace = AuthorizedWorkspace {
        id: "workspace".into(),
        root: workspace_root.clone(),
    };
    let adapter =
        RealCodexAdapter::from_host(host.clone(), "workspace", workspace_root.clone()).unwrap();
    adapter.start_thread(&workspace, None).await.unwrap();
    let incompatible = TurnOptions {
        model: Some("official-model".into()),
        model_provider: Some("official".into()),
        ..Default::default()
    };
    assert!(matches!(
        adapter
            .send_user_message(&workspace, "thread-1", "hi", &incompatible)
            .await,
        Err(AdapterError::ThreadProviderMismatch)
    ));
    // A fresh adapter must resume the stored Thread, then enforce the same guard.
    let cold = RealCodexAdapter::from_host(host, "workspace", workspace_root).unwrap();
    assert!(matches!(
        cold.send_user_message(&workspace, "thread-1", "hi", &incompatible)
            .await,
        Err(AdapterError::ThreadProviderMismatch)
    ));
    let before = std::fs::read_to_string(&capture).unwrap();
    assert!(!before.contains("turn/start"));
    assert!(before.contains("thread/resume"));
    let compatible = TurnOptions {
        model: Some("internal-model-2".into()),
        model_provider: Some("internal".into()),
        access_mode: Some("read-only".into()),
        ..Default::default()
    };
    cold.send_user_message(&workspace, "thread-1", "hi", &compatible)
        .await
        .unwrap();
    let requests: Vec<Value> = std::fs::read_to_string(capture)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let turns: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "turn/start")
        .collect();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["params"]["model"], "internal-model-2");
    assert!(turns[0]["params"].get("modelProvider").is_none());
}
