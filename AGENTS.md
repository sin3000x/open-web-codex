# open-web-codex Agent Guide

This guide supplements the user-level engineering principles with rules that
are specific to this repository. Component guides may add implementation
details, but must preserve the ownership and contracts defined here.

## North star

Build a self-hosted, browser-first, multi-user Codex workbench by reusing the
official Codex Runtime rather than reimplementing it.

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
- Do not expose raw JSON-RPC, app-server request IDs, local paths, credentials,
  configuration key paths or unbounded Runtime payloads to the browser.
- Do not add a desktop shell, Tauri layer, sidecar daemon, loopback proxy or a
  second browser-to-Runtime gateway.
- Resolve every Profile and authorized Workspace through authenticated platform
  records. Browser input is never trusted as a server-local path, and every
  Runtime `cwd` must fall within an authorized execution root.
- Scope processes, caches, subscriptions, model catalogs, Secrets, Workspace
  grants and events by their authoritative identity. Cross-user access denial
  is a release requirement.
- Persist durable platform events and approvals before browser fan-out.
  Reconnect may replay missing projections, but projections never replace
  authoritative Codex history.
- Codex-generated JSON Schema, TypeScript and capability data are protocol
  truth. Regenerate them from Rust types; never hand-edit generated artifacts
  or manually claim unsupported capabilities.

## Retained Codex customization

The target is a small, deliberate Codex diff, not a zero-diff subtree. Preserve
these product-critical seams and keep them concentrated in their owning modules:

- third-party Chat Completions request, streaming and tool-call translation;
- Provider identity, configuration and Provider-scoped model metadata;
- Provider model discovery, refresh, caching and cross-Provider isolation;
- versioned app-server APIs for Provider configuration, catalog access and
  selected Provider/model propagation;
- equivalent Provider selection, configuration, refresh and failure workflows
  in the TUI.

Place transport translation in `codex-api`, Provider facts in Provider modules,
wire types in `app-server-protocol`, request handling in app-server modules and
presentation in dedicated TUI Provider modules. `codex-core` contains only the
smallest necessary transport-selection seam. Web, Profile, authorization and
browser state never move into `codex/`.

Before modifying high-churn Codex code, run
`scripts/codex-upstream-status.sh`, inspect
`docs/custom-codex-patch-map.md`, and classify every non-generated difference
as `retain-core`, `upstreamed`, `move-out` or `drop`. Official synchronization
must use `scripts/sync-codex-upstream.sh --apply` on its dedicated sync branch,
preserve upstream structure first and then replay only documented retained
seams.

## Project contracts

- A feature proposal must identify its owning layer, typed inputs and outputs,
  capability gate, persistence scope and validation path before implementation.
  Split cross-layer work until every change has one clear owner.
- Browser APIs are stable product resources, not app-server passthroughs.
  Generated Runtime types stay behind the Platform Server and Profile Host.
- Workspaces exist independently of Threads and Runs. Starting, resuming or
  updating a Thread passes an authorized `cwd` through the official Codex
  contract; it does not create a Thread-owned checkout. Managed clones or
  worktrees are explicit Workspace resources with their own lifecycle and may
  serve multiple authorized Threads.
- Durable Artifacts have their own identity, authorization and retention
  lifecycle. Producing Run/Thread/Turn/Item IDs are provenance only and must not
  prevent later authorized history from resolving embedded content.
- Provider credentials remain encrypted platform Secrets and are injected only
  into the owned Profile process. They never enter browser-readable state.
- Skills, Plugins and MCP are discovered and executed by Codex Runtime.
  Capability packages may supply declarations and launchers, but the WebApp and
  platform startup path must not simulate discovery or edit hidden Profile
  configuration.
- TUI parity is required for retained Provider capabilities; a Web-only
  Provider workflow is incomplete.

## Project sources of truth

- Product: `docs/product-design.md`
- Architecture and ownership: `docs/architecture.md`
- Verified Runtime capability: `docs/capability-baseline.md`
- Current delivery state and order: `docs/development-plan.md`
- Codex synchronization: `docs/codex-upstream-sync.md`
- Retained Codex seams: `docs/custom-codex-patch-map.md`

Read the documents relevant to the owning layer before changing behavior.
Canonical documents describe current live state only. Component documents may
add detail but cannot redefine product scope, capability status or ownership.

## Minimum delivery gates

- Follow `apps/web/AGENTS.md` for Web/platform work and `codex/AGENTS.md` for
  Runtime work.
- Web changes require type checking and relevant tests; integration changes
  require contract coverage.
- Codex changes require its formatting and scoped test workflow. TUI changes
  require snapshot coverage.
- Protocol changes require regenerated Schema and TypeScript, updated fixtures,
  Web and Codex checks, `npm run check:codex-contracts`, and a real
  `npm run smoke:codex-app-server -- --require-manifest` run.
- Authorization, persistence and recovery changes must cover denial, restart,
  interruption and concurrency as applicable.
