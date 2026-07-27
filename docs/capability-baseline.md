# Capability baseline

This document records checked-in, directly observed capability state. Product
requirements belong in `product-design.md`; planned work belongs in
`development-plan.md`.

## Snapshot

Observed on 2026-07-25 from the current working branch:

| Component | State |
| --- | --- |
| Codex subtree | integrated through `openai/codex` `6e5a2d6b8d148a5554fdceb6f399ca45bd1c78d9` |
| Observed official main | `cba0e2701c9e3e67a877a16dbbd7a577d477a630`; 126 commits await the next dedicated sync branch |
| Local Codex seams | retained changes remain classified by `docs/custom-codex-patch-map.md`; compare them against `codex-upstream/main`, never this repository's `main` |
| Local customization footprint | six retained Runtime/TUI seams, derived artifacts and focused tests; `ToolName` uses the official implementation |
| Web platform | Restored browser UI, Axum/PostgreSQL platform, native Profile Registry/Host, encrypted Provider Secret injection, durable approvals, current per-Run Git workspaces, lease-based Run orchestration, typed REST resources and authenticated WebSocket |

## Reproduced evidence

- `scripts/codex-upstream-status.sh` reports the subtree integrated through
  `6e5a2d6b8d14` with 126 official commits awaiting a dedicated sync. The
  customization status script reports 919 raw path differences: 790
  upstream-only, 70 local-only and 59 diverged.
- The current upstream structure and all six documented seams are integrated;
  regenerated app-server Schema and TypeScript fixtures have no drift.
- The locally built `codex app-server` completes `initialize` and returns
  `capabilityManifest`, `codexHome`, `platformFamily`, `platformOs` and
  `userAgent`.
- The observed manifest contains 18 declarations, including
  `models.providers`.
- On the current Runtime lineage, `just fmt`, app-server and config Schema
  generation, 288 app-server protocol tests, 175 Chat transport tests, 26
  Provider metadata tests, 57 Provider transport tests, 41 model-manager tests,
  17 focused Core Chat tests, 27 focused Core Provider tests, five app-server
  model-list tests, and the 3,233-test full TUI suite pass.
- The latest official authentication routing, forked approval-reviewer,
  cross-environment Turn diff and `PathUri` canonicalization changes are
  integrated. Their focused evidence is 159 login, 19 app-server fork, 24 Core
  Turn diff, 86 apply-patch and 60 `PathUri` passing tests.
- The Web contract check passes, and the locally built current Codex CLI passes
  the real app-server initialize Smoke with 18 Capability Manifest declarations.
- The platform source contains PostgreSQL migrations and API handlers for
  bootstrap/session, organization membership, project, Task, Run and persisted
  Run events. The current single-user Server ensures an implicit local Owner;
  the browser obtains a local Session and enters the WebApp without login or
  registration. Username/password login and its Argon2id implementation remain
  in the Server for later multi-user restoration but are not exposed by the
  current browser entry.
- The native Profile Host real-binary smoke covers an offline Turn, paginated
  full-item history, process-instance rotation, restart and Thread resume/read.
  A second real-binary Provider smoke covers two custom
  Providers, forced model refresh, switching, cache isolation and omission of
  direct credentials from returned catalogs.
- Blank PostgreSQL migration and restart tests pass. AES-256-GCM Provider Secret
  storage is identity-bound, and the real app-server secured-Provider smoke
  proves that Codex config receives only a generated environment key while
  ciphertext and private child environment are removed together on deletion.
- The authenticated HTTP security regression uses two Organizations and proves
  resource-ID isolation, Profile-owner enforcement, session Organization
  switching, role-gated writes, durable approval decision delivery, uncertain
  delivery retry, Runtime request-id reuse isolation across process instances,
  stale-request cancellation and an audit record. Passwords use Argon2id;
  accepted legacy SHA-256 hashes are upgraded on successful login.
- Git Runtime validation covers source/ref rejection, private mirror creation,
  one clone per Run, locking, selected-path commit, status projection and
  cleanup. Run orchestration covers idempotent creation, `SKIP LOCKED` leasing,
  heartbeats, cancellation, recovery and authorized workspace binding. This is
  the checked-in implementation, not the target ownership model: it must
  converge to independent authorized Workspace resources and official Codex
  Thread `cwd` semantics. Neither a Thread nor a Run should implicitly own a
  checkout.
- The browser uses only typed platform REST resources and an authenticated
  `/api/events/ws` stream. Initial navigation snapshots establish the latest
  durable Task cursor after subscription; reconnects replay only later
  sequences, and live delivery is filtered by Organization. The former local gateway, raw RPC,
  query-token event stream and desktop application are absent.
- The checked-in 1421 WebApp presentation and CSS match the established WebApp.
  `src/services/webClient.ts` is the primary compatibility seam; three complete
  source-file hashes pin the reviewed non-visual Thread-context wiring in
  `WebApp.tsx` and FileManager so the exception cannot expand into UI drift.
  Typecheck, production build and no-desktop gates pass; all 1,210 browser
  tests pass. The UI-parity report still records the deliberate browser UI
  extensions that have not yet been folded into its reference baseline.
  Direct-Server tests cover authoritative history,
  reconnect/resync replay, status recovery, current selected Thread's
  Run-backed checkout projection,
  Provider/model defaults, approvals, structured input, MCP and rate limits.
- The 1421 WebApp adapter currently covers managed Projects, Threads/Turns,
  durable events, approvals and structured input, Provider/model selection,
  Profile rate limits, MCP status, workspace files, Git status, message send,
  interrupt and steer. A real Codex/DeepSeek journey verifies Provider add and
  switch, code execution, real stdio MCP invocation, approval resolution,
  delayed Turn state, cross-Thread history restore, file preview and durable/live
  event ordering. Browser smoke additionally verifies Running-to-Idle sidebar
  convergence while switching Threads. Both root and `/web` load only WebApp;
  the old root App/Bridge remain as unreferenced source and are intentionally
  deferred from pruning.
- The latest official managed-config exact-value enforcement, missing sandbox
  path handling, and skill-name metrics sanitization changes are integrated.
  Their config, MCP, Core Skills, protocol and app-server regressions pass on
  the synchronized tree.
- The latest official loopback proxy allowlist behavior is integrated without
  adding a local customization. All 207 network-proxy tests pass: 206 pass in
  the local-port environment, while its DNS-failure case passes under network
  isolation because the host resolver otherwise synthesizes an address for the
  reserved `.invalid` name. The added CLI sandbox cases are Linux-only and do
  not compile as tests on the current macOS host.
- The latest official named `/new` and `/clear` session lifecycle is integrated
  through the upstream TUI structure; the Provider event and slash-command
  seams merge without a parallel session implementation.
- Official repository-rule review attribution, failed-turn TUI recovery, and
  inherited-FD Windows process-tree coverage are integrated without new local
  seams. The failed-turn TUI regression passes with the Provider dispatcher
  attachment intact.
- The `6e5a2d6b8d14` update is conflict-free against the retained seams. Its 63
  focused realtime-conversation tests pass, as do the HTTP client, LM Studio
  and model-manager suites. The broader scoped run passes 3,144 of 3,149 tests;
  the remaining five require binding local mock ports or launching a nested
  exec-server, both denied by the current validation sandbox.

The successful checks prove only the surfaces named above. They do not prove
multi-Profile process routing, production sandbox strength, complete Cookie/CSRF
security, Push delivery, or every Studio capability.

## Runtime capability assessment

| Capability | Manifest/source state | Validation status |
| --- | --- | --- |
| Protocol Schema | available | generated JSON/TypeScript artifacts exist |
| Capability negotiation | available, provisional | `initialize` emits schema version, build identity, protocol range, status, limits and reasons; method registries validate Manifest wire-name refs, experimental consistency, and product attribution policy; capability declarations remain hand-assembled Alpha subset rather than full generated policy |
| Thread lifecycle | declared supported | real start, persisted full-history pagination, process restart, resume and read smoke passed. New platform Threads select official paginated history. Unloaded Threads resume with `excludeTurns`; the adapter concurrently reads indexed `thread/turns/list(itemsView=notLoaded)` and `thread/items/list` pages and joins Items by stable Turn id. Existing legacy rollout histories retain one isolated `thread/turns/list(itemsView=full)` compatibility branch until those Profile histories are retired. The Server inserts durable platform approval projections into their original Turn/sequence positions. Inline Visualization references currently use Run/Thread ownership while retaining producer Turn/Item provenance; this scope is a platform limitation rather than a Codex Thread fact and must be replaced by durable Artifact authorization that survives later Runs. The browser does not restore approvals from local storage, and whitespace-only Agent messages are omitted. Multi-cwd and official `cwd` authorization remain gates |
| Turn lifecycle | declared supported | real offline Turn start/completion and post-restart recovery passed |
| Approval lifecycle | declared supported | command, file and permission requests are persisted before a request-id-free browser projection; decisions use optimistic versioning and audit; process-instance identity prevents stale response delivery and supports request-id reuse; uncertain delivery retry and restart cancellation regressions pass. Expiry remains a gate |
| Profile multi-workspace | declared supported | manifest limits are present; ownership and concurrency behavior remain unverified |
| Memory lifecycle | declared unsupported | Codex contains compaction/memory surfaces, but the Web-safe status/export/reset bridge is absent |
| Native Agent CRUD | declared unsupported | no stable Web-safe CRUD/validation contract |
| Multi-agent trajectory | declared experimental | Codex parent/child and collab events exist; fixture and real trajectory smoke are pending |
| Skills | degraded/unsupported by operation | listing is declared; a real new Thread proves the selected `local-maps-mcp` capability root injects the `map-utils` Skill into Runtime context. Profile-wide listing does not enumerate Thread-selected roots; safe write, validation and isolated testing are not enabled |
| Plugins | declared unsupported | do not enable Studio lifecycle or permissions UI |
| MCP | config degraded; OAuth/full-form elicitation unsupported | `map_utils` uses one selected Mapbox or Google credential and typed configuration elicitation. All normal map Tools are preapproved; provider, credential, delivery, API, timeout, and network failures remain explicit. The current `map.v3` Mapbox-style contract is covered by unit and launcher smoke; the credentialed real-model smoke remains to be rerun. |
| Tools discovery | declared unsupported | do not expose a platform fallback catalog |
| Structured reply cards / map cards | available | `create_map_card` publishes one `map.v3` Artifact contract. Sources are platform-managed GeoJSON with mutually exclusive standard inline `data` or Open Web `data_ref`; authorized Resource refs become opaque Artifact URLs. Layers are official Mapbox Style Specification Layer JSON, validated by `@mapbox/mapbox-gl-style-spec` and passed to `map.addLayer` unchanged except for browser-local layer/source IDs. There is no hand-maintained paint/layout/filter/expression whitelist or map-specific source/layer/payload count limit. Official unknown-property diagnostics are warnings; invalid known syntax fails. Standard camera fields are top-level, while optional text-only hover and legend behavior lives under `extensions`. The Server independently projects the stable browser DTO, and Assistant messages render only explicit standalone Artifact embed directives. |
| Provider/model management | declared supported by the checked-in Runtime | `models.providers`, `modelProvider/list`, controlled Profile config writes, provider-scoped refresh and context-window persistence are wired. A model refresh or context-window edit schedules Server-owned app-server replacement at the next safe Turn boundary: an in-flight Turn is preserved, the adapter invalidates process-local bindings, resumes the same persisted Thread and starts its next Turn against the rebuilt model catalog. The browser groups built-in, local and custom Providers from Runtime-supplied kinds, defaults the built-in and local groups closed, distinguishes the LM Studio and Ollama `gpt-oss` entries, and switches by clicking the Provider row. Editable model context windows use one save action that persists every changed model sequentially so catalog replacements cannot race. The current Provider and model use a shared high-contrast selected treatment in both themes. The platform stores the last Provider/model pair as a global default and copies it into every new Task; each existing Thread keeps its own database-backed pair. Scoped Runtime/TUI tests, two-Provider cache-isolation smoke, encrypted platform Secret injection/deletion smoke, live existing-Thread Provider transport rebinding and real same-Thread next-Turn context refresh pass |

## Web platform assessment

| Surface | Current state | Production gap |
| --- | --- | --- |
| Independent server | Axum server serves the browser, REST API, authenticated WebSocket, Profile Host and Runner from one deployable; the single-host deployer builds locked Release artifacts, securely provisions or verifies the fixed `open_web_codex` database, keeps verbose output in bounded logs, health-checks rollout and persists non-secret status metadata | HTTPS reverse proxy, OS supervision, rollback, backup/restore and remaining config hardening are still external GA gates |
| Persistence | PostgreSQL migrations cover users/sessions, organizations/memberships, Profiles/capabilities/encrypted Secrets, projects, tasks, Runs, leases, Workspaces, durable approvals/audit and versioned Run-event projections | artifacts, retention, legacy-row repair and complete constraints remain missing |
| Authentication | current single-user startup creates an implicit local Owner and the browser obtains a local Session without credentials; sessions still bind an Organization and all resource authorization remains active; retained bootstrap/login use Argon2id | interactive login/registration is intentionally absent; public or multi-user deployment requires restoring authentication, HttpOnly-only sessions, CSRF, rate limiting and complete logout/revocation flows |
| Authorization | Project/Task/Run and runtime calls enforce session Organization; Provider/approval calls additionally enforce Profile ownership; a two-Organization denial regression passes | centralized policy abstraction, Project-specific roles and the full concurrent multi-user matrix remain missing |
| Codex bridge | Fake/Real adapter and event projection exist; Real uses the native Profile Registry/Host JSONL connection. Provider Secrets are encrypted and injected only into the owned child environment. Runtime-facing operations remain internal and browser routes are typed | composition is still one configured Profile process per server; per-user dynamic process routing remains incomplete |
| Task/Run | CRUD/start/cancel/message/steer/compact/review, idempotent scheduling, DB leases/heartbeats/recovery, per-Run Git workspaces, authoritative Codex history, safe Item/Delta and approval projection, snapshot-cursor initialization plus monotonic reconnect replay, terminal execution, workspace files, nested Git roots, full local Git operations and explicit remote operations exist | replace per-Run checkout ownership with independent authorized Workspace resources; pass and validate official Codex Thread `cwd` without creating a Thread-owned checkout. Artifact storage, approval expiry, protected-branch policy and full multi-Profile routing remain incomplete |
| Browser | established 1421 WebApp presentation runs through typed resources for workspace/thread/message, approvals, Provider/model, MCP/rate-limit snapshots, files and Git status; files, Git and MCP currently resolve the selected Thread's Run-owned Workspace. Thread creation opens an immediate client-ID-bound temporary window named `Thread`, supports concurrent out-of-order responses and retryable failure with disabled input, and replaces the sidebar and conversation-header label together when the Server returns a name. The first successful text message derives a bounded Server-persisted title for placeholder Threads and returns it with the Turn-start response, while a data migration repairs existing placeholder titles from their earliest durable user-message event. Thread switching hides uncached history behind a loading state until its authoritative projection has rendered; an unchanged completed Thread already loaded in the current session renders directly from its event-invalidated cache. A cold page refresh establishes lightweight Task event cursors rather than replaying the entire durable delta history, and MCP sidebar state comes from the latest persisted startup-status projection instead of the full Runtime tools/resources inventory. It projects the Task Provider/model from the cached catalog, performs a catalog cache-miss lookup in the background and never writes the global Profile selection merely because a Thread was opened. The shell fills the full viewport and progressively expands its sidebar and conversation column on 2K/4K displays. The real core journey and Thread-switch browser smoke pass; no standalone Gateway or old root Bridge is loaded or built | resolve files, Git and MCP through an authorized Workspace selected independently of Thread/Run storage; validate the Codex Thread `cwd` at every operation. Broader visual/accessibility regression, deferred unused-source pruning, cookie-only sessions and production accessibility remain incomplete |

## Immediate capability gates

1. Derive the remaining Manifest method sets and experimental state from
   generated Codex protocol/build facts.
2. Keep the digest-addressed contract bundle and Web feature policy in sync.
3. Keep the passing real Thread restart/resume/read and Provider/approval/MCP
   journey as a release gate; add multi-cwd and multi-agent isolation coverage
   before promoting the corresponding declarations to product support.
4. Replace the single configured Profile composition root with authorized
   per-user Profile routing before multi-user Beta.
5. Replace per-Run checkout provisioning with independent authorized Workspace
   resources and official Codex Thread `cwd` handling. Cover shared-Workspace
   concurrency, authorization, resume, explicit lifecycle and Git-operation
   containment.
6. Replace Run/Thread-owned Artifact storage with durable Artifact identity and
   authorization so embedded content survives later Runs; keep Turn/Item only
   as provenance.
