use super::*;
use color_eyre::eyre::WrapErr;
use pretty_assertions::assert_eq;
use std::path::Path;

#[test]
fn app_scoped_key_path_quotes_dotted_app_ids() {
    assert_eq!(
        app_scoped_key_path("plugin.linear", "enabled"),
        "apps.\"plugin.linear\".enabled"
    );
}

#[test]
fn trusted_project_edit_targets_project_trust_level() {
    assert_eq!(
        trusted_project_edit(Path::new("/workspace/team.project")),
        ConfigEdit {
            key_path: "projects.\"/workspace/team.project\".trust_level".to_string(),
            value: serde_json::json!("trusted"),
            merge_strategy: MergeStrategy::Replace,
        }
    );
}

#[test]
fn format_config_error_preserves_server_validation_message() {
    let err = Err::<(), _>(color_eyre::eyre::eyre!(
        "config/batchWrite failed: Invalid configuration: features.fast_mode=true violates \
         managed requirements; allowed set [fast_mode=false]"
    ))
    .wrap_err("config/batchWrite failed in TUI")
    .unwrap_err();

    assert_eq!(
        format_config_error(&err),
        "config/batchWrite failed in TUI: config/batchWrite failed: Invalid configuration: \
         features.fast_mode=true violates managed requirements; allowed set [fast_mode=false]"
    );
}

#[test]
fn build_model_selection_edit_writes_only_model_without_clear_values() {
    assert_eq!(
        build_model_selection_edit("roma-model"),
        ConfigEdit {
            key_path: "model".to_string(),
            value: serde_json::json!("roma-model"),
            merge_strategy: MergeStrategy::Replace,
        }
    );
}

#[test]
fn build_model_provider_edit_omits_null_fields() {
    let provider = ModelProviderInfo {
        name: "Roma".to_string(),
        base_url: Some("http://sim.isc.huawei.com:8080/v1".to_string()),
        env_key: Some("DEEPSEEK_API_KEY".to_string()),
        wire_api: codex_model_provider_info::WireApi::Chat,
        models: Vec::new(),
        ..Default::default()
    };

    let edit = build_model_provider_edit("roma", &provider).expect("provider edit");

    assert_eq!(edit.key_path, "model_providers.\"roma\"");
    assert_eq!(edit.merge_strategy, MergeStrategy::Replace);
    assert_eq!(
        edit.value,
        serde_json::json!({
            "name": "Roma",
            "base_url": "http://sim.isc.huawei.com:8080/v1",
            "env_key": "DEEPSEEK_API_KEY",
            "wire_api": "chat",
            "requires_openai_auth": false,
            "supports_websockets": false,
            "supports_web_search": false,
            "supports_image_generation": false,
        })
    );
}

#[test]
fn build_model_provider_models_edit_writes_provider_models() {
    let models = vec![codex_model_provider_info::ProviderModelInfo {
        model_id: "deepseek-chat".to_string(),
        model_name: Some("DeepSeek Chat".to_string()),
        max_token_len: Some(64_000),
        max_output_tokens: Some(8_000),
        show_in_picker: true,
        context_window: Some(128_000),
        ..Default::default()
    }];

    assert_eq!(
        build_model_provider_models_edit("deepseek", &models),
        ConfigEdit {
            key_path: "model_providers.\"deepseek\".models".to_string(),
            value: serde_json::json!([
                {
                    "model_id": "deepseek-chat",
                    "model_name": "DeepSeek Chat",
                    "max_token_len": 64000,
                    "max_output_tokens": 8000,
                    "show_in_picker": true,
                    "context_window": 128000,
                    "supported_reasoning_levels": [],
                }
            ]),
            merge_strategy: MergeStrategy::Replace,
        }
    );
}
