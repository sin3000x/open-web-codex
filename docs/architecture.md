# Architecture

## Architectural objective

`open-web-codex` is a multi-user Web control plane around the official Codex
runtime. It is not a browser reimplementation of Codex. The design maximizes
reuse of `codex/`, keeps product-specific Runtime changes behind narrow
app-server contracts, and preserves regular subtree synchronization with
`openai/codex/main`.

The initial deployable is a modular monolith with PostgreSQL and colocated
Profile Host/Runner processes. Boundaries are interfaces and ownership rules,
not a requirement to create network microservices. Components separate only
when measured capacity or isolation needs justify it.

## System shape

```text
Browser
  -> authenticated platform HTTP + WebSocket API
  -> authorization / Task / Run / Approval services
  -> Profile Host ---------------------> persistent per-user Profile Home
  -> Codex app-server                   -> Thread / Turn / memory / agents
  -> event normalizer and durable projection

Workspace authorization service
  -> authorized existing execution roots
  -> explicit managed clone/worktree resources

Run orchestrator
  -> validates the Thread's Codex cwd against authorized Workspace roots
  -> Runner sandbox / Git delivery

Codex build
  -> generated protocol Schema + TypeScript
  -> generated Capability Manifest + fixtures + digest
  -> Web feature policy and compatibility gate
```

The checked-in repository has one browser bridge. `apps/web/src/platform` calls
typed platform resources under `/api`; live updates use an authenticated
WebSocket whose first frame carries the session token. `apps/web/server`,
`apps/web/crates` and `apps/web/migrations` own the server boundary. There is no
local sidecar, raw browser JSON-RPC route, query-token event stream, or trusted
browser-supplied filesystem path.

The established React component tree and styles remain the browser product.
Desktop imports are replaced at their original call sites by browser adapters,
so UI components do not own platform authorization or transport details. Native
window, tray, updater, daemon and desktop file-manager actions either map to a
safe browser capability or return an explicit deployment-managed/unavailable
result; they never cause a Tauri runtime to reappear.

## Facts and ownership

| Fact | Authoritative owner | Web may persist |
| --- | --- | --- |
| User, organization, membership and session | Web platform database | complete platform record |
| Project, Task, Run, Thread model selection, lease, approval and audit | Web platform database | complete platform record |
| Profile ownership and process health | Web database + Profile Host | mapping, health, build and capability snapshot |
| Thread, Turn, items, compaction and model-visible context | Codex Profile/app-server | opaque IDs, event projection and search index |
| Provider config and runtime model catalog | Codex Profile/app-server | secret references, global default Provider/model selection, policy and display cache scoped to Profile |
| Agent scheduling and parent/child execution | Codex runtime | observable trajectory and status projection |
| Skills, plugins, MCP and memory state | Codex Profile/app-server | permissions, audit and capability-gated projection |
| Thread current working directory | Codex Profile/app-server | authorized Workspace ID and safe display metadata |
| Workspace authorization and managed checkout lifecycle | Web platform + filesystem/Git | complete authorization record and safe lifecycle metadata |
| Repository objects and checkout contents | Filesystem/Git | status, diff summary and artifact references |
| Durable Artifact identity, authorization and retention | Web platform Artifact store | producer Run/Thread/Turn/Item provenance and safe references |

The platform must recover model-visible history from Codex. Event projections
are rebuildable UI/read models and never become a second Thread store, memory
engine or agent scheduler.

## Web / app-server / Codex server boundary contract

All feature work must start by selecting the owning layer below. A change that
cannot be placed in exactly one owner must be split until each part has a clear
owner and contract.

| Layer | Owns | Must not own |
| --- | --- | --- |
| WebApp / browser | Presentation state, input controls, optimistic UI, safe rendering of platform DTOs, accessibility, and browser-only fallbacks | MCP/Skills/Plugins discovery, tool catalogs, model-visible prompt injection, Thread/Turn semantics, filesystem authority, credentials, raw app-server JSON-RPC, or local Profile paths |
| Platform app-server | Authentication, authorization, Profile/Runner lifecycle, Task/Run/Approval/Git persistence, Secret injection, audit, durable event projection, typed browser DTOs and capability gating | Model reasoning, context compaction, memory, tool execution policy, MCP/Skills/Plugins lifecycle, Provider transport internals, or untyped protocol passthrough to the browser |
| Profile Host / adapter | Narrow, typed bridge from platform resources to Codex app-server requests and notifications; process-instance isolation; request-id mapping; safe event normalization | Product UI behavior, broad protocol rewriting, model/tool discovery emulation, or persistent state that belongs to Codex Profile or platform tables |
| Codex app-server / Runtime | Thread/Turn lifecycle, model context, tools, MCP, Skills, Plugins, memory, multi-agent coordination, Provider model/transport behavior and generated protocol facts | Web sessions, organizations, browser DTOs, platform authorization, Git workspace provisioning, deployment scripts, or Profile ownership policy |
| Plugin / Skill / MCP package | Model-visible capability instructions and tool/server declarations consumed by Codex discovery | Web-side command interception, platform config mutation, or hidden Profile `config.toml` edits |

Planning and code review must reject these anti-patterns:

1. A browser command or composer shortcut that answers a Runtime capability
   question without sending the user's intent through Codex.
2. A server route or startup script that injects MCP/Skill/Plugin configuration
   directly into a Profile as a substitute for Codex discovery or a typed
   platform lifecycle API.
3. A Web/server prompt injection workaround for provider-neutral capabilities
   when the capability can be expressed by Runtime tools, Skills, Plugins, MCP
   or a generated app-server contract.
4. Browser exposure of raw request ids, raw JSON-RPC, local paths, credentials,
   unbounded protocol payloads or unvetted tool catalogs.
5. Product-specific changes spread through high-churn Codex files when the same
   behavior can live in `apps/web`, a plugin, a skill, an MCP server or a narrow
   generated protocol seam.

Every feature proposal must include a short boundary note naming the owner,
inputs, outputs, capability gate and tests. If the owner is Codex, follow the
upstream customization workflow before editing. If the owner is Web/platform,
prove that the implementation consumes typed contracts rather than recreating
Runtime behavior.

## Multi-user isolation model

The authorization chain is:

```text
session -> user -> organization membership -> project permission
        -> profile/workspace grant
        -> task/thread -> run/event/approval
        -> durable artifact grant + producer provenance
```

- One member has one persistent personal Profile by default.
- A Profile has a dedicated `CODEX_HOME`, credentials, Provider configuration,
  Threads, memory, skills, plugins and MCP configuration.
- One Profile has at most one primary app-server process. Cross-process locking
  and a process registry enforce the invariant.
- A Profile may execute multiple authorized Tasks only within measured Runtime
  concurrency limits. It never shares a Home with another user.
- A Workspace is an independently authorized execution root. It may be an
  operator-registered existing directory or an explicitly created managed
  clone/worktree. It is never implicitly owned by a Thread, Task or Run.
- Codex owns each Thread's current `cwd` and supports changing it through its
  official Thread/Turn contracts. Multiple Threads may use the same authorized
  Workspace; starting, resuming or running a Thread does not create a checkout.
- Profile Host validates that every Runtime `cwd` is contained by a Workspace
  authorized for the Profile/user. Runner revalidates the Workspace grant for
  Git and delivery operations. Normal browser users never submit trusted
  filesystem paths.
- Cache, subscription, model and secret keys include their user/Profile scope.
  Cross-user and guessed-ID denial tests are release gates.

### Single-Profile convergence mode

The current near-term runtime target is a deliberately narrowed deployment mode:
one implicit local Owner, one persistent Profile Home, one primary Profile
Host process and a fixed set of authorized Workspace roots from which each
Thread's Codex `cwd` is selected. This is a deployment constraint, not a
boundary exception. The same ownership table above continues to apply:

- The platform starts and monitors the single Profile Host, injects only
  authorized environment and secret references, and records safe diagnostics.
- The selected `CODEX_HOME`, Profile identity, authorized Workspace roots,
  Runner/source roots and capability roots are fixed at startup or by typed
  platform lifecycle state; browser input never changes server-local paths.
- To unblock the single Profile smoke, the platform may copy a file-backed
  `auth.json` from an already logged-in local Codex home into an empty Profile
  home before starting the Profile Host. This is a transitional single-user
  import path only; it must not become the multi-user credential model.
- Skills, Plugins and MCP are still discovered and executed by Codex Runtime.
  The WebApp does not scan `.mcp.json`, run plugin launchers, answer MCP
  inventory questions locally or write hidden Profile configuration.
- MCP startup notifications are persisted as the browser's lightweight status
  projection. Thread hydration reads that projection and never calls the full
  Runtime MCP inventory path, whose tools and resources are unrelated to the
  sidebar status surface.
- The Server ensures the implicit local Owner on startup, and the browser
  obtains a local Session without rendering login or registration. Session,
  Organization, Profile and resource authorization remain the internal request
  context; this transition mode is not a public or multi-user authentication
  design.
- Local capability packages such as `tools/maps-mcp` are made available as
  selected capability roots. Their launchers own package bootstrap, dependency
  checks and MCP server startup; Profile Host only reports safe startup status
  and categorized failures.
- This mode must pass single Profile smoke tests for Provider login/model
  discovery, Runtime MCP discovery, MCP startup, third-party Provider tool calls,
  map-card rendering and Thread resume before multi-Profile routing work
  resumes.

## Runtime bridge

Codex produces a build-specific contract bundle containing:

1. JSON Schema and TypeScript definitions generated by app-server protocol.
2. A Capability Manifest derived from the build's method registry,
   experimental annotations, limits and build identity.
3. Protocol fixtures and stable structured error metadata.
4. Codex commit, target, binary digest and compatibility notes.

The Web build consumes the bundle by digest. A separate Web feature policy maps
product features to capability IDs and minimum versions; it cannot claim a
server supports a feature. A capability is enabled only when generated
contracts, offline fixtures and a real app-server smoke test agree.

Browser DTOs are stable platform resources, not passthrough JSON-RPC. Raw
app-server request IDs, Profile paths, local paths, credentials and unknown
protocol payloads remain inside the Host/adapter boundary. Unknown Runtime
events may be retained for diagnostics but cannot be exposed as an unsafe public
API or crash the event stream.


### Rich reply cards and map visualization

Structured reply cards are browser projections of Codex message content and
platform artifacts. Codex remains responsible for deciding when to use tools and
what to say. Skills, Plugins and MCP servers may provide model-visible
instructions or tools that return versioned `structuredContent`; the Web
platform may validate and render a supported contract, but it must not make the
model "discover" a capability by intercepting composer text or injecting ad-hoc
prompts.

The target map-card contract follows this flow. The checked-in Run/Thread
Artifact ownership is recorded as a migration gap in
`docs/capability-baseline.md` and `docs/development-plan.md`; it is not part of
the target architecture:

1. Geocoding and routing tools publish GeoJSON as standard MCP Resources. Their
   `outputSchema`-validated `data_ref` contains the raw MCP server ID and the same
   URI exposed by `resource_link.uri`. The complete reference is card-compatible;
   its server and URI are directly reusable by MCP `resources/read`. The raw
   server ID is distinct from the model-visible `mcp__server` Tool namespace.
   `map_utils.create_map_card` exposes one `map.v3` contract. GeoJSON `sources`
   are keyed by source ID; inline GeoJSON uses standard `source.data`, while a
   complete Resource reference uses the mutually exclusive Open Web
   `source.data_ref`. Standard GeoJSON source options are preserved. `layers`
   is official Mapbox Style Specification Layer JSON and is validated with the
   official validator rather than a second paint/layout/filter/expression
   whitelist. Official unknown-property diagnostics are warnings and invalid
   known syntax is rejected. Standard camera fields remain top-level. Optional
   Open Web behavior is isolated under `extensions.hover` and
   `extensions.legend`; neither is represented as a Mapbox layer field.
   `create_map_card`
   advertises an MCP `outputSchema` and returns a
   generic `open-web-artifact` / `inline-visualization.v1` envelope. Its first
   renderer kind is `map.v3`; the Tool also generates the complete
   `::codex-inline-vis{artifact="..."}` line. Resource sources copy a complete
   `data_ref` from an earlier completed Tool item available to the producing
   Runtime context.
   Tool `content` only tells the model to copy the embed line and is never a
   rendering input.
2. The Server recognizes the generic envelope without branching on MCP server
   or Tool names, dispatches `renderer.kind` through a renderer registry and
   validates the stable card envelope, source authorization graph, camera, and
   extension references without reimplementing Mapbox style semantics. It registers
   the Inline Visualization Artifact with a durable identity and an explicit
   organization/user or project authorization grant independent of the
   producing Run and Thread. The producing Turn and Tool Item are retained as
   provenance, not as an authorization or lifecycle boundary. Resource
   server/URI pairs resolve only to earlier completed Tool
   items, are loaded through official `mcpServer/resource/read`, and are replaced
   by authorized Artifact URLs before renderer payload persistence. Public Tool
   projection strips the payload and MCP URI. Registration runs in a savepoint,
   so a projection failure cannot suppress the underlying Tool terminal event.
3. Tool completion never displays a map. An Agent Message places the Tool-generated
   embed line between arbitrary Markdown segments. The Web parser accepts only
   standalone directives in Agent Messages, excludes fenced and indented code,
   and buffers incomplete streaming directives. `file="*.html"` retains the
   official local-HTML meaning; `artifact="..."` resolves an authorized typed
   renderer. The parser does not inspect Tool, Reasoning, Command or user text.
4. Live Agent Message completion receives the same safe renderer DTO used by
   authoritative history. Resolution uses the durable Artifact ref and current
   caller authorization, so later Runs and authorized Threads may reuse a
   completed Artifact without inheriting its producer's lifecycle. Producer
   Turn/Item identity only verifies provenance because
   `thread/turns/list` may synthesize `item-N` identities. The old Tool-attached `replyCard`, dual-write, old-history
   reconstruction, Assistant JSON scan and position fallback paths are absent.
5. The browser reads referenced GeoJSON from authenticated Artifact URLs and
   renders point, line and polygon layers with Mapbox GL in an explicit Mercator
   projection. Point layers support circle, square, diamond, triangle and pin
   shapes plus CORS-enabled HTTPS PNG/JPEG/WebP icons. Line and polygon borders
   support opacity, width, cap/join and dash arrays. Any geometry may declare a
   bounded list of GeoJSON properties for a text-only hover popup; the renderer
   creates DOM text nodes rather than accepting Tool-supplied HTML. Fit
   viewports run after map load and after the container receives its first real
   size; camera viewports preserve explicit center and zoom. Card chrome shows
   the user-authored summary and legend, not internal source/layer counts or
   viewport diagnostics. The browser reads the
   restricted public `pk.` token through the typed
   authenticated `/api/configuration/maps` resource. Without a token the map
   card remains visible and opens an in-card configuration dialog; authorized
   owners/admins save through the same resource and all visible cards update.
   The shared dialog selects the one active Mapbox or Google Maps provider for
   server-side `map_utils` tools; saving replaces the prior provider and key.
   `VITE_MAPBOX_ACCESS_TOKEN` remains a build-time fallback.
   The retained Chat Completions transport still classifies text accompanying
   Tool calls as `commentary` and text-only completion as `final_answer`. This
   phase classification is compatibility behavior, not a
   Chat Completions wire guarantee. A Chat response does not identify ordinary
   `content` as reasoning, commentary, or final answer merely because it also
   contains Tool calls. This known gap can misclassify user-visible preambles;
   the proposed replacement preserves standard Chat text with unspecified phase,
   keeps Reasoning as a separate Item, and preserves first-appearance Item order.
   The target contract and staged work are defined in
   `docs/chat-responses-translation-spec.md` and
   `docs/chat-responses-translation-plan.md`.
6. The selected provider/key pair is one encrypted global entry in
   `platform_configuration_secrets`; the next save atomically replaces its
   value. The browser receives provider/configured status and, only while
   Mapbox is active, the restricted public `pk.` token required by Mapbox GL.
   The Server delivers the selected provider and key directly to a strictly
   validated local MCP elicitation URL without opening that one-time page.
   The global scope is temporary and reserves a later per-user move.
7. `map.v3` has no card-specific 16 KiB limit. Small GeoJSON can be inline;
   large GeoJSON stays outside the model/card payload and is loaded lazily from
   the Artifact cache. A general 128 MiB per-Resource memory-safety boundary is
   enforced by the Server; future larger formats require a streamed PMTiles or
   MVT source contract.
8. Invalid or unresolved data is not promoted into a browser card. Public
   ResourceLink projections remove source URIs and private metadata, while
   Artifact responses expose only authorized opaque URLs. Local paths,
   credentials, app-server request IDs and unbounded protocol payloads never
   reach the browser; ordinary Tool events may still show logical MCP
   server/tool names.

This design follows the official Codex inline-visualization directive already
implemented in the upstream TUI and the Apps SDK separation between data tools
and render tools. It does not broaden the Codex subtree: Chat translation
preserves text, official app-server Items remain unchanged, and Artifact
authorization/rendering stay in the Web platform. The implemented contract and
remaining Chat translation stages are defined in `docs/adr/005-map-reply-cards.md`,
`docs/chat-responses-translation-spec.md` and
`docs/chat-responses-translation-plan.md`.

## Primary runtime flows

### Create and run a Task

1. Platform authenticates the session and authorizes project/task creation.
2. A transaction creates the Task and queued Run using an idempotency key.
3. The user selects an authorized Workspace. A new managed clone/worktree, when
   needed, is created explicitly as an independent resource before the Thread.
   Scheduler leases the Run and validates that Workspace grant without creating
   a checkout.
4. Profile Host locks/starts the user's Profile, verifies contract
   compatibility and starts, resumes or updates the mapped Codex Thread with a
   `cwd` contained by the authorized Workspace.
5. Runtime events are normalized, assigned a per-Task monotonic sequence and
   persisted before browser fan-out. After the WebSocket is subscribed, the
   initial browser snapshot establishes each Task cursor at its latest durable
   sequence; only reconnect gaps are replayed. Authoritative Codex history
   hydrates the selected Thread, so a page refresh does not replay every old
   Item delta through the presentation tree.
6. Terminal state is reconciled across database, Codex Profile and Git. No Run
   remains `running` without a valid lease/heartbeat and recoverable owner.

### Approval or structured input

1. Each app-server process receives a fresh Runtime instance UUID. Profile Host
   receives a Codex Server Request and persists an internal mapping to
   Profile/Task/Run/Thread plus that instance before notification.
2. Platform filters recipients by resource permission and approval policy.
3. The first valid decision wins through compare-and-swap semantics.
4. Host responds only when both the process instance and request id still match.
   Active Turns and unresolved Server Requests block credential-triggered
   restart; after an actual restart, old-instance requests become cancelled and
   a reused numeric request id cannot receive the stale response.
5. An uncertain transport delivery remains retryable only with the same stored
   decision; expiry or Run termination produces an explicit terminal state.

### Provider model catalog refresh

1. The typed Provider service authorizes and persists a Provider-scoped model
   refresh or context-window edit through the app-server config contract.
2. Profile Registry marks the owned app-server process for replacement. An
   active Turn or unresolved Server Request continues on the current process
   and blocks replacement.
3. At the next safe Turn boundary, the adapter replaces the process under one
   serialized Runtime operation, invalidates process-local Thread bindings and
   resumes the same persisted Codex Thread before starting its next Turn.
4. The replacement Runtime rebuilds its startup-scoped model catalog from the
   Profile configuration. Context accounting and compaction remain Runtime
   behavior; the Server only owns the safe process lifecycle transition.
5. Opening an existing Thread is not a configuration boundary. The browser
   projects the Task's persisted Provider/model pair from the already loaded
   Profile catalog. New Threads opt into the official paginated history mode;
   Profile Host resumes an unloaded Thread with `excludeTurns`, then the adapter
   joins indexed `thread/turns/list(itemsView=notLoaded)` and
   `thread/items/list` streams by stable Turn id instead of invoking the
   app-server's serial full-item compatibility hydrator. Existing legacy
   rollout histories stay in one isolated compatibility branch until those
   Profile histories are retired. The browser reuses an unchanged completed
   Thread projection already loaded in the current session and invalidates it
   on background Runtime events. A Provider catalog cache miss may trigger a
   read-only background lookup, but Thread hydration never writes the global
   Profile selection, refreshes the Runtime catalog or waits for that lookup.

### Commit and push

Runner revalidates Workspace authorization, containment of the Thread's current
`cwd` and Git status immediately before the operation. Commit and Push are
explicit user actions with audit records. Force Push, implicit Merge and
automatic remote branch deletion are outside the product contract.

## Upstream synchronization boundary

`codex/` is a Git subtree tracking official `openai/codex/main`. Before touching
high-churn Runtime files, run `scripts/codex-upstream-status.sh`. Official
updates use `scripts/sync-codex-upstream.sh --apply` on a dedicated
`codex/sync-upstream-*` branch.

Prefer, in order:

1. consume an existing upstream app-server method;
2. add generated protocol/manifest metadata around upstream structure;
3. add the smallest isolated Runtime seam with scoped tests;
4. implement platform policy outside `codex/`.

Never fork Thread history, compaction, memory, multi-agent scheduling, Skills,
Plugins or MCP into the Web platform for short-term convenience.

## Current implementation boundary

The live capability and delivery status are intentionally not duplicated here.
Use `docs/capability-baseline.md` for verified Runtime/platform facts and
`docs/development-plan.md` for completed and next work. ADRs under `docs/adr/`
record accepted implementation choices without redefining these ownership
rules.
