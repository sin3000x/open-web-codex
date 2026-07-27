# open-web-codex

`open-web-codex` is a self-hosted, browser-first Codex workbench. It combines the
Self-hosted browser workbench and platform host with a narrowly customized
Codex runtime in one repository.

The product keeps the responsibilities deliberately separate:

- `apps/web` owns users, projects, tasks, runs, permissions, worktrees, approvals,
  audit data, and the browser experience.
- `codex` owns model execution, Thread/Turn semantics, multi-agent behavior,
  memory, skills, plugins, MCP, and the app-server protocol.
- generated protocol artifacts are the integration boundary. The Web application
  must not reimplement Codex runtime behavior.

## Repository layout

```text
apps/web/                 Browser client and session-backed platform server
codex/                    Customized Codex runtime subtree
docs/product-design.md    Canonical product requirements and release scope
docs/capability-baseline.md
docs/development-plan.md  Executable milestone plan
docs/codex-upstream-sync.md
scripts/                  Monorepo and upstream-sync tooling
```

## Get started

For a single-host release deployment, make sure a PostgreSQL server is running,
then run:

```bash
./scripts/deploy.sh
```

The deployer installs exact Web dependencies, builds optimized browser,
platform Server and repository Codex artifacts, starts the Server in the
background, and verifies its health. Verbose build output stays in
`.local/open-web-codex/logs/deploy.log`; the terminal shows stage progress and a
service summary. Open `http://127.0.0.1:4800/web` after it succeeds.

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

For development, run the restored standalone WebApp with the deterministic
test Runtime:

```bash
./scripts/start-all.sh --fake
```

Then open `http://127.0.0.1:1421/web`. The current single-user WebApp creates an
implicit local Session and enters directly, without a login or registration
screen. It calls the platform Server on port `4800` through typed REST resources
and `/api/events/ws`; there is no separate Gateway process. Use
`./scripts/start-all.sh` for the repository Codex Runtime. See
[the MVP runbook](docs/mvp-runbook.md) for the browser flow, binary override and
known limitations.

Web application:

```bash
cd apps/web
npm ci
npm run typecheck
npm test
```

Codex runtime:

```bash
cd codex/codex-rs
just fmt --check
just test -p codex-app-server-protocol
```

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

- [Product design](docs/product-design.md)
- [Capability baseline](docs/capability-baseline.md)
- [Development plan](docs/development-plan.md)
- [Architecture](docs/architecture.md)

## Extension guides

- [Domain Agent extension architecture](docs/domain-agent-extension-architecture.md)
- [Skills, MCP, and custom UI extensions](docs/custom-skills-mcp-ui-guide.md)

The original component licenses remain in `apps/web/LICENSE` and `codex/LICENSE`.
See [LICENSES.md](LICENSES.md) for the repository licensing map.
