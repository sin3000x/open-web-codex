#!/bin/bash
# Browser-first replacement for the main-branch start-all topology.
# - `npm run dev` keeps the original 1420 default.
# - this script keeps the original standalone Web UI on 1421 at `/web`.
# - the authenticated platform Server runs on 4800.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DATA_DIR="${OPEN_WEB_CODEX_DATA_DIR:-$ROOT/.local/open-web-codex}"
RUN_DIR="$DATA_DIR/run"
LOG_DIR="$DATA_DIR/logs"
VITE_PID_FILE="$RUN_DIR/vite-1421.pid"
VITE_LOG="$LOG_DIR/vite-1421.log"
SERVER_PID_FILE="$RUN_DIR/server.pid"
CODEX_MODE_VALUE="${CODEX_MODE:-real}"

usage() {
  printf 'Usage: ./scripts/start-all.sh [--fake|--stop]\n'
}

mkdir -p "$RUN_DIR" "$LOG_DIR"

read_vite_pid() {
  [[ -f "$VITE_PID_FILE" ]] && tr -d '[:space:]' <"$VITE_PID_FILE"
}

vite_running() {
  local pid="${1:-}" command
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null || return 1
  command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command" == *"vite"* && "$command" == *"--port 1421"* ]]
}

server_running() {
  local pid="${1:-}" command
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null || return 1
  command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command" == *"open-web-codex-server"* ]]
}

health_ok() {
  curl --silent --fail http://127.0.0.1:4800/api/health 2>/dev/null \
    | grep -Eq '"ok"[[:space:]]*:[[:space:]]*true'
}

stop_all() {
  local pid
  pid="$(read_vite_pid || true)"
  if vite_running "$pid"; then
    kill -TERM "$pid"
  fi
  rm -f "$VITE_PID_FILE"
  "$ROOT/scripts/run-local.sh" --stop || true
}

case "${1:-}" in
  "") ;;
  --fake) CODEX_MODE_VALUE="fake" ;;
  --stop|stop)
    stop_all
    printf 'open-web-codex services stopped.\n'
    exit 0
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

case "$CODEX_MODE_VALUE" in
  real|fake) ;;
  *) printf 'error: CODEX_MODE must be real or fake.\n' >&2; exit 2 ;;
esac

stop_all

if [[ ! -d "$ROOT/apps/web/node_modules" ]]; then
  (cd "$ROOT/apps/web" && npm ci)
fi
if [[ ! -x "$ROOT/apps/web/target/debug/open-web-codex-server" ]]; then
  (cd "$ROOT/apps/web" && CARGO_INCREMENTAL=0 cargo build --locked -p open-web-codex-server)
fi

if [[ "$CODEX_MODE_VALUE" == "real" && -z "${CODEX_BIN:-}" ]]; then
  if [[ -x "$ROOT/codex/codex-rs/target/debug/codex" ]]; then
    export CODEX_BIN="$ROOT/codex/codex-rs/target/debug/codex"
  else
    printf 'error: the repository Codex binary is missing.\n' >&2
    printf 'Build it with: cd codex/codex-rs && CARGO_INCREMENTAL=0 cargo build -p codex-cli --bin codex -p codex-code-mode-host --bin codex-code-mode-host\n' >&2
    printf 'For a Server/UI smoke test, use: ./scripts/start-all.sh --fake\n' >&2
    exit 1
  fi
fi

RUN_LOCAL_ARGS=(--background --no-build)
if [[ "$CODEX_MODE_VALUE" == "fake" ]]; then
  RUN_LOCAL_ARGS+=(--fake)
fi
"$ROOT/scripts/run-local.sh" "${RUN_LOCAL_ARGS[@]}"

(
  cd "$ROOT/apps/web"
  OPEN_WEB_CODEX_FRONTEND_PORT=1421 \
    OPEN_WEB_CODEX_FRONTEND_HOST=0.0.0.0 \
    nohup npx vite --port 1421 --host 0.0.0.0 >"$VITE_LOG" 2>&1 &
  printf '%s\n' "$!" >"$VITE_PID_FILE"
)

for _ in $(seq 1 150); do
  vite_pid="$(read_vite_pid || true)"
  server_pid=""
  [[ -f "$SERVER_PID_FILE" ]] && server_pid="$(tr -d '[:space:]' <"$SERVER_PID_FILE")"
  if vite_running "$vite_pid" \
    && server_running "$server_pid" \
    && health_ok \
    && curl --silent --fail http://127.0.0.1:1421/web >/dev/null 2>&1; then
    printf 'Web UI:  http://127.0.0.1:1421/web\n'
    printf 'Server:  http://127.0.0.1:4800\n'
    printf '1420 remains the default port for: cd apps/web && npm run dev\n'
    exit 0
  fi
  sleep 0.2
done

printf 'error: services did not become healthy; inspect %s and %s\n' \
  "$VITE_LOG" "$LOG_DIR/server.log" >&2
stop_all
exit 1
