# open-web-codex

`open-web-codex` is a self-hosted, browser-first Codex workbench. It combines the
Self-hosted browser workbench and platform host with a narrowly customized
Codex runtime in one repository.

The product keeps the responsibilities deliberately separate:

- `apps/web` owns users, projects, tasks, runs, permissions, independently
  authorized Workspaces, approvals, audit data, and the browser experience.
- `codex` owns model execution, Thread/Turn semantics, multi-agent behavior,
  memory, skills, plugins, MCP, and the app-server protocol.
- generated protocol artifacts are the integration boundary. The Web application
  must not reimplement Codex runtime behavior.

## Repository layout

```text
apps/web/                 Browser client and session-backed platform server
codex/                    Customized Codex runtime subtree
docs/README.md            Documentation authority and reading paths
docs/product-vision.md    Long-term product north star
docs/product-design.md    V1 product requirements and release scope
docs/architecture.md      Current technical architecture and ownership
docs/capability-baseline.md  Current verified capability evidence
docs/roadmap.md           Accepted medium-term stage order
docs/development-plan.md  Current and next milestone plan
docs/codex-upstream-sync.md
scripts/                  Monorepo and upstream-sync tooling
```

## Get started

For a single-host release deployment, make sure a PostgreSQL server is running,
then run:

```bash
./scripts/deploy.sh
```

The deployer validates PostgreSQL and delegates the optimized browser, platform
Server and repository Codex build plus health-checked replacement to the
canonical local runner. The Platform Server and Runtime components use exact
Cargo dependency fingerprints, so unchanged Release binaries are not rebuilt.
Deployment policy output stays in `.local/open-web-codex/logs/deploy.log`;
detailed build and startup output stays in
`.local/open-web-codex/logs/run-local.log`. Open `http://127.0.0.1:4800/web`
after it succeeds.

Runtime logs default to WARN, including errors. Set `RUST_LOG` explicitly to
override the filter, for example `RUST_LOG=info ./scripts/deploy.sh`.
Run recovery verifies the existing Runtime Thread before acquiring a new lease.
An explicit Runtime rejection of `thread/resume` fails the Run; transport errors
leave it pending recovery. Recovery never silently replaces a Thread.

When no database configuration exists, an interactive deploy asks whether to
use an existing PostgreSQL database or create the database and an application
user. The database name is always `open_web_codex`; passwords are read without
echo and the resulting URL is stored in
`.local/open-web-codex/database-url` with mode `600`. Non-interactive hosts must
provide `DATABASE_URL` or `--database-url-file` explicitly.

```bash
./scripts/deploy.sh --status
./scripts/deploy.sh --stop
```

Use `--database-url-file` for an externally managed PostgreSQL credential file and set
`OPEN_WEB_CODEX_MASTER_KEY` from a Secret Manager for a production host. Bind
to loopback behind an HTTPS reverse proxy instead of exposing port 4800
directly. This is the production-shaped single-host launcher; the remaining GA
security, backup and supervised-service gates are tracked in the development
plan.

For development, start the same-origin WebApp and deterministic test Runtime:

```bash
./scripts/run-local.sh --fake --background
```

Then open `http://127.0.0.1:4800/web`. The current single-user WebApp creates an
implicit local Session and enters directly, without a login or registration
screen. It calls the platform Server through same-origin typed REST resources
and `/api/events/ws`; there is no separate Gateway process. Omit `--fake` to use
the repository Codex Runtime. See
[the MVP runbook](docs/mvp-runbook.md) for the browser flow, binary override and
known limitations.

For frontend hot reload, keep the platform Server on port 4800 and run
`npm run dev` from `apps/web`; Vite uses port 1420 and proxies API and WebSocket
traffic to the platform Server.

Web application:

```bash
cd apps/web
npm ci
npm run typecheck
npm test
```

Rust validation uses separate, bounded test profiles:

```bash
(cd codex && just fmt-check)
./scripts/test-web-rust.sh
./scripts/test-codex.sh -p codex-app-server-protocol
```

Repository launch, deploy, and Rust test workflows use `sccache` when it is
installed. The local compiler cache defaults to an 8 GiB maximum. Cargo target
outputs are retained for normal incremental builds and are not subject to an
automatic repository storage watermark:

```bash
# macOS; on other platforms install a prebuilt sccache binary on PATH.
brew install sccache
make cargo-cache-status
```

Set `OPEN_WEB_CODEX_SCCACHE_MODE=required` to fail when sccache is unavailable,
and `SCCACHE_CACHE_SIZE` to change its hard cache limit. The wrappers disable
rustc incremental output because it cannot be cached by sccache.

Codex Cargo builds also use a separate checksum-verified V8 artifact cache. It
defaults to `~/Library/Caches/open-web-codex/codex-v8` on macOS and can be moved
with `OPEN_WEB_CODEX_V8_CACHE_DIR`. A valid cached archive and binding are used
without contacting GitHub; only a missing or failed-integrity cache entry invokes
the official one-time artifact resolver.

Inspect the official Codex upstream status:

```bash
./scripts/codex-upstream-status.sh
```

Create a dedicated sync branch and merge the latest `openai/codex` main branch
into the `codex/` subtree:

```bash
./scripts/sync-codex-upstream.sh --apply
```

Read [the upstream sync runbook](docs/codex-upstream-sync.md) before resolving a
non-trivial sync conflict.

## Canonical documents

- [Documentation map](docs/README.md)
- [Product vision](docs/product-vision.md)
- [Product design](docs/product-design.md)
- [Enterprise multi-Agent architecture](docs/enterprise-agent-platform-architecture.md)
- [Current architecture](docs/architecture.md)
- [Security model](docs/security-model.md)
- [Capability baseline](docs/capability-baseline.md)
- [Roadmap](docs/roadmap.md)
- [Development plan](docs/development-plan.md)

## Extension guides

- [Domain Agent extension architecture](docs/domain-agent-extension-architecture.md)
- [Skills, MCP, and custom UI extensions](docs/custom-skills-mcp-ui-guide.md)

The original component licenses remain in `apps/web/LICENSE` and `codex/LICENSE`.
See [LICENSES.md](LICENSES.md) for the repository licensing map.
