# Custom Codex patch map

This is the current, replay-oriented record of product-specific seams under
`codex/`. It is the authority for deciding which local changes are reapplied
after an official subtree update. Generated schemas, TypeScript definitions,
fixtures, and snapshots are derivatives of the source seams and are not
independent custom behavior.

The integrated official base is `3b45c29062ff0e76e71c91b6753290400e7fa8da`.
The target is a small, explicit set of Provider Runtime and TUI seams; it is
not a zero-diff Codex subtree.

## Long-term acceptance criteria

1. Every non-generated `codex/` difference is classified as `retain-core`,
   `upstreamed`, `move-out`, or `drop`.
2. Only the retained seams below remain after convergence. `core` keeps only
   the minimum transport dispatch required by the Provider Runtime.
3. The Web platform owns Profiles, credentials injection, authorization,
   Provider CRUD orchestration, and browser DTOs. It never exposes raw
   app-server JSON-RPC or configuration paths.
4. A current official Codex update can be replayed in the documented order,
   followed by generated-contract validation, focused Runtime/TUI tests, and a
   real Provider app-server smoke.

## Current state

The integrated delivery freeze is
`3b45c29062ff0e76e71c91b6753290400e7fa8da`. Later official commits are
intentionally deferred to the next dedicated synchronization stage. Because
official main can move independently of this repository, live pending-commit
and raw-difference counts come only from the status scripts below, not from a
number embedded in this document.

All current product-specific differences are classified under the retained
seams and decisions below. Generated app-server artifacts are derived from the
current Runtime sources; the Runtime/TUI scoped validation matrix is the
required post-sync gate. Machine-readable evidence is in
`.sync/codex-customization-inventory.json`.

Use `scripts/codex-customization-status.sh` as the inventory input. It compares
`HEAD:codex` directly with the current `codex-upstream/main` tree; this
repository's `main` branch is never the convergence baseline.
`.sync/codex-customization-inventory.json` records a timestamped comparison
commit, counts, and classification progress for audit. Refresh it whenever the
inventory is deliberately updated or an official sync changes the integrated
tree; do not treat that snapshot as a live upstream counter.

The script separates the raw tree difference into:

- `upstream-only`: the local subtree still matches the integrated upstream
  base; this is pending official work, not a local customization.
- `local-only`: the current official tree still matches the integrated base;
  this is a candidate local customization to classify.
- `diverged`: both local and current official trees differ from the integrated
  base; this needs an explicit replay or upstream-equivalence decision.

## Retained seams

| ID | Seam and source paths | Reason to retain | Replay order | Required validation | Removal condition |
| --- | --- | --- | --- | --- | --- |
| `provider-chat-transport` | `codex-api/src/{chat_translate.rs,chat_translate_history.rs,chat_translate_tests.rs,endpoint/chat.rs,sse/chat.rs}`, isolated Core transport in `core/src/client/chat.rs` with minimal dispatch in `core/src/client.rs` and same-Turn metadata propagation in `core/src/session/inject.rs` | Translates third-party Chat Completions requests, streams, mixed/interrupted Tool calls and assistant output into Codex semantics. Responses namespaces, including MCP plugin tools, are flattened to Chat functions with request-scoped reverse mapping. The bridge maps native client `tool_search` into one ordinary Chat function; completed client ToolSearch call/output pairs in canonical Thread history are projected into the current Chat request with exact typed namespace/name reverse targets and corresponding function schemas. The projection scans valid history rather than using a `turn_id` invalidation boundary, stably deduplicates identical target/schema pairs, and rejects identity/schema collisions. Non-OpenAI Chat Providers keep internal metadata only until local translation; Chat wire messages never serialize it. This request-scoped projection never mutates canonical `Prompt.tools`, Platform state, or the Core ToolRouter registry. The SSE bridge validates every returned Tool name and argument against the exact request map before emitting any FunctionCall Item; an unavailable name is a non-retryable provider protocol violation rather than a transient stream disconnect. The Core Runtime registry remains the final execution authority; Chat owns no deferred-tool plan. Plaintext native Agent messages retain the established assistant-side Chat projection, while collaboration calls use the official direct-plaintext marker; encrypted Agent messages remain a typed preflight rejection. | 1 | `just test -p codex-api`; history-loaded schema/reverse-target replay across Turns and resume-shaped input, stable duplicate suppression, orphan/failed/cancelled handling, collision/schema-conflict, unavailable-tool atomic rejection and SSE restoration tests; Chat request/stream/history round-trip; Responses prompt-loading integration; `apps/web/scripts/real-deepseek-e2e.mjs` opt-in gate records only request/response metadata and returns typed Provider failures when the model does not produce structured Tool calls | Upstream provides equivalent third-party Chat transport or configured providers expose native Responses `tool_search`. |
| `provider-metadata-models` | `model-provider-info/src/lib.rs`, `PROVIDER_MODELS.md`, `model-provider/src/provider.rs`, `models_endpoint.rs`, `models-manager/src/manager.rs`, `config/src/thread_config/**`; Provider turn-client selection in `core/src/client/provider.rs` and `core/src/session/turn.rs`; minimal capability consumption in `core/src/tools/spec_plan.rs` | Defines `WireApi::Chat`, Provider-scoped model and tool-capability metadata, model discovery, selection, normalization, cache isolation, and correct transport rebinding when an existing Thread switches Provider. Per-model capability metadata is an explicit typed `ProviderModelConfig` list on `ModelProviderInfo`; `ModelsManager` applies the exact `model_id` entry to native `ModelInfo.supports_search_tool`, with absent entries false for configured Providers and no name/description/provider inference. Function-tool support is an explicit `ModelProviderInfo.supports_function_tools` configuration/catalog fact with a safe false default; Provider listing exposes the exact bool, and configured Chat Providers that do not opt in fail tool planning with `ProviderFunctionToolsUnsupported` rather than a generic bad request or inferred capability. ToolSearch remains the model-level `ModelInfo.supports_search_tool` fact after this exact typed merge. | 2 | `just test -p codex-model-provider-info`; `just test -p codex-model-provider`; `just test -p codex-models-manager`; `just test -p codex-config`; focused Core tool-plan and Provider hot-switch tests; app-server Provider list true/false projection and typed function-tools error tests; regenerated app-server Schema/TypeScript; Provider switch/cache-isolation smoke | Upstream exposes equivalent Provider metadata, scoped catalog, capability gates, safe typed failure semantics, cache semantics, and live Thread Provider transport rebinding. |
| `provider-app-server-api` | `app-server-protocol/src/protocol/{common.rs,v2/model.rs,v2/shared.rs}`, `app-server/src/{message_processor.rs,request_processors.rs,request_processors/catalog_processor.rs}`, `codex-api/src/{endpoint/models.rs,endpoint/session.rs,endpoint/mod.rs,lib.rs}`, `login/src/auth/default_client.rs`, `model-provider/src/{lib.rs,models_endpoint.rs,provider.rs}`, generated schema/TypeScript | Provides the versioned `modelProvider/models/list` Provider-scoped fresh catalog request in addition to existing Provider listing/selection APIs, exposes the typed Provider capability response including `functionTools`, and makes Provider list return the explicit per-Provider `supportsFunctionTools` configuration fact. The protocol also preserves `providerFunctionToolsUnsupported` as a safe typed Turn error so downstream clients do not need Runtime-message heuristics. The handler resolves only the requested Provider from the Runtime config registry; `codex-api` owns the bounded streaming unary response helper and strict rich/OpenAI-compatible `/models` parsing and body-free HTTP/transport classification; `login/src/auth/default_client.rs` owns the preserving route-aware client constructor used only by this request, mirroring `create_client_for_route` while disabling URL/response-header diagnostics so default originator/User-Agent, Cloudflare/factory cookies, sandbox direct routing, custom-CA fallback, and the selected route policy remain intact; `model-provider` owns Provider auth, route, timeout, typed telemetry categories, and summaries that copy validated display names exactly. Duplicate IDs/slugs, malformed display names, and a body over the bound reject the whole catalog. This request never switches the current Provider, mutates Thread/Turn state, or touches ModelsManager/cache. | 3 | Replay upstream structure first, then reapply the `default_client.rs` preserving no-request-logging constructor, bounded `endpoint/session.rs` helper, parser, handler, Provider list capability projection, typed error conversion, and telemetry attachments. Run `just test -p codex-login default_client`; `just test -p codex-api parses_openai_compatible_catalog_and_bounds_ids`; `just test -p codex-api classifies_catalog_shape_and_transport_errors_without_details`; `just test -p codex-model-provider fresh_catalog`; `just test -p codex-app-server-protocol`; `just test -p codex-app-server model_provider_list`; `just test -p codex-app-server model_provider_models_list`; generated Schema/TypeScript and schema drift gate; real app-server initialize plus Provider method smoke. Remove the login helper when upstream exposes an equivalent preserving route-aware constructor or the catalog uses an owning route-aware pool that can disable diagnostics without dropping these defaults. | Upstream provides the required stable API, Provider-scoped fresh catalog transport, explicit capability listing and typed errors, a preserving no-request-logging route constructor, and generated contract. |
| `provider-tui-workflows` | Dedicated modules under `tui/src/app/event_dispatch/provider_config.rs`, `app_event/provider.rs`, `app_server_session/provider_models.rs`, `chatwidget/provider_{model_context,popups,sections}.rs` and `onboarding/auth/provider_setup{,/render}.rs`; narrow attachments in upstream-owned parents | TUI Provider selection, model selection, onboarding, refresh, configuration, and error UX are product-critical client behavior. | 4 | `just test -p codex-tui`; Provider workflow snapshots | Upstream TUI provides equivalent Provider and model workflows, or the product explicitly retires TUI parity. |
| `legacy-response-tool-history` | `app-server-protocol/src/protocol/legacy_response_tool_history.rs`, narrow integration in `thread_history.rs` | Existing Profiles can contain raw `ResponseItem` tool-call/output pairs that official semantic history projection does not materialize. | 5 | Protocol tests plus reload fixture containing raw call/output pairs | Supported Profiles no longer contain this rollout format, or upstream materializes it. |
| `agent-role-governance` | `config/src/config_toml.rs`, `core/src/{config/mod.rs,agent/role.rs,agent/registry.rs,agent/control/spawn.rs,tools/spec_plan.rs}`, generated config Schema and focused tests | Adds typed request contracts for the exact `agents.allowed_roles` catalog and bounded `agents.role_spawn_limits`. Runtime enforces the catalog in model-visible discovery and role application, and reserves per-role capacity atomically with the session thread slot. This does not create a Platform feature gate or change global Agent backend selection; child Roles explicitly set `features.multi_agent_v2=false`. | 6 | Config parse/rejection tests; exact role schema and execution tests; concurrent reservation/release tests | Upstream supports equivalent request-scoped Agent-role catalog and cardinality contracts. |
| `selected-plugin-mcp-policy` | `config/src/types.rs`, `core-plugins/src/loader.rs`, narrow policy application in `core/src/config/mod.rs`, selected-root propagation in `core/src/agent/{control.rs,control/spawn.rs}` and the `thread_manager.rs` spawn contract, plus `ext/mcp/tests/executor_plugin_mcp.rs` | Applies typed per-server/per-tool policy for selected Plugin roots and preserves the parent's selected capability roots across non-fork child spawn without enabling unrelated Profile capabilities. | 8 | Config and Plugin loader tests; selected MCP allowlist executor test; selected-root child inheritance test; real root/child MCP inventory assertions | Upstream provides selected-root inheritance and equivalent typed Plugin MCP server/tool restriction. |
| `multi-agent-v2-item-lifecycle` | `core/src/tools/handlers/multi_agents_v2{.rs,/spawn.rs,/message_tool.rs}` and focused handler tests | Emits bounded typed `CollabAgentToolCall` start/completion Items for V2 spawn and message operations, allowing clients to persist observable Agent behavior without decrypting or heuristically parsing child context. | 9 | Focused V2 spawn/message lifecycle tests; app-server Item projection test; real browser activity recovery | Upstream emits equivalent typed collaboration Items for V2 operations. |
| `multi-agent-v2-parent-activity-notifications` | `core/src/session/input_queue.rs`, `core/src/session/mod.rs`, `core/src/codex_thread.rs`, `core/src/agent/{control.rs,spawn.rs}`, `core/src/tools/handlers/multi_agents_v2/wait.rs` and focused tests | Keeps the V2 parent wait contract bounded and event-driven: child progress wakes the parent through an in-memory activity signal without copying child output into the parent's model context; terminal results remain mailbox messages, and spawned children preserve the protocol selected by the parent. | 9 | Input-queue activity tests; V2 wait progress-wakeup test; child completion and every-followup-turn tests; inherited-version regression test; real app-server parent/child wait smoke | Upstream exposes equivalent bounded parent-activity delivery and immutable parent-to-child collaboration-protocol inheritance. |
| `platform-runtime-path-identity-history` | `app-server-protocol/src/protocol/v2/thread.rs`, `app-server/src/{config_manager_service.rs,request_processors/thread_processor.rs}`, `core/src/{agent/role.rs,agent/control/spawn.rs,thread_manager.rs}`, narrow child identity ordering in `thread_manager.rs`, and `thread-store/src/local/{rollout_lineage.rs,thread_history.rs,thread_history/**,thread_history_materialization.rs}` | Validates relative Role paths against the owning Profile config layer, publishes child parent/Role identity before lifecycle events, reapplies the persisted native Role when a V1 or V2 child Thread is cold-resumed, and repairs lagging/inconsistent paginated projections from valid canonical rollout JSONL. The shared Role layer projection now preserves MCP plus enabled task Skills only when the server-owned Root execution config carries the typed native `agents.<role>.runtime_mcp_projection` declaration; ordinary Profile/Workspace Role files default to bounded behavior and cannot expand parent authority. Fresh spawn and cold resume use that same safe projection, while resume still restores Runtime-owned model/provider/cwd/permissions. It does not add a Platform Role or MCP configuration owner. | 10 | Relative Role-file tests; typed managed Role MCP/Skill projection, legacy marker rejection, and unmarked authority-rejection tests; child identity ordering tests; V1/V2 child cold-resume Role/MCP inventory tests; Thread-store materialization/restart tests; real history and producing-child MCP Resource read recovery | Upstream cold-resumes a persisted child with its native Role configuration and equivalent config-layer path, identity and canonical history behavior. |

## Current inventory classification

Current Web contract limitation (2026-10-15): the checked-in `TurnStartParams` and
`ThreadSettingsUpdateParams` do not expose a Provider override. The retained-seam
switching/rebinding descriptions below are not evidence of live Web Thread
switching support. Web requires a new conversation for a different Provider and
validates the Runtime Thread before sending. This correction adds no Core seam.

The integrated delivery freeze remains
`3b45c29062ff0e76e71c91b6753290400e7fa8da`. The timestamped inventory records
the comparison used for this classification; live counts must be regenerated
with `scripts/codex-customization-status.sh`. Any newly diverged path must be
reviewed during the next dedicated sync before a retained seam is replayed.
Generated artifacts, tests, and snapshots follow their owning source seam.

| Classification | Source paths | Decision and reason |
| --- | --- | --- |
| `retain-core`: Chat transport | `codex-api/src/chat_translate.rs`, `endpoint/chat.rs`, `endpoint/models.rs`, `endpoint/mod.rs`, `sse/chat.rs`, `sse/mod.rs`, `core/src/client/chat.rs`, and minimal `core/src/client.rs` dispatch | Required third-party Chat Completions transport. Request DTOs, Responses-to-Chat conversion, bounded rich/OpenAI-compatible model-catalog parsing with body-free error classification, tool mapping and SSE translation live in `codex-api`; Core transport logic is isolated in `client/chat.rs`, while `client.rs` retains only `WireApi` dispatch. |
| `retain-core`: Provider metadata and models | `model-provider-info/src/lib.rs`, `model-provider/src/{lib.rs,models_endpoint.rs,provider.rs}`, `models-manager/src/manager.rs`, `config/src/thread_config/**`, `core/src/client/provider.rs`, and Provider fields in `core` session/config integration | Required Provider identity, model discovery, scoped cache/refresh, Thread propagation, and per-turn model-client rebinding. Accept upstream model/catalog and client changes before replaying Provider-specific behavior. |
| `retain-core`: app-server Provider API | `app-server-protocol/src/protocol/{common.rs,mod.rs,v1.rs,v2/model.rs,v2/thread.rs,v2/turn.rs}`, `app-server/src/{models.rs,message_processor.rs,request_processors.rs}`, `request_processors/catalog_processor.rs` | Required `modelProvider/list`, Provider-scoped models, refresh, selection, Thread/Turn-level Provider override, and capability exposure. Keep only Provider request registrations and handlers when replaying high-churn dispatch files. |
| `retain-core`: TUI Provider workflows | `tui/src/{app/event_dispatch/provider_config.rs,app_event/provider.rs,app_server_session/provider_models.rs,chatwidget/provider_model_context.rs,chatwidget/provider_popups.rs,chatwidget/provider_sections.rs,onboarding/auth/provider_setup.rs,onboarding/auth/provider_setup/render.rs}` plus Provider model/config UI followers; narrow integration in upstream-owned parent modules | TUI Provider configuration, model selection, onboarding, refresh, and error UX are core behavior. Take upstream TUI orchestration first; reattach isolated Provider modules and their event handlers. |
| `retain-core`: legacy response-tool history | `app-server-protocol/src/protocol/legacy_response_tool_history.rs`, narrow integration in `thread_history.rs` | Preserves supported existing Profiles without creating a browser feature surface. |
| `retain-core`: exact Agent role governance | `config/src/config_toml.rs`, `core/src/{config/mod.rs,agent/role.rs,agent/registry.rs,agent/control/spawn.rs,tools/spec_plan.rs}`, generated config Schema and focused tests | Runtime exposes and executes only the exact request-scoped role catalog, with atomically enforced per-role resident-instance limits. Child delegation is disabled by explicit Role feature configuration; no product-specific global V1/V2 priority change is retained. |
| `retain-core`: selected Plugin MCP policy | `config/src/types.rs`, `core-plugins/src/loader.rs`, narrow application in `core/src/config/mod.rs`, selected-capability propagation through `core/src/agent/{control.rs,control/spawn.rs}` and the `thread_manager.rs` spawn contract, plus `ext/mcp` tests | A selected capability root is not blanket MCP authority. Runtime applies its typed per-server/per-tool policy and preserves only the parent's selected roots when spawning a non-fork child. |
| `retain-core`: V2 collaboration Item lifecycle | `core/src/tools/handlers/multi_agents_v2{.rs,/spawn.rs,/message_tool.rs}` and focused handler tests | V2 spawn and message operations emit existing typed collaboration Item start/completion events with bounded prompt and receiver metadata, instead of requiring clients to infer behavior from encrypted child context. |
| `retain-core`: Runtime path, identity and history fixes | `app-server-protocol/src/protocol/v2/thread.rs`, `app-server/src/{config_manager_service.rs,request_processors/thread_processor.rs}`, `core/src/{agent/role.rs,agent/control/spawn.rs,thread_manager.rs}`, child identity ordering in the narrow `thread_manager.rs` integration, `thread-store/src/local/{rollout_lineage.rs,thread_history.rs,thread_history/**,thread_history_materialization.rs}`, and focused tests | Profile-managed relative Role files validate against their owning config layer; child events expose parent/Role identity before lifecycle delivery; cold-resumed V1 and V2 children reapply their persisted native Role before MCP use; paginated history converges to valid canonical rollout JSONL after projection lag or restart. |
| `upstream-first, then replay` | `core/src/{codex_thread.rs,guardian/review_session.rs,session/**}`, `protocol/src/{openai_models.rs,protocol.rs}`, `app-server/src/request_processors/turn_processor.rs`, `app-server/README.md`, TUI thread-routing/event files | These files contain substantial official SessionIo, AgentRunner, model-catalog, rate-limit, paging, fork, and TUI behavior. The current upstream structure is already integrated; future updates must preserve it and reapply only the adjacent retained seam. |
| `retain-core`: Provider propagation followers | `core/src/session/{handlers.rs,turn.rs}`, `exec/src/lib.rs`, `login/src/auth_env_telemetry.rs`, `app-server` remote-thread/turn tests, and `core` stream/header/model-switching tests | These changes propagate the selected Provider through settings and the actual turn client, preserve Provider-scoped cache test isolation, or satisfy the expanded Provider metadata shape. They follow the owning Provider seam and are not independent feature surfaces. |
| `upstreamed` | `protocol/src/tool_name.rs` | The local normalization patch is removed and this file matches official Codex. Chat-only namespace flattening and reverse mapping remain inside `codex-api`, so protocol and MCP tool identity use official semantics. |
| `upstreamed`: MCP standard elicitation decoder backport | `rmcp-client/src/elicitation_client_service.rs` | Backports the official `61de0d8fe812137cec943d58309b26df1dd227b5` handling of standard `elicitation/create` when the current RMCP model supplies it as `CustomRequest`. It parses the existing `CreateElicitationRequestParams` and uses the pre-existing `send_elicitation` path; it adds no Runtime capability, protocol, feature, Manifest, or Platform behavior. Validate the direct service regression, production `dev-small` CLI, official app-server elicitation gate, and the real Platform Data-child gate. Remove this row and local hunk when the next full official Codex synchronization includes `61de0d8` or a later equivalent upstream implementation; do not replay it as a retained seam. |
| `move-out` | `utils/home-dir/src/lib.rs` missing-`CODEX_HOME` auto-creation | Profile creation belongs to the Platform Host. `apps/web/crates/profile-host::ensure_profile_home` provisions the directory before the native Platform Server spawn; `utils/home-dir` has returned to official missing-`CODEX_HOME` rejection semantics. |
| Derived artifacts and tests | Schema, TypeScript, fixtures, snapshots, lockfiles, and focused tests not named above | They follow the owning source seam. Regenerate artifacts and update tests/snapshots through their normal build/test commands; do not classify or replay them independently. |

## Current convergence analysis

The integrated `3b45c29062ff` structure has no unresolved tree conflicts. On
that frozen synchronized base,
`codex-api/src/common.rs` matches the official object exactly. Chat request
DTOs and owned Responses-to-Chat conversion live in `chat_translate.rs`; the
Core client calls that converter immediately before the Chat endpoint. The
Provider-specific live-switch attachment rebuilds a turn client in
`core/src/client/provider.rs` when endpoint/auth metadata changes, so an existing
Thread cannot retain the previous Provider's transport. The remaining attachment
points in `core/src/tools/spec_plan_tests.rs`,
`tui/src/app_event.rs`, and
`tui/src/app_server_session.rs` contain only the replayed Chat/Provider seams on
top of the current upstream files. `ClientRequest.ts` and the other protocol
artifacts are generated from Rust protocol sources and currently reproduce
without drift.

The TUI Provider form uses one inline `ProviderFormView`. The superseded
field-by-field prompt events, confirmation picker, wire picker, and unused
onboarding wire renderer were dropped after full TUI coverage proved that they
had no call path. They are not part of the retained replay seam.

Provider configuration actions, model-catalog refresh, Provider event payloads,
app-server model-context projection, chat-widget Provider context and custom
onboarding now live in dedicated Provider modules. High-churn upstream
dispatchers, session adapters and auth widgets keep only narrow module and call
attachments, so replay is driven by isolated modules instead of large inline
hunks.

The upstream tree still rejects `wire_api = "chat"`, does not expose
`modelProvider/list`, does not provide the custom TUI Provider management flow,
and does not materialize legacy raw tool call/output pairs. Those seams cannot
be dropped without losing existing behavior. Upstream already owns thread-level Provider selection and
`modelProvider/capabilities/read`; replay must reuse those APIs rather than add
parallel variants.

Chat request DTOs, model-catalog wire DTOs, wire conversion, tool identity
mapping and SSE translation are concentrated in `codex-api`. Core Chat
request preparation and Provider auth/retry/telemetry hooks are isolated in
`core/src/client/chat.rs`; `core/src/client.rs` retains only the narrow
`WireApi::Chat` dispatch. The Core branch is covered by a real mock
`/v1/chat/completions` integration test in addition to interrupted-stream,
tool-call, message-phase and namespace/MCP tests.

Inline Visualization does not create another retained Codex seam. The integrated
upstream TUI owns both native `visualize` content references and
`::codex-inline-vis{file="..."}` parsing, streaming tracking and HTML viewer assets.
The Web platform preserves that meaning through an authorized Thread-scoped
HTML/static-image endpoint and adds only the narrow `artifact="..."` attribute
for the retained `map.v3` renderer; these extensions stay outside `codex/`.
Chat and Responses transports preserve all forms as ordinary Agent Message text;
neither `codex-api` nor Core may interpret `map.v3`, register Platform Artifacts
or create card Items. If official
app-server later exposes a typed equivalent, the Web extension must converge to
that official contract and be deleted.

The legacy response-tool history adapter is isolated in one module and emits
structured, non-sensitive debug telemetry. It does not maintain a parallel
hand-written protocol string list.

Provider CRUD, Secret injection, Profile lifecycle, authorization and browser
DTO adaptation do not belong in `codex/`. Provider CRUD, controlled config
writes, model refresh normalization and browser DTOs now live in
`apps/web/crates/provider-service` and typed Server routes. The service calls the
retained app-server Provider API. The Platform Server encrypts direct credentials with an external
master key, writes only a generated environment-variable reference through the
app-server config API, and injects plaintext only into the owned Profile child.

### Third-party Chat tool policy

The retained Chat transport exposes only tools that preserve their execution
semantics on an OpenAI-compatible Chat Completions wire:

- top-level `function` tools remain directly visible;
- `namespace` function tools, including MCP plugin tools, are flattened to a
  unique `namespace__tool` Chat name and restored through a request-scoped map
  before Codex dispatch;
- standalone and hosted Web Search require the Provider's explicit
  `supports_web_search` capability; configured third-party Providers default
  to disabled while the OpenAI Provider opts in;
- image generation requires the Provider's explicit
  `supports_image_generation` capability;
- function-tool calls require the Provider's explicit `supports_function_tools`
  capability; configured Providers default to disabled and an incompatible Chat
  turn returns `ProviderFunctionToolsUnsupported` rather than guessing from a
  model name, description, wire error, or `wire_api` alone;
- remote Thread configuration transports the function-tools flag; missing
  optional values resolve to the safe disabled default. Web Search and image
  generation remain governed by their existing Runtime/provider capability
  paths and are not inferred from this Chat flag;
- native client `tool_search` is flattened to the reserved `tool_search`
  function only for typed `wire_api = "chat"` Providers. Its call and result
  round-trip through Chat history. Completed client ToolSearch call/output pairs
  in canonical history contribute exact namespace/name reverse targets and
  corresponding schemas to the request-scoped Chat map; identical target/schema
  pairs are stably deduplicated and identity/schema collisions are typed
  failures. The projection never rewrites canonical `Prompt.tools`, creates a
  Platform cache, or changes the Core ToolRouter registry. A new Turn or resume
  does not invalidate loaded Tools; native `tool_search` runs only when the
  needed Tool is absent from the current request. Failed, cancelled and orphan
  outputs do not create targets, and a history-visible Tool removed from the
  current Runtime is still rejected by Core typed execution authority. The
  Runtime's existing deferred registry and Role MCP server scope remain the
  sole source of discoverability and dispatch authority; while `tool_search` is
  visible, Chat requests disable parallel Tool calls so a Provider cannot
  legitimately combine discovery and an undiscovered business Tool in one
  response;
- all returned Chat Tool calls are validated against the exact request-scoped
  reverse map before any FunctionCall Item is emitted; unavailable names and
  malformed arguments terminate as non-retryable provider protocol violations,
  never as a transient stream disconnect or a partially accepted Tool group;
- hosted `web_search` and `image_generation` remain hidden because a generic
  third-party endpoint cannot execute OpenAI-hosted tools;
- `custom` freeform tools and unknown Responses tool kinds remain hidden
  because Chat function calling cannot preserve their input grammar.

Do not encode transport policy or tool-name classification in `core` or MCP
configuration. Provider facts live in Provider crates, `codex-api` owns wire
translation, and `core` only consumes Provider capabilities at existing tool
planning gates.

## Required boundaries

- Keep Chat translation in `codex-api`, Provider facts in Provider crates,
  Provider wire types in `app-server-protocol`, request handling in
  `app-server`, and presentation in dedicated TUI Provider modules.
- Do not add Web routes, desktop commands, Profile lifecycle, authorization,
  browser state, or raw RPC proxies under `codex/`.
- Do not hand-edit generated Schema or TypeScript files.
- Do not broaden `core/src/client.rs`; extract Provider transport behavior into
  Provider-specific modules and keep only the dispatch seam in Core.
- Do not preserve an implementation that upstream now supplies. Mark it
  `upstreamed`, return to upstream code, and remove it from this table.

## Sync and progress protocol

For every official sync:

1. Run `scripts/codex-upstream-status.sh` and compare each retained seam with
   the new upstream implementation.
   Run `scripts/codex-customization-status.sh` to record the exact source-tree
   difference set against that official commit.
2. Apply seams in the `Replay order` column.
3. Regenerate protocol artifacts after each protocol/configuration change.
4. Run the seam validations, then the Web contract checks and real smoke.
5. Update this file with the current paths, symbols, validation evidence, and
   removal conditions. Update `.sync/codex-upstream.json` and
   `.sync/codex-customization-inventory.json` with the integrated commit,
   comparison, validation, and next action. Do not retain historical status
   narratives here.

For every convergence change:

1. Add its source files to the inventory with one of the four classifications.
2. Link any retained behavior to a seam above, or create a narrowly scoped new
   seam with a reason, validation command, and removal condition.
3. Move Web/platform behavior to `apps/web` before deleting its Runtime
   counterpart.

## Legacy response tool history audit

Official paginated rollout history persists semantic `ItemCompleted(TurnItem)` records and
`thread_history_projection.rs` intentionally ignores raw `ResponseItem` records. Existing Profiles
can still contain older rollouts with only `FunctionCall`/`CustomToolCall` and matching output
records. Without the compatibility seam those tool calls disappear after reload.

The seam is restricted to protocol projection:

- it correlates raw calls and outputs by `call_id`;
- a richer semantic item with the same ID always wins;
- an unmatched call is closed as failed when its Turn ends;
- it does not change model-visible context, tool execution or Thread/Turn ownership;
- it is isolated in one module so an upstream replacement can remove it atomically.

No additional Codex changes are required for browser event replay. Live notification durability,
cursor replay and UI reconciliation are owned by `apps/web`.
