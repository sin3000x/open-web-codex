pub mod approvals;
pub mod artifacts;
pub mod bootstrap;
pub mod browser_workspaces;
pub mod configuration;
pub mod copilots;
pub mod events;
pub mod generation;
pub mod github;
pub mod health;
pub mod me;
pub mod organizations;
pub mod profile;
pub mod profile_content;
pub mod projects;
pub mod provider_metrics;
pub mod providers;
pub mod runs;
pub mod runtime_agents;
pub mod sessions;
pub mod tasks;
pub mod terminals;
pub mod threads;
pub mod workspaces;

use std::sync::Arc;

use crate::copilot_installation::CopilotInstallationService;
use crate::delivery_contracts::DeliveryRegistry;
use axum::{Extension, Router};
use open_web_codex_adapter::CodexAdapter;
use open_web_codex_approval_service::ApprovalService;
use open_web_codex_git_runtime::GitRuntime;
use open_web_codex_platform_store::AppState;
use open_web_codex_provider_service::secured::AuthorizedProviderOperations;
use open_web_codex_run_orchestrator::RunOrchestrator;
use open_web_codex_secret_store::PostgresSecretStore;

#[derive(Clone)]
pub struct RuntimeProfileBinding {
    pub runtime_key: String,
    pub name: String,
    pub codex_home: Option<Arc<std::path::PathBuf>>,
}

/// Assemble all platform API routes.
pub fn router(
    adapter: Arc<dyn CodexAdapter>,
    providers: Arc<dyn AuthorizedProviderOperations>,
    approvals: Arc<ApprovalService>,
    git: Arc<GitRuntime>,
    orchestrator: Arc<RunOrchestrator>,
    configuration_secrets: Arc<PostgresSecretStore>,
    profile: RuntimeProfileBinding,
    deliveries: Arc<DeliveryRegistry>,
    copilots: Arc<CopilotInstallationService>,
) -> Router<AppState> {
    Router::new()
        .route("/bootstrap", axum::routing::post(bootstrap::bootstrap))
        .route("/sessions", axum::routing::post(sessions::create_session))
        .route(
            "/sessions/local",
            axum::routing::post(sessions::create_local_session),
        )
        .route(
            "/sessions/current",
            axum::routing::delete(sessions::delete_session),
        )
        .route(
            "/sessions/organization",
            axum::routing::put(sessions::select_organization),
        )
        .route("/me", axum::routing::get(me::me))
        .route(
            "/browser-workspace-preferences",
            axum::routing::get(browser_workspaces::list),
        )
        .route(
            "/browser-workspace-preferences/{id}",
            axum::routing::put(browser_workspaces::update_settings),
        )
        .route(
            "/browser-workspace-preferences/{id}/runtime-codex-args",
            axum::routing::put(browser_workspaces::set_runtime_codex_args),
        )
        .route(
            "/browser-workspace-preferences/{id}/worktree-setup",
            axum::routing::get(browser_workspaces::worktree_setup_status)
                .post(browser_workspaces::mark_worktree_setup_ran),
        )
        .route(
            "/configuration/maps",
            axum::routing::get(configuration::get_maps).put(configuration::update_maps),
        )
        .route(
            "/configuration/maps/use",
            axum::routing::post(configuration::use_maps),
        )
        .route("/profile/account", axum::routing::get(profile::account))
        .route(
            "/profile/login",
            axum::routing::post(profile::start_login).delete(profile::cancel_login),
        )
        .route(
            "/profile/login/{login_id}",
            axum::routing::get(profile::login_status),
        )
        .route(
            "/profile/rate-limits",
            axum::routing::get(profile::rate_limits),
        )
        .route("/profile/usage", axum::routing::get(profile::usage))
        .route(
            "/profile/collaboration-modes",
            axum::routing::get(profile::collaboration_modes),
        )
        .route("/profile/apps", axum::routing::get(profile::apps))
        .route(
            "/profile/runtime-status",
            axum::routing::get(profile::runtime_status),
        )
        .route(
            "/profile/mcp-servers",
            axum::routing::get(profile::mcp_servers),
        )
        .route(
            "/profile/experimental-features",
            axum::routing::get(profile::experimental_features),
        )
        .route("/profile/skills", axum::routing::get(profile::skills))
        .route("/profile/copilots", axum::routing::get(copilots::status))
        .route(
            "/profile/copilots/activate",
            axum::routing::post(copilots::activate),
        )
        .route(
            "/profile/copilots/deactivate",
            axum::routing::post(copilots::deactivate),
        )
        .route(
            "/profile/features/{name}",
            axum::routing::put(profile_content::set_experimental_feature),
        )
        .route(
            "/profile/files/{kind}",
            axum::routing::get(profile_content::read_profile_file)
                .put(profile_content::write_profile_file),
        )
        .route(
            "/profile/agents",
            axum::routing::get(profile_content::get_agents).post(profile_content::create_agent),
        )
        .route(
            "/profile/config/model",
            axum::routing::get(profile_content::get_config_model),
        )
        .route(
            "/profile/agents/settings",
            axum::routing::put(profile_content::set_agents_core),
        )
        .route(
            "/profile/agents/{name}",
            axum::routing::patch(profile_content::update_agent)
                .delete(profile_content::delete_agent),
        )
        .route(
            "/profile/agents/{name}/config",
            axum::routing::get(profile_content::read_agent_config)
                .put(profile_content::write_agent_config),
        )
        .route(
            "/profile/prompts",
            axum::routing::get(profile_content::list_prompts)
                .post(profile_content::create_prompt)
                .put(profile_content::update_prompt)
                .delete(profile_content::delete_prompt),
        )
        .route(
            "/profile/prompts/move",
            axum::routing::post(profile_content::move_prompt),
        )
        .route(
            "/profile/approval-rules",
            axum::routing::post(profile_content::remember_approval_rule),
        )
        .route("/approvals", axum::routing::get(approvals::list_pending))
        .route(
            "/runs/{id}/approval-requests",
            axum::routing::get(approvals::list_run_approval_requests),
        )
        .route(
            "/runs/{id}/user-input-requests",
            axum::routing::get(approvals::list_run_user_inputs),
        )
        .route(
            "/runs/{id}/mcp-form-requests",
            axum::routing::get(approvals::list_run_mcp_forms),
        )
        .route(
            "/approvals/{id}/decision",
            axum::routing::post(approvals::decide),
        )
        .route(
            "/approvals/{id}/user-input",
            axum::routing::post(approvals::respond_user_input),
        )
        .route(
            "/approvals/{id}/mcp-form",
            axum::routing::post(approvals::respond_mcp_form),
        )
        .route("/tasks/{id}/runs", axum::routing::post(runs::start_run))
        .route("/runs", axum::routing::get(runs::list_runs))
        .route("/runs/{id}", axum::routing::get(runs::get_run))
        .route(
            "/runs/{id}/agents",
            axum::routing::get(runtime_agents::list_for_run),
        )
        .route(
            "/runs/{id}/agent-activities",
            axum::routing::get(runtime_agents::list_activities_for_run),
        )
        .route(
            "/runs/{id}/agent-executions",
            axum::routing::get(runtime_agents::list_executions_for_run),
        )
        .route(
            "/runs/{id}/provider-metrics",
            axum::routing::get(provider_metrics::list_for_run),
        )
        .route("/artifacts/{id}", axum::routing::get(artifacts::get))
        .route(
            "/artifacts/{id}/content",
            axum::routing::get(artifacts::read_content),
        )
        .route(
            "/artifacts/{id}/download",
            axum::routing::get(artifacts::download),
        )
        .route("/runs/{id}/thread", axum::routing::get(threads::read))
        .route(
            "/runs/{id}/inline-maps/{card_ref}/sources/{source_id}",
            axum::routing::get(threads::read_inline_map_source),
        )
        .route(
            "/threads/{thread_id}/inline-visualizations/{file}",
            axum::routing::get(threads::read_inline_visualization),
        )
        .route(
            "/runs/{id}/thread/turns",
            axum::routing::get(threads::list_turns),
        )
        .route(
            "/runs/{id}/thread/token-usage",
            axum::routing::get(threads::read_token_usage),
        )
        .route(
            "/runs/{id}/agents/{thread_id}/turns",
            axum::routing::get(threads::list_agent_turns),
        )
        .route(
            "/runs/{id}/thread/archive",
            axum::routing::post(threads::archive),
        )
        .route(
            "/runs/{id}/thread/name",
            axum::routing::put(threads::set_name),
        )
        .route(
            "/runs/{id}/generate",
            axum::routing::post(generation::generate),
        )
        .route("/runs/{id}/cancel", axum::routing::post(runs::cancel_run))
        .route(
            "/runs/{id}/interrupt",
            axum::routing::post(runs::interrupt_run),
        )
        .route("/runs/{id}/steer", axum::routing::post(runs::steer_run))
        .route(
            "/runs/{id}/compact",
            axum::routing::post(runs::compact_run_thread),
        )
        .route("/runs/{id}/review", axum::routing::post(runs::start_review))
        .route(
            "/workspaces",
            axum::routing::get(workspaces::list_workspaces).post(workspaces::create_workspace),
        )
        .route(
            "/workspaces/{id}",
            axum::routing::get(workspaces::get_workspace).delete(workspaces::remove_workspace),
        )
        .route(
            "/workspaces/{id}/status",
            axum::routing::get(workspaces::status),
        )
        .route(
            "/workspaces/{id}/files",
            axum::routing::get(workspaces::list_files)
                .post(workspaces::upload_files)
                .delete(workspaces::delete_file)
                .layer(axum::extract::DefaultBodyLimit::max(252 * 1024 * 1024)),
        )
        .route(
            "/workspaces/{id}/git-roots",
            axum::routing::get(workspaces::list_git_roots).put(workspaces::set_git_root),
        )
        .route(
            "/workspaces/{id}/files/content",
            axum::routing::get(workspaces::read_file),
        )
        .route(
            "/workspaces/{id}/files/download",
            axum::routing::get(workspaces::download_file),
        )
        .route(
            "/workspaces/{id}/assets",
            axum::routing::get(workspaces::read_image_asset),
        )
        .route(
            "/workspaces/{id}/agents",
            axum::routing::put(workspaces::write_agents_file),
        )
        .route(
            "/workspaces/{id}/diffs",
            axum::routing::get(workspaces::diffs),
        )
        .route(
            "/workspaces/{id}/stage",
            axum::routing::post(workspaces::stage),
        )
        .route(
            "/workspaces/{id}/stage-all",
            axum::routing::post(workspaces::stage_all),
        )
        .route(
            "/workspaces/{id}/unstage",
            axum::routing::post(workspaces::unstage),
        )
        .route(
            "/workspaces/{id}/revert",
            axum::routing::post(workspaces::revert),
        )
        .route(
            "/workspaces/{id}/revert-all",
            axum::routing::post(workspaces::revert_all),
        )
        .route(
            "/workspaces/{id}/branches",
            axum::routing::get(workspaces::list_branches).post(workspaces::create_branch),
        )
        .route(
            "/workspaces/{id}/branches/checkout",
            axum::routing::post(workspaces::checkout_branch),
        )
        .route(
            "/workspaces/{id}/branch/rename",
            axum::routing::post(workspaces::rename_branch),
        )
        .route(
            "/workspaces/{id}/upstream/rename",
            axum::routing::post(workspaces::rename_upstream_branch),
        )
        .route(
            "/workspaces/{id}/apply",
            axum::routing::post(workspaces::apply_workspace),
        )
        .route(
            "/workspaces/{id}/terminals",
            axum::routing::post(terminals::open),
        )
        .route(
            "/workspaces/{id}/terminals/{terminal_id}/write",
            axum::routing::post(terminals::write),
        )
        .route(
            "/workspaces/{id}/terminals/{terminal_id}/resize",
            axum::routing::post(terminals::resize),
        )
        .route(
            "/workspaces/{id}/terminals/{terminal_id}",
            axum::routing::delete(terminals::close),
        )
        .route("/workspaces/{id}/log", axum::routing::get(workspaces::log))
        .route(
            "/workspaces/{id}/commits/{sha}/diff",
            axum::routing::get(workspaces::commit_diffs),
        )
        .route(
            "/workspaces/{id}/github/issues",
            axum::routing::get(github::issues),
        )
        .route(
            "/workspaces/{id}/github/repository",
            axum::routing::post(github::create_repository),
        )
        .route(
            "/workspaces/{id}/github/pull-requests",
            axum::routing::get(github::pull_requests),
        )
        .route(
            "/workspaces/{id}/github/pull-requests/{number}/diff",
            axum::routing::get(github::pull_request_diff),
        )
        .route(
            "/workspaces/{id}/github/pull-requests/{number}/comments",
            axum::routing::get(github::pull_request_comments),
        )
        .route(
            "/workspaces/{id}/github/pull-requests/{number}/checkout",
            axum::routing::post(github::checkout_pull_request),
        )
        .route(
            "/workspaces/{id}/remote",
            axum::routing::get(workspaces::remote),
        )
        .route(
            "/workspaces/{id}/fetch",
            axum::routing::post(workspaces::fetch),
        )
        .route(
            "/workspaces/{id}/pull",
            axum::routing::post(workspaces::pull),
        )
        .route(
            "/workspaces/{id}/push",
            axum::routing::post(workspaces::push),
        )
        .route(
            "/workspaces/{id}/sync",
            axum::routing::post(workspaces::sync),
        )
        .route(
            "/workspaces/{id}/commit",
            axum::routing::post(workspaces::commit),
        )
        .route(
            "/organizations",
            axum::routing::get(organizations::list_organizations)
                .post(organizations::create_organization),
        )
        .route(
            "/organizations/{id}",
            axum::routing::get(organizations::get_organization),
        )
        .route(
            "/organizations/{id}/members",
            axum::routing::get(organizations::list_members).post(organizations::add_member),
        )
        .route("/health", axum::routing::get(health::health_check))
        .route("/events/ws", axum::routing::get(events::websocket))
        .route(
            "/projects",
            axum::routing::get(projects::list_projects).post(projects::create_project),
        )
        .route(
            "/projects/managed",
            axum::routing::post(projects::create_managed_project),
        )
        .route(
            "/projects/{id}",
            axum::routing::get(projects::get_project).delete(projects::delete_project),
        )
        .route(
            "/projects/{id}/thread-contexts",
            axum::routing::get(projects::list_thread_contexts),
        )
        .route(
            "/tasks",
            axum::routing::get(tasks::list_tasks).post(tasks::create_task),
        )
        .route("/tasks/{id}", axum::routing::get(tasks::get_task))
        .route(
            "/tasks/{id}/model-selection",
            axum::routing::put(tasks::update_model_selection),
        )
        .route(
            "/tasks/{id}/messages",
            axum::routing::post(tasks::send_message),
        )
        .route(
            "/tasks/{id}/events",
            axum::routing::get(tasks::list_task_events),
        )
        .route(
            "/tasks/{id}/resource-refs",
            axum::routing::get(tasks::list_task_resource_refs),
        )
        .route(
            "/tasks/{id}/artifacts",
            axum::routing::get(artifacts::list_for_task),
        )
        .route("/providers", axum::routing::get(providers::list_providers))
        .route(
            "/providers/{id}",
            axum::routing::put(providers::upsert_provider).delete(providers::delete_provider),
        )
        .route(
            "/providers/{id}/select",
            axum::routing::post(providers::select_provider),
        )
        .route(
            "/providers/{id}/models/refresh",
            axum::routing::post(providers::refresh_provider_models),
        )
        .route(
            "/providers/{provider_id}/models/{model_id}",
            axum::routing::patch(providers::update_provider_model),
        )
        .route(
            "/providers/{provider_id}/models/{model_id}/select",
            axum::routing::post(providers::select_provider_model),
        )
        .layer(Extension(adapter))
        .layer(Extension(providers))
        .layer(Extension(approvals))
        .layer(Extension(git))
        .layer(Extension(orchestrator))
        .layer(Extension(configuration_secrets))
        .layer(Extension(profile))
        .layer(Extension(deliveries))
        .layer(Extension(copilots))
}
