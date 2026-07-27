.PHONY: mvp deploy deploy-status deploy-stop web-install web-check web-test contracts-check codex-upstream-status codex-upstream-sync

mvp:
	./scripts/start-all.sh

deploy:
	./scripts/deploy.sh

deploy-status:
	./scripts/deploy.sh --status

deploy-stop:
	./scripts/deploy.sh --stop

web-install:
	cd apps/web && npm ci

web-check:
	cd apps/web && npm run typecheck && npm run check:codex-contracts

web-test:
	cd apps/web && npm test

contracts-check:
	cd apps/web && npm run test:codex-capabilities && npm run test:codex-fixtures && npm run test:codex-harness

codex-upstream-status:
	./scripts/codex-upstream-status.sh

codex-upstream-sync:
	./scripts/sync-codex-upstream.sh --apply
