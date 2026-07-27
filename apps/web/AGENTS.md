# Web platform agent guide

The repository root `AGENTS.md` and its canonical documents are authoritative.
This file adds implementation rules for `apps/web/**`.

## Ownership

- `src/WebApp.tsx` and its component tree own the restored 1421 browser
  presentation. `src/services/webClient.ts` is its narrow compatibility seam.
- `browser/client.ts` owns the authenticated typed REST/WebSocket transport.
  Browser adapters may translate existing UI method shapes, but may not create
  a second Gateway protocol or expose raw Runtime payloads.
- `server/**` owns HTTP/WebSocket composition, authentication, authorization,
  browser DTO mapping, Profile/Runner wiring, and static asset delivery.
- `crates/profile-host` owns persistent Profile process lifecycle and the
  native app-server JSONL connection.
- `crates/run-orchestrator` and `crates/git-runtime` own Run leases, recovery,
  private mirrors, explicit managed Workspace lifecycle and Git operations.
  Workspaces are independent authorized execution roots; Threads select a
  `cwd`, and neither Threads nor Runs implicitly own checkouts.
- `crates/provider-service` owns Provider CRUD orchestration through typed
  app-server methods; secrets remain in `crates/secret-store`.
- `crates/platform-contracts` contains stable browser-facing DTOs. Generated
  Codex protocol types remain internal contract facts.

## Hard boundaries

- Do not add a desktop shell, sidecar daemon, loopback proxy, native window API,
  or platform-specific release pipeline.
- Do not expose raw app-server JSON-RPC, request IDs, filesystem paths,
  credentials, or configuration key paths to the browser.
- Browser subscriptions use `/api/events/ws`, authenticate in the first frame,
  and deliver tenant-filtered, durable platform projections.
- Browser requests never supply a trusted server-local path. Resolve Workspace
  IDs through authorized platform records, validate each Codex `cwd` against
  the resulting roots, and allow multiple Threads to use the same authorized
  Workspace.
- Do not duplicate Thread/Turn history, compaction, memory, skills, plugins,
  MCP, or multi-agent scheduling in the platform.
- Do not hand-edit generated Codex schemas or TypeScript. Regenerate them from
  the checked-in Runtime.

## Changes

1. Read `docs/capability-baseline.md` and `docs/development-plan.md` before
   changing behavior.
2. Put behavior in the owning crate and keep routes thin.
3. Add cross-tenant denial coverage for authorization-sensitive resources.
4. Persist a safe event/approval projection before broadcasting it.
5. For Runtime contract changes, regenerate the bundle and run the real
   app-server smoke.
6. Keep `npm run check:no-desktop` passing.

## Validation

For browser-only changes:

```bash
npm run lint
npm run typecheck
npm run test
npm run build
```

For platform Rust changes:

```bash
cargo fmt --all --check
cargo test --workspace --locked
```

For security or persistence changes, also run ignored PostgreSQL integration
tests with a disposable `TEST_DATABASE_URL`. Cross-project protocol changes
also require `npm run check:codex-contracts` and
`npm run smoke:codex-app-server -- --require-manifest`.
