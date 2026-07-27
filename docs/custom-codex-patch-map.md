# Custom Codex patch map

This is the current, replay-oriented record of product-specific seams under
`codex/`. It is the authority for deciding which local changes are reapplied
after an official subtree update. Generated schemas, TypeScript definitions,
fixtures, and snapshots are derivatives of the source seams and are not
independent custom behavior.

The integrated official base is `6e5a2d6b8d148a5554fdceb6f399ca45bd1c78d9`.
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

The integrated base is `6e5a2d6b8d148a5554fdceb6f399ca45bd1c78d9`.
The observed official main is `cba0e2701c9e3e67a877a16dbbd7a577d477a630`;
126 official commits await the next dedicated synchronization branch. The raw
comparison contains 919 paths: 46 added locally, 803 modified, and 70 missing
locally, classified by the status script as 790 `upstream-only`, 70
`local-only`, and 59 `diverged`.

All product-specific differences on the integrated base are classified under
the retained seams and decisions below. The 59 diverged paths require the next
sync to accept official structure first and replay only those classified seams.
Generated app-server artifacts have no drift on the integrated base, and the
Runtime/TUI scoped validation matrix passes there. Machine-readable evidence is in
`.sync/codex-customization-inventory.json`.

Use `scripts/codex-customization-status.sh` as the inventory input. It compares
`HEAD:codex` directly with the current `codex-upstream/main` tree; this
repository's `main` branch is never the convergence baseline.
`.sync/codex-customization-inventory.json` records the latest comparison
commit, counts, and classification progress. Refresh it whenever the inventory
is updated or an official sync changes the target tree.

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
| `provider-chat-transport` | `codex-api/src/chat_translate.rs`, `chat_translate_tests.rs`, `endpoint/chat.rs`, `sse/chat.rs`; isolated Core transport in `core/src/client/chat.rs` with minimal dispatch in `core/src/client.rs` | Translates third-party Chat Completions requests, streams, mixed/interrupted Tool calls and assistant output into Codex semantics. Responses namespaces, including MCP plugin tools, are flattened to Chat functions with request-scoped reverse mapping; Responses-only tools without complete Chat semantics remain hidden. The checked-in stream translator currently marks text accompanying Tool calls as `commentary` and text-only completion as `final_answer`; this is a known non-standard heuristic scheduled for replacement by the phase-neutral, ordered contract in `docs/chat-responses-translation-spec.md`. | 1 | `just test -p codex-api`; focused Core Chat endpoint, interrupted, namespace/MCP, unsupported-tool-policy, phase-neutral Message, explicit Provider extension, ordered Item and tool-call translation tests; real third-party live/history smoke | Upstream provides equivalent supported third-party wire translation, including ordered stream lifecycle, namespace/MCP restoration and explicit rather than Tool-presence-inferred phase/reasoning behavior. |
| `provider-metadata-models` | `model-provider-info/src/lib.rs`, `PROVIDER_MODELS.md`, `model-provider/src/provider.rs`, `models_endpoint.rs`, `models-manager/src/manager.rs`, `config/src/thread_config/**`; Provider turn-client selection in `core/src/client/provider.rs` and `core/src/session/turn.rs`; minimal capability consumption in `core/src/tools/spec_plan.rs` | Defines `WireApi::Chat`, Provider-scoped model and tool-capability metadata, model discovery, selection, normalization, cache isolation, and correct transport rebinding when an existing Thread switches Provider. | 2 | `just test -p codex-model-provider-info`; `just test -p codex-model-provider`; `just test -p codex-config`; focused Core tool-plan and Provider hot-switch tests; regenerated config Schema; Provider switch/cache-isolation smoke | Upstream exposes equivalent Provider metadata, scoped catalog, capability gates, cache semantics, and live Thread Provider transport rebinding. |
| `provider-app-server-api` | `app-server-protocol/src/protocol/v2/model.rs`, `app-server/src/request_processors/catalog_processor.rs`, `app-server/src/models.rs`, request registration, generated capability declarations | Provides versioned Provider listing, Provider-scoped model listing, controlled selection/configuration, and forced refresh for both TUI and Platform Host. | 3 | `just test -p codex-app-server-protocol`; `just test -p codex-app-server model_list`; generated Schema/TypeScript; real app-server Provider smoke | Upstream provides the required stable API and generated contract. |
| `provider-tui-workflows` | Dedicated modules under `tui/src/app/event_dispatch/provider_config.rs`, `app_event/provider.rs`, `app_server_session/provider_models.rs`, `chatwidget/provider_{model_context,popups,sections}.rs` and `onboarding/auth/provider_setup{,/render}.rs`; narrow attachments in upstream-owned parents | TUI Provider selection, model selection, onboarding, refresh, configuration, and error UX are product-critical client behavior. | 4 | `just test -p codex-tui`; Provider workflow snapshots | Upstream TUI provides equivalent Provider and model workflows, or the product explicitly retires TUI parity. |
| `legacy-response-tool-history` | `app-server-protocol/src/protocol/legacy_response_tool_history.rs`, narrow integration in `thread_history.rs` | Existing Profiles can contain raw `ResponseItem` tool-call/output pairs that official semantic history projection does not materialize. | 5 | Protocol tests plus reload fixture containing raw call/output pairs | Supported Profiles no longer contain this rollout format, or upstream materializes it. |
| `capability-manifest` | `app-server-protocol/src/capability_manifest.rs`, `initialize_processor.rs`, generated Schema/TypeScript, `apps/web/contracts/codex/**` fixtures | Lets the Platform Host gate product features against the actual Runtime build instead of assuming support. | 6 | Protocol tests, generated artifacts, fixture replay, `--require-manifest` smoke | Method facts and experimental state are generated from the official protocol/build and an upstream equivalent contract exists. |

## Current inventory classification

The current comparison against `codex-upstream/main` contains 919 paths: 790
`upstream-only`, 70 `local-only`, and 59 `diverged`. The integrated base remains
the validation baseline until the 126 pending official commits are accepted on a
new sync branch. Generated artifacts, tests, and snapshots follow their owning
source seam.

| Classification | Source paths | Decision and reason |
| --- | --- | --- |
| `retain-core`: Chat transport | `codex-api/src/chat_translate.rs`, `endpoint/chat.rs`, `endpoint/models.rs`, `endpoint/mod.rs`, `sse/chat.rs`, `sse/mod.rs`, `core/src/client/chat.rs`, and minimal `core/src/client.rs` dispatch | Required third-party Chat Completions transport. Request DTOs, Responses-to-Chat conversion, model-catalog wire DTOs, tool mapping and SSE translation live in `codex-api`; Core transport logic is isolated in `client/chat.rs`, while `client.rs` retains only `WireApi` dispatch. |
| `retain-core`: Provider metadata and models | `model-provider-info/src/lib.rs`, `model-provider/src/{lib.rs,models_endpoint.rs,provider.rs}`, `models-manager/src/manager.rs`, `config/src/thread_config/**`, `core/src/client/provider.rs`, and Provider fields in `core` session/config integration | Required Provider identity, model discovery, scoped cache/refresh, Thread propagation, and per-turn model-client rebinding. Accept upstream model/catalog and client changes before replaying Provider-specific behavior. |
| `retain-core`: app-server Provider API | `app-server-protocol/src/protocol/{common.rs,mod.rs,v1.rs,v2/model.rs,v2/thread.rs,v2/turn.rs}`, `app-server/src/{models.rs,message_processor.rs,request_processors.rs}`, `request_processors/catalog_processor.rs` | Required `modelProvider/list`, Provider-scoped models, refresh, selection, Thread/Turn-level Provider override, and capability exposure. Keep only Provider request registrations and handlers when replaying high-churn dispatch files. |
| `retain-core`: TUI Provider workflows | `tui/src/{app/event_dispatch/provider_config.rs,app_event/provider.rs,app_server_session/provider_models.rs,chatwidget/provider_model_context.rs,chatwidget/provider_popups.rs,chatwidget/provider_sections.rs,onboarding/auth/provider_setup.rs,onboarding/auth/provider_setup/render.rs}` plus Provider model/config UI followers; narrow integration in upstream-owned parent modules | TUI Provider configuration, model selection, onboarding, refresh, and error UX are core behavior. Take upstream TUI orchestration first; reattach isolated Provider modules and their event handlers. |
| `retain-core`: compatibility and capability | `app-server-protocol/src/capability_manifest.rs`, `protocol/legacy_response_tool_history.rs`, narrow integration in `thread_history.rs`, `initialize_processor.rs` | The Manifest gates Platform features; legacy history preserves supported existing Profiles. Neither is browser implementation code. |
| `upstream-first, then replay` | `core/src/{codex_thread.rs,guardian/review_session.rs,session/**}`, `protocol/src/{openai_models.rs,protocol.rs}`, `app-server/src/request_processors/turn_processor.rs`, `app-server/README.md`, TUI thread-routing/event files | These files contain substantial official SessionIo, AgentRunner, model-catalog, rate-limit, paging, fork, and TUI behavior. Preserve upstream structure and reapply only the adjacent retained seam. |
| `retain-core`: Provider propagation followers | `core/src/session/{handlers.rs,turn.rs}`, `exec/src/lib.rs`, `login/src/auth_env_telemetry.rs`, `app-server` remote-thread/turn tests, and `core` stream/header/model-switching tests | These changes propagate the selected Provider through settings and the actual turn client, preserve Provider-scoped cache test isolation, or satisfy the expanded Provider metadata shape. They follow the owning Provider seam and are not independent feature surfaces. |
| `upstreamed` | `protocol/src/tool_name.rs` | The local normalization patch is removed and this file matches official Codex. Chat-only namespace flattening and reverse mapping remain inside `codex-api`, so protocol and MCP tool identity use official semantics. |
| `move-out` | `utils/home-dir/src/lib.rs` missing-`CODEX_HOME` auto-creation | Profile creation belongs to the Platform Host. `apps/web/crates/profile-host::ensure_profile_home` provisions the directory before the native Platform Server spawn; `utils/home-dir` has returned to official missing-`CODEX_HOME` rejection semantics. |
| Derived artifacts and tests | Schema, TypeScript, fixtures, snapshots, lockfiles, and focused tests not named above | They follow the owning source seam. Regenerate artifacts and update tests/snapshots through their normal build/test commands; do not classify or replay them independently. |

## Current convergence analysis

The integrated `6e5a2d6b8d14` structure has no unresolved tree conflicts. The
observed official main is 126 commits ahead, so its 59 diverged paths must be
resolved on the next dedicated sync branch. On the integrated base,
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
does not materialize legacy raw tool call/output pairs, and has no equivalent
Capability Manifest. Those seams cannot be dropped without losing existing
behavior. Upstream already owns thread-level Provider selection and
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
upstream TUI owns `::codex-inline-vis{file="..."}` parsing, streaming tracking
and local HTML link fallback. The Web platform preserves that meaning and adds
the narrow `artifact="..."` attribute for authorized typed renderers; this
extension stays outside `codex/`. Chat and Responses transports preserve both
forms as ordinary Agent Message text; neither `codex-api` nor Core may interpret
`map.v3`, register Platform Artifacts or create card Items. If official
app-server later exposes a typed equivalent, the Web extension must converge to
that official contract and be deleted.

The Capability Manifest derives method names from typed protocol request and
notification enums. The legacy response-tool history adapter is isolated in
one module and emits structured, non-sensitive debug telemetry. Neither seam
maintains parallel hand-written protocol string lists.

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
- remote Thread configuration does not currently transport these capability
  flags and therefore resolves both to the safe disabled default;
- `tool_search` remains hidden because deferred discovery and result loading
  do not have a complete Chat lifecycle; Chat Providers therefore disable the
  Provider `tool_search` capability so MCP tools stay directly visible instead
  of becoming unreachable deferred tools;
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
