# open-web-codex Web platform

This directory contains the browser client and the session-backed platform that
hosts the official Codex runtime. It has no desktop application, local daemon,
or browser-to-Codex protocol bridge.

## Runtime shape

```text
Browser
  -> versioned REST resources + authenticated WebSocket
  -> open-web-codex-server
       -> PostgreSQL authorization and durable workflow state
       -> Profile Host -> codex app-server
       -> Workspace service -> authorized roots + managed Git resources
       -> Run orchestrator -> validate selected Workspace + execute/audit
```

The browser never receives local paths, credentials, app-server request IDs, or
raw JSON-RPC. Codex remains the owner of Thread/Turn, context, memory, tools,
skills, plugins, MCP, and multi-agent execution.

Workspaces are independent authorized execution roots. A Thread keeps its
current `cwd` in Codex; a Run references the Workspace selected for that
attempt. Neither object implicitly creates or owns a checkout.

## Requirements

- Node.js 20 or newer and npm
- Stable Rust toolchain
- PostgreSQL
- Git
- A Codex binary for real Runtime mode

## Local development

From the repository root, start the same-origin WebApp and Server together:

```bash
./scripts/run-local.sh --fake --background
```

Open `http://127.0.0.1:4800/web`. The current single-user flow creates an
implicit local Owner and Session, skips login and registration screens, and
opens the WebApp directly. Server-side Session, Organization, Profile and
resource authorization remain in use. No daemon or Gateway process is started.
Real Runtime mode is the default when `--fake` is omitted; see
[`../../docs/mvp-runbook.md`](../../docs/mvp-runbook.md) for configuration.

For frontend hot reload, keep the platform Server running and start Vite:

```bash
./scripts/run-local.sh --fake --background

# another terminal
cd apps/web
npm ci
npm run dev
```

Vite uses port 1420 by default and proxies `/api` and WebSocket upgrades to
`127.0.0.1:4800`.

## Single-host release deployment

From the repository root:

```bash
./scripts/deploy.sh
```

This validates deployment policy and delegates the optimized build and
health-checked service replacement to `run-local.sh`, the single build and
service-lifecycle owner. The Platform Server and Runtime components use exact
Cargo dependency fingerprints. The platform Server serves the built WebApp
directly at `http://127.0.0.1:4800/web`; Vite is not part of the deployed
topology. Deployment output is kept in
`.local/open-web-codex/logs/deploy.log`, and build or startup details are kept
in `.local/open-web-codex/logs/run-local.log`.

Without `DATABASE_URL` or a saved credential file, an interactive deployment
offers to connect to an existing PostgreSQL server or create an application
role and database. The database name cannot be changed from `open_web_codex`.
Passwords are not echoed or included in process arguments; the generated URL
is kept at `.local/open-web-codex/database-url` with mode `600`. A
non-interactive deployment fails before compilation when database configuration
is absent.

```bash
./scripts/deploy.sh --check
./scripts/deploy.sh --status
./scripts/deploy.sh --stop
```

The deployer reuses the same persistent Profile, Secret Store, Runner, PID and
server-log directories as `run-local.sh`. Repository build workflows use a
bounded sccache when available and separate `dev-small`, `ci-test` and release
profiles. Cargo target outputs are retained for incremental builds; repository
scripts do not apply an automatic storage watermark or profile cleanup. A
public deployment still requires an HTTPS reverse proxy, external Secret Store
key management, PostgreSQL backup/restore and an OS-level service supervisor.

## Validation

Run these commands from the repository root:

```bash
(cd apps/web && npm run check:no-desktop)
(cd apps/web && npm run lint)
(cd apps/web && npm run typecheck)
(cd apps/web && npm run test)
(cd apps/web && npm run build)
(cd apps/web && cargo fmt --all --check)
./scripts/test-web-rust.sh
(cd apps/web && npm run test:codex-harness)
```

PostgreSQL integration tests use `TEST_DATABASE_URL` and are ignored by default.
The real app-server smoke additionally requires a built Codex binary.

With an isolated real Server already running and the Codex/MCP test binaries
built, run the deterministic single-agent acceptance gate:

```bash
E2E_BASE_URL=http://127.0.0.1:4810 \
npm run test:e2e:real-platform
```

The harness starts its own deterministic Chat model server, registers it as a
Provider, creates a managed Project, Workspace and Task for
`warehouse-network-single-agent`, uploads the six fixtures and drives one Root
Turn: data preparation, one `evaluate_baseline` call with no route, cost or
prepared-input arguments, the coverage map data and the map card. It asserts the
exact Tool sequence, completed MCP Items, Resource provenance for the baseline
and map data only (route and cost matrices are internal and must not be linked),
the persisted prepared file, the standalone map embed, and that no collaboration
Item or child Agent is projected.

The real Provider gate is opt-in and shares the bounded Provider proxy, fixture
upload, Task/Run wait, timeline projection and terminal cleanup in
`scripts/real-deepseek/harness.mjs`; its prompt and typed acceptance helpers live
in `scripts/real-deepseek/single-agent.mjs`. `E2E_REAL_DEEPSEEK_SELF_TEST=1` runs
its local-only self tests without any network access.

单 Agent 门固定选择 `warehouse-network-single-agent`。它要求 Agent 对完整 prepared input 创建并执行
calculations 脚本，由 Planner 用完整报价的分层单位成本均值构建经校验并绑定证据的成本，再求解满足
12h 需求加权覆盖至少 90% 的最少新增仓方案。门禁直接断言 completed
`find_sites_for_service_target` 的 typed 结果（两阶段求解与 90% 覆盖）、其 `basis` 中绑定的
`observed_quote_mean` 证据与证据路径、无 child 协作事件，以及 prepared/calculations 文件全部位于
package-owned 输出目录；不要求模型遵循固定 Tool 次数或顺序。Protocol failures (invalid wire Tool,
accepted Tool followed by a disconnected stream, permission/identity/server failure, or cleanup
failure) remain typed terminal failures; the script does not retry a prompt variant or fabricate a
result:

```bash
E2E_REAL_DEEPSEEK=1 \
E2E_REAL_DEEPSEEK_TOOL_CHOICE_MODE=observe \
E2E_BASE_URL=http://127.0.0.1:4810 \
E2E_REAL_DEEPSEEK_SOURCE_PROVIDER_ID=deepseek \
npm run test:e2e:real-deepseek-single-agent
```

This gate requires the source Provider to already have a Platform-managed
credential. It does not print or persist the credential value. A first real
failure is classified from the bounded timeline; because model randomness is
not a mechanism fix, any manual retry is owned by the operator and limited to
two attempts.

Each gate refreshes the temporary Provider's model catalog through the official
Provider API, then persists `supportsSearchTool=true` for the exact
`deepseek-v4-flash` model through the typed model metadata endpoint. It does
not write `model_catalog_json`, infer capabilities from the model name, or retry
with prompt variants. The multi business task must add only
`WH-CANDIDATE-BALIKPAPAN` to the baseline and return both city-count and
demand-weighted 12h coverage-rate deltas. A missing typed business result is a
typed failure. The Provider capability, Run, Workspace and Project are restored
or removed in terminal cleanup.

The default `observe` mode never rewrites `tool_choice`; the optional
`force_first_tool` mode is diagnostic only. The Chat bridge preserves completed
client ToolSearch call/output pairs from canonical Thread history and projects
their loaded schemas and reverse targets into each current Chat request; it
consumes internal metadata locally and never serializes it into Chat `messages`.
The production gate verifies
structured Skill selection, Data preparation, Network route/12h calculation,
a typed Balikpapan facility-change assessment, and completed
`create_network_map_card` delivery with canonical provenance.
All generated Workspace files are constrained to
`outputs/warehouse-network/{prepared,requests,calculations,deliverables}/`; the
gate rejects root-level or source-directory outputs.

The D6 warehouse package gate disables `shell_tool` in the native Root, Data
and Network Role configs only; it does not change the Profile-wide capability
set. Root retains native collaboration, `tool_search` and user-input tools,
while the child Roles retain their typed domain MCP surfaces. The Data and map
Skills treat successful preparation and `create_network_map_card` results as
terminal Tool outcomes, so the model must hand off or deliver the returned
reference instead of continuing exploratory or low-level map calls. The real
gate timeline records bounded typed map status/errors and producer provenance;
it never records keys, full prompts, schemas or arguments. A model-side typed
failure or an unexposed wire name such as `bash` remains a failure and is not
presented as a successful map delivery or shell capability.

## Layout

```text
src/WebApp.tsx             established browser WebApp UI
src/services/webClient.ts  narrow WebApp-to-Server compatibility seam
browser/client.ts          authenticated typed REST/WebSocket transport
browser/browser-entry.ts   implicit local Session and WebApp entry
server/                    Axum composition root and session-backed routes
crates/auth/               retained sessions, password hashing, and RBAC
crates/profile-host/       persistent CODEX_HOME and app-server lifecycle
crates/profile-registry/   single-owner Profile process registry
crates/provider-service/   authorized Provider orchestration
crates/secret-store/       encrypted credential persistence
crates/git-runtime/        mirrors and independent managed Workspaces
crates/run-orchestrator/   leases, recovery, cancellation, and execution
crates/approval-service/   durable app-server approvals
crates/platform-store/     PostgreSQL state and event bus
crates/platform-contracts/ browser-safe DTOs
migrations/                platform schema
scripts/                   contracts, smoke tests, and boundary checks
```

Canonical product, architecture, security, capability, roadmap and delivery
documents are routed by the repository-level
[`docs/README.md`](../../docs/README.md).
