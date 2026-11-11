# open-web-codex Agent Guide

This guide supplements the user-level engineering principles with rules that
are specific to this repository. Component guides may add implementation
details, but must preserve the ownership and contracts defined here.

## Engineering principles

1. Establish facts before changing behavior. Inspect the authoritative data,
   the actual execution path and relevant measurements, and distinguish
   verified facts from assumptions.
2. Fix root causes. Do not hide defects with hard-coded substitutes, silent
   degradation, fabricated success, swallowed errors, delays or visual masking.
   Cold starts, restarts, failures and recovery paths must be correct.
3. Preserve a single source of truth. Live updates, persistence, history
   recovery and derived views must have consistent semantics and converge.
4. Respect ownership boundaries. Behavior belongs in the layer that owns its
   lifecycle and semantics; prefer official contracts and upstream
   abstractions.
5. Make extensions typed and general. Use stable types, capabilities,
   registries and lifecycle contracts instead of heuristics based on names,
   display text, paths or content.
6. Give every asynchronous operation a stable identity and explicit success,
   failure, rejection, cancellation, timeout and interruption outcomes.
   Concurrency, reconnects, refreshes and out-of-order delivery must not create
   duplicate or stale state.
7. Persist state in the scope of its authoritative owner. Treat caches as
   disposable projections. Secrets must never enter logs, browser-visible
   payloads, documentation or source code.
8. Keep critical paths small and observable. Load only necessary data, avoid
   repeated full scans when an authoritative projection exists, and use
   measurements to judge performance.
9. Do not preserve compatibility with historical, non-official project
   implementations. This rule never authorizes breaking official Codex
   contracts or changing official Runtime behavior.
10. Validate normal, failure, interruption, recovery, concurrency and
    persistence paths as applicable. Work is complete only when the
    implementation, boundary validation and delivery checks all succeed.
11. Documentation describes current facts. Put historical explanation in
    decision records or version history instead of accumulating superseded
    behavior in current-state documents.
12. Before starting, inspect interrupted or in-progress work. Preserve
    unrelated user changes, keep commits single-purpose, validate the precise
    affected scope and never claim completion before the evidence supports it.
13. Treat compliance as a gate for every new requirement and every refactor.
    Before implementation, identify all applicable `AGENTS.md` files and
    authoritative project documents, then record how the proposed owner,
    contracts, capability gate, persistence scope and validation satisfy them.
    Recheck the same constraints against the final diff before delivery. If a
    requirement conflicts with a governing rule, stop and resolve the conflict
    explicitly rather than proceeding by assumption.

## Development-stage contracts and recovery

This repository is in active development. The latest checked-in implementation
and its current contracts are the only supported project state.

- The compatibility prohibition applies to historical, non-official project
  APIs, schemas, configuration, events and UI adapters. Codex app-server APIs,
  configuration, CLI behavior and Thread restoration semantics must continue to
  follow the checked-in upstream contract. Removing project compatibility code
  must not change official Runtime behavior.
- Project-owned interfaces may advance directly to the latest contract. Update
  every caller, fixture, seed, generated artifact, test and current document in
  the same change, then delete the superseded path.
- A clean environment must initialize completely from the current schema and
  configuration. Do not add runtime migration branches for historical
  non-official database schemas, configuration shapes or event formats. When
  development data is incompatible, use an explicit documented rebuild
  procedure outside the startup path; never guess or repair it automatically.
- Do not maintain old and new project APIs in parallel, dual-read or dual-write
  old and new fields, infer missing fields, or retain deprecated fields without
  a current contract owner. Generated protocol artifacts come only from the
  current Rust types.
- Prohibited fallback behavior includes silent downgrade, fake success,
  swallowed errors, guessed protocol versions, default values that hide missing
  authoritative state, and switching to an undisclosed implementation. A
  missing capability must produce a typed, explicit unavailable or failure
  result.
- Reliability recovery is allowed when it is part of the owning contract:
  bounded retries, reconnects, idempotent recovery and durable state replay.
  Recovery must be typed, observable and limited by attempts or time while
  preserving the original error cause. Never automatically retry a
  non-idempotent operation without an explicit idempotency key and contract.
- Capability gates come only from typed Codex Runtime discovery, generated
  contracts or an authoritative persisted platform capability record. Never
  infer support from version strings, file presence, error text, UI state,
  display names or hard-coded Provider names. The WebApp must not advertise a
  Runtime capability the Runtime has not reported.

## Root-cause completion criteria

An error is fixed only when all applicable conditions are met:

1. The authoritative state and the layer that owns the failure are identified.
2. The correction is made in that owning layer instead of masking the error
   earlier in the call chain.
3. The obsolete path, hard-coded substitute or duplicate source of state that
   caused the failure is removed.
4. Tests reproduce the original failure and cover the correct recovery or
   explicit terminal result.
5. The error remains observable through structured types and safe diagnostics.
6. Architecture, capability-baseline or operational documentation is updated
   when its current facts or owner contracts changed.

## North star

Build a self-hosted, browser-first, multi-user Codex workbench by reusing the
official Codex Runtime rather than reimplementing it.

The human-readable long-term product direction and staged enterprise evolution
live in `docs/product-vision.md`. The rules below are the engineering
constraints that preserve that direction while the product evolves.

Each user owns an isolated, persistent Profile containing identity,
`CODEX_HOME`, configuration, Threads, memory, Skills, Plugins, MCP state and
Provider/model selection. A Workspace is an independently authorized execution
root, not storage owned by a Thread or Run. Codex owns each Thread's current
`cwd`; the platform validates that directory against the user's authorized
Workspaces. Multiple Threads may use the same Workspace. A Run is only a
scheduling and audit attempt and never provisions or owns a checkout.

The browser reaches Codex only through the authenticated Web platform and a
versioned app-server bridge. Keep product-specific Codex changes narrow,
explicit and replayable so the `codex/` subtree can continue to synchronize
with `openai/codex`.

## Current delivery stage and evolution

The current delivery stage is single-user and single-Profile. Exact runtime
composition, authentication entry and milestone status are current-state facts
owned by `docs/architecture.md`, `docs/capability-baseline.md` and
`docs/development-plan.md`; do not duplicate them here.

For current Copilot-platform work, ADR-018 plus the package-selection and
shared-Tool decisions in ADR-019 are the normative design and development
baseline. The current stage is the built-in network-planning
Copilot's architecture correction and complete runtime closure; public SDK,
Studio, five-object publishing, Marketplace, a domain-free demo, a second domain
and multi-user product flows are later-stage work. Treat the warehouse Supervisor
path as migration input and regression evidence, not as a platform contract. A
typed scan of the explicitly configured trusted application Copilot root is the
only current package discovery path; do not extend it with Prompt wiring,
aliases, Workspace/cwd/source-tree discovery, false
installation/readiness, model-visible transaction protocols or projection-owned
workflow behavior.

In phase one, do not add a Platform-owned Task-to-Task message, context, result,
data, or adoption API. Reuse Codex-native exchange first: Thread context and
native Agent collaboration for live coordination, MCP Resources for typed
provider-owned intermediate data, ordinary files in the authorized Workspace
for user-visible durable data, and Artifacts only for explicit final
deliverables. A bounded Web projection may make exact authorized Resource
references from official Thread Items discoverable, but it must not store the
content, become a Resource Broker, or create a second context/data system.

The user, Skills, model, and Tools may use validated Workspace-relative paths
and typed MCP Resource references. Reject server-absolute paths, path or
symlink escape, ambiguous `source_ref` aliases, and historical asset/Dataset
IDs. Do not add COW, overlay, snapshots, Task-private files, Platform data
revisions, semantic registries, bindings, whole-dataset fingerprints/reuse
gates, or Artifact-based data transfer. Provider-owned pair-level reuse of
deterministic route and cost facts is a domain Tool responsibility, not a
Platform data plane.

- Do not build tenant administration, invitations, member roles, interactive
  multi-user authentication or cross-tenant UI in the current stage.
- Keep User, Organization, Profile, Workspace and owner identities in database
  records, authorization entry points, cache keys, process keys and event
  scopes. A current single-process composition must not become an unscoped
  global singleton.
- Validate current resource ownership and authorization boundaries. A complete
  concurrent cross-user isolation and denial matrix is a mandatory gate before
  multi-user operation is enabled, not a reason to build multi-user product
  flows during the single-user stage.
- Design every change against the accepted long-term architecture and stage
  order. A capability from a later stage may be implemented early only when a
  current feature has a real dependency, the object or boundary already exists
  in the long-term architecture, and the current stage has an executable
  acceptance path.
- Do not build an unused parallel framework for a hypothetical future need.
  Record an ADR when an early capability creates a lasting architectural
  constraint. Future value alone never justifies adding responsibility to
  Codex core.

## Ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| Browser WebApp | Presentation, interaction, optimistic UI, accessibility and safe rendering of typed platform DTOs | Thread/Turn semantics, model context, tool or MCP discovery, credentials, filesystem authority, raw app-server protocol or product persistence |
| Platform Server | Users, organizations, authorization, durable workflow state, Profile/Runner lifecycle, approvals, audit, Git orchestration, Secret injection, browser DTOs and durable event projections | Reasoning, context compaction, memory, tool execution, Skills/Plugins/MCP lifecycle or Provider transport internals |
| Profile Host / adapter | Isolated persistent `CODEX_HOME`, one primary app-server process per Profile, typed request bridging and safe Runtime event normalization | Product UI behavior, browser contracts, Runtime emulation or a second store for Thread state |
| Codex app-server / Runtime | Thread, Turn, Item and context semantics; compaction, memory, agents, tools, Skills, Plugins, MCP and Provider execution | Web sessions, organizations, browser authorization, browser DTOs, deployment policy or Workspace provisioning |
| Workspace / Runner / Git | Authorized execution roots, explicit managed clone/worktree lifecycle, repository operations, Run scheduling, leases, recovery and delivery | Model-visible conversation state or implicit checkout ownership by a Thread or Run |
| Skill / Plugin / MCP package | Model-visible capability instructions, declarations, tools and resources consumed through Codex discovery | Hidden Profile mutation, Web command interception or platform authorization |
| Contract layer | Generated Codex protocol facts internally and stable, bounded platform DTOs externally | Hand-maintained claims about Runtime support or raw protocol passthrough to the browser |

Codex is the authoritative owner of model-visible conversation state. Platform
events, database projections and browser caches are rebuildable views, never a
second Thread, memory or agent system.

## Non-negotiable boundaries

- Do not recreate Codex capabilities in the WebApp, server routes, startup
  scripts or database. Runtime-facing behavior goes through Codex discovery or
  a typed app-server contract.
- Do not expose raw JSON-RPC, app-server request IDs, server-absolute paths,
  credentials, configuration key paths or unbounded Runtime payloads to the
  browser. Validated Workspace-relative paths are an intentional product
  contract, not server-local authority.
- Do not add a desktop shell, Tauri layer, sidecar daemon, loopback proxy or a
  second browser-to-Runtime gateway.
- Resolve every Profile and authorized Workspace through authenticated platform
  records. Browser input is never trusted as a server-local path, and every
  Runtime `cwd` must fall within an authorized execution root.
- Scope processes, caches, subscriptions, model catalogs, Secrets, Workspace
  grants and events by their authoritative identity even in the current
  single-user composition. Cross-user access denial is a gate before multi-user
  operation is enabled.
- Persist durable platform events and approvals before browser fan-out.
  Reconnect may replay missing projections, but projections never replace
  authoritative Codex history.
- Codex-generated JSON Schema, TypeScript and capability data are protocol
  truth. Regenerate them from Rust types; never hand-edit generated artifacts
  or manually claim unsupported capabilities.

## Retained Codex customization

The target is the smallest necessary, deliberate Codex diff, not a zero-diff
subtree. Do not customize `codex-rs` when the behavior can live in upstream
configuration, app-server v2, a Skill, Plugin or MCP package, or the owning
platform layer. Every retained change must be minimal, modular, typed, tested
and replayable after an upstream synchronization.

A new or expanded `codex/` customization may not begin until its Patch Map
entry, optionally backed by a linked design record, supplies:

1. Evidence that official configuration, app-server v2, Skills, Plugins, MCP
   and the owning platform layer cannot provide the required behavior.
2. The smallest owning crate and exact intended file scope.
3. Typed inputs, outputs, capability gate and isolated validation.
4. The replay sequence after accepting a new upstream structure.
5. Its `retain-core`, `upstreamed`, `move-out` or `drop` classification.
6. An exit condition and the upstream capability that would allow the
   customization to be deleted.

Absent that evidence, do not modify Codex. A possible future requirement is not
evidence of necessity.

Only seams currently classified as `retain-core` in
`docs/custom-codex-patch-map.md` may remain. The Patch Map is the sole current
inventory and owns each seam's module placement, replay sequence,
parity/validation obligations and exit condition; do not duplicate that
inventory in `AGENTS.md`. Web, Profile, authorization and browser state never
move into `codex/`.

Before modifying Codex Runtime source, run
`scripts/codex-upstream-status.sh` and
`scripts/codex-customization-status.sh`, inspect
`docs/custom-codex-patch-map.md`, and classify every non-generated difference
as `retain-core`, `upstreamed`, `move-out` or `drop`. Official synchronization
must use `scripts/sync-codex-upstream.sh --apply` on its dedicated sync branch,
preserve upstream structure first and then replay only documented retained
seams.

## Project contracts

- A feature proposal must identify its owning layer, typed inputs and outputs,
  capability gate, persistence scope and validation path before implementation.
  Split cross-layer work until every change has one clear owner.
- Browser APIs are typed product resources with one current contract, not
  app-server passthroughs. Generated Runtime types stay behind the Platform
  Server and Profile Host.
- Workspaces exist independently of Threads and Runs. Starting, resuming or
  updating a Thread passes an authorized `cwd` through the official Codex
  contract; it does not create a Thread-owned checkout. Managed clones or
  worktrees are explicit Workspace resources with their own lifecycle and may
  serve multiple authorized Threads.
- A phase-one Task fixes one authorized Workspace. Root and child Threads use
  that same native `cwd`; the platform does not create a Task-owned checkout,
  worktree, overlay, snapshot, COW layer, or file binding. Same-Workspace Tasks
  may reuse ordinary files and authorized MCP Resources through their owning
  native contracts; the Platform does not connect their contexts or results.
- Durable Artifacts have their own identity, authorization and retention
  lifecycle. Producing Run/Thread/Turn/Item IDs are provenance only and must not
  prevent later authorized history from resolving delivery content. In phase
  one, Artifacts are reports, maps, and other explicit user deliverables; they
  are not Agent inputs or Task-to-Task data transport.
- MCP Resource content remains owned by its configured provider. Codex owns
  resource discovery/read execution and official Tool Item history; a Web
  authorization or discoverability projection may reference an exact official
  Item and `{server, uri}` but must remain rebuildable and must not copy the
  content, version it, or infer resource identity from model text.
- Provider credentials remain encrypted platform Secrets and are injected only
  into the owned Profile process. They never enter browser-readable state.
- Skills, Plugins and MCP are discovered and executed by Codex Runtime.
  Capability packages may supply declarations and launchers, but the WebApp and
  platform startup path must not simulate discovery or edit hidden Profile
  configuration.

## Project sources of truth

- Documentation authority and routing: `docs/README.md`
- Long-term product vision: `docs/product-vision.md`
- V1 product requirements: `docs/product-design.md`
- Long-term enterprise multi-agent rationale: `docs/enterprise-agent-platform-architecture.md`
- Current architecture and ownership: `docs/architecture.md`
- Normative security boundaries: `docs/security-model.md`
- Verified Runtime/platform capability: `docs/capability-baseline.md`
- Accepted stage order: `docs/roadmap.md`
- Current and next milestone execution: `docs/development-plan.md`
- Accepted phase-one Copilot design and development baseline:
  `docs/adr/018-built-in-network-copilot-runtime-closure.md`
- Accepted Task-selected package and shared-Tool baseline:
  `docs/adr/019-task-selected-copilot-packages-and-shared-tools.md`
- Accepted single-agent warehouse Copilot and business-question Tool baseline:
  `docs/adr/026-single-agent-warehouse-copilot-and-business-question-tools.md`
- Deferred phase-two public Copilot platform implementation input:
  `docs/agent-capability-lifecycle-plan.md`
- Frozen warehouse Supervisor prototype migration input:
  `docs/enterprise-supervisor-copilot-plan.md`
- Codex synchronization: `docs/codex-upstream-sync.md`
- Retained Codex seams: `docs/custom-codex-patch-map.md`

Read the documents relevant to the owning layer before changing behavior.
Keep every document inside the role defined by `docs/README.md`: target
documents must not be presented as current capability, current-state documents
must not accumulate replaced history, and plans must not claim implementation
without capability evidence. Component documents may add detail but cannot
redefine product scope, capability status, security invariants or ownership.

## Minimum delivery gates

- Follow `apps/web/AGENTS.md` for Web/platform work and `codex/AGENTS.md` for
  Runtime work.
- Run platform Rust tests through `scripts/test-web-rust.sh` and Codex tests
  through `scripts/test-codex.sh`. These wrappers preserve the component-owned
  test commands while selecting the bounded `ci-test` Cargo profile and
  enabling sccache when available. Cargo build outputs remain available for
  incremental runs; the wrappers do not enforce a storage watermark.
- Web changes require type checking and relevant tests; integration changes
  require contract coverage.
- Codex changes require its formatting and scoped test workflow. TUI changes
  require snapshot coverage.
- Protocol changes require regenerated official Codex Schema and TypeScript,
  the owner `schema_fixtures` drift gate, relevant Web and Codex checks, and a
  real `npm run smoke:codex-app-server` run against the official
  `initialize` and `thread/list` contracts.
- Authorization, persistence and recovery changes must cover denial, restart,
  interruption and concurrency as applicable.
