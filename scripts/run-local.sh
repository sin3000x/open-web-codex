#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
web_root="$repo_root/apps/web"
runtime_root="$repo_root/codex/codex-rs"

action="foreground"
skip_build="${OPEN_WEB_CODEX_SKIP_BUILD:-0}"
codex_mode="${CODEX_MODE:-real}"
build_profile="${OPEN_WEB_CODEX_BUILD_PROFILE:-debug}"
bind_host="${OPEN_WEB_CODEX_BIND_HOST:-127.0.0.1}"
server_port="${OPEN_WEB_CODEX_SERVER_PORT:-4800}"
data_dir="${OPEN_WEB_CODEX_DATA_DIR:-$repo_root/.local/open-web-codex}"
database_url="${DATABASE_URL:-}"
database_url_file=""
database_max_connections="${DATABASE_MAX_CONNECTIONS:-10}"

run_dir="$data_dir/run"
log_dir="$data_dir/logs"
pid_file="$run_dir/server.pid"
server_log="$log_dir/server.log"
master_key_file="$data_dir/master-key"
profile_home="${CODEX_HOME:-$data_dir/profiles/default}"
runner_root="${OPEN_WEB_CODEX_RUNNER_ROOT:-$data_dir/runner}"
web_dist="$web_root/dist"
server_bin=""

usage() {
  cat <<'EOF'
Usage: ./scripts/run-local.sh [options]

Options:
  --background              Start the platform in the background.
  --stop                    Stop the platform recorded for the data directory.
  --status                  Show process and health status.
  --no-build                Reuse existing browser and Rust build outputs.
  --release                 Build and run optimized release binaries.
  --fake                    Use the deterministic in-memory Codex adapter.
  --bind HOST               Bind host (default: 127.0.0.1).
  --port PORT               HTTP/WebSocket port (default: 4800).
  --database-url URL        PostgreSQL connection URL.
  --database-url-file PATH  Read the PostgreSQL URL from a local file.
  --database-max-connections COUNT
                            PostgreSQL pool size (default: 10).
  -h, --help                Show this help.

Environment:
  CODEX_MODE                         real (default) or fake
  CODEX_BIN                          Codex CLI binary used in real mode
  CODEX_HOME                         Persistent Profile home
  OPEN_WEB_CODEX_IMPORT_CODEX_AUTH_FROM
                                     Single-Profile transition: import
                                     file-backed auth.json from this Codex home
                                     when CODEX_HOME has no auth.json
  OPEN_WEB_CODEX_MASTER_KEY          Base64-encoded 32-byte key; a local key is
                                     generated under the data directory if absent
  OPEN_WEB_CODEX_RUNNER_ROOT         Private mirror/workspace root
  OPEN_WEB_CODEX_DATA_DIR            Runtime data and logs directory
  OPEN_WEB_CODEX_MAPS_MCP_VENV       Shared maps MCP Python environment
                                     (default: $OPEN_WEB_CODEX_DATA_DIR/tool-envs/maps-mcp)
  OPEN_WEB_CODEX_SKIP_MAPS_MCP_SETUP 1 to skip startup preparation of maps MCP
  OPEN_WEB_CODEX_BIND_HOST           Bind host
  OPEN_WEB_CODEX_SERVER_PORT         HTTP/WebSocket port
  OPEN_WEB_CODEX_SKIP_BUILD          1 to reuse build outputs
  OPEN_WEB_CODEX_BUILD_PROFILE       debug (default) or release
  OPEN_WEB_CODEX_DISABLE_CODEX_SANDBOX
                                     1 to trust the surrounding container and
                                     avoid nested Codex bubblewrap sandboxing
  DATABASE_URL                       PostgreSQL connection URL
  DATABASE_MAX_CONNECTIONS           PostgreSQL pool size
EOF
}

error() {
  printf 'error: %s\n' "$*" >&2
}

while (($# > 0)); do
  case "$1" in
    --background) action="background" ;;
    --stop) action="stop" ;;
    --status) action="status" ;;
    --no-build) skip_build="1" ;;
    --release) build_profile="release" ;;
    --fake) codex_mode="fake" ;;
    --bind)
      (($# >= 2)) || { error "$1 requires a value"; exit 2; }
      bind_host="$2"
      shift
      ;;
    --port)
      (($# >= 2)) || { error "$1 requires a value"; exit 2; }
      server_port="$2"
      shift
      ;;
    --database-url)
      (($# >= 2)) || { error "$1 requires a value"; exit 2; }
      database_url="$2"
      shift
      ;;
    --database-url-file)
      (($# >= 2)) || { error "$1 requires a value"; exit 2; }
      database_url_file="$2"
      shift
      ;;
    --database-max-connections)
      (($# >= 2)) || { error "$1 requires a value"; exit 2; }
      database_max_connections="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      error "unknown option: $1"
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$skip_build" in 0|1) ;; *) error "OPEN_WEB_CODEX_SKIP_BUILD must be 0 or 1"; exit 2 ;; esac
case "$codex_mode" in real|fake) ;; *) error "CODEX_MODE must be real or fake"; exit 2 ;; esac
case "$build_profile" in debug|release) ;; *) error "OPEN_WEB_CODEX_BUILD_PROFILE must be debug or release"; exit 2 ;; esac
[[ "$server_port" =~ ^[1-9][0-9]*$ ]] || { error "port must be a positive integer"; exit 2; }
[[ "$database_max_connections" =~ ^[1-9][0-9]*$ ]] || { error "database pool size must be a positive integer"; exit 2; }

server_bin="$web_root/target/$build_profile/open-web-codex-server"
debug_server_bin="$web_root/target/debug/open-web-codex-server"
release_server_bin="$web_root/target/release/open-web-codex-server"

read_pid() {
  [[ -f "$pid_file" ]] && tr -d '[:space:]' <"$pid_file"
}

is_running() {
  local pid="${1:-}"
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null
}

is_server_running() {
  local pid="${1:-}" command
  is_running "$pid" || return 1
  command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command" == "$debug_server_bin" || "$command" == "$debug_server_bin "* \
    || "$command" == "$release_server_bin" || "$command" == "$release_server_bin "* ]]
}

health_ok() {
  command -v curl >/dev/null 2>&1 || return 1
  curl --silent --fail "http://$bind_host:$server_port/api/health" 2>/dev/null \
    | grep -Eq '"ok"[[:space:]]*:[[:space:]]*true'
}

show_status() {
  local pid
  pid="$(read_pid || true)"
  if is_server_running "$pid"; then
    printf 'server: running (PID %s)\n' "$pid"
  else
    printf 'server: stopped\n'
  fi
  if health_ok; then
    printf 'health: healthy\nweb:    http://%s:%s/\n' "$bind_host" "$server_port"
  else
    printf 'health: unavailable\n'
  fi
}

stop_server() {
  local pid attempt
  pid="$(read_pid || true)"
  if ! is_server_running "$pid"; then
    rm -f "$pid_file"
    printf 'open-web-codex is not running for %s\n' "$data_dir"
    return 0
  fi
  kill -TERM "$pid"
  for attempt in $(seq 1 100); do
    if ! is_server_running "$pid"; then
      rm -f "$pid_file"
      printf 'open-web-codex stopped.\n'
      return 0
    fi
    sleep 0.1
  done
  error "server PID $pid did not stop within 10 seconds"
  return 1
}

case "$action" in
  stop) stop_server; exit $? ;;
  status) show_status; exit 0 ;;
esac

if [[ -n "$database_url_file" ]]; then
  [[ -r "$database_url_file" ]] || { error "database URL file is not readable: $database_url_file"; exit 2; }
  IFS= read -r database_url <"$database_url_file" || true
fi
if [[ -z "$database_url" ]]; then
  database_user="${USER:-postgres}"
  database_url="postgresql://$database_user@127.0.0.1:5432/open_web_codex"
fi
case "$database_url" in postgres://*|postgresql://*) ;; *) error "database URL must use postgres:// or postgresql://"; exit 2 ;; esac

mkdir -p "$run_dir" "$log_dir" "$profile_home" "$runner_root"
maps_mcp_venv="${OPEN_WEB_CODEX_MAPS_MCP_VENV:-$data_dir/tool-envs/maps-mcp}"
if [[ "$codex_mode" == "real" && -z "${OPEN_WEB_CODEX_MASTER_KEY:-}" ]]; then
  if [[ ! -f "$master_key_file" ]]; then
    command -v openssl >/dev/null 2>&1 || { error "openssl is required to create the local Secret Store key"; exit 1; }
    umask 077
    openssl rand -base64 32 >"$master_key_file"
  fi
  OPEN_WEB_CODEX_MASTER_KEY="$(tr -d '\r\n' <"$master_key_file")"
  export OPEN_WEB_CODEX_MASTER_KEY
fi

codex_bin="${CODEX_BIN:-}"
using_repository_codex="0"
if [[ "$codex_mode" == "real" && -z "$codex_bin" ]]; then
  codex_bin="$runtime_root/target/$build_profile/codex"
  using_repository_codex="1"
fi
code_mode_host_bin="$runtime_root/target/$build_profile/codex-code-mode-host"

build_all() {
  command -v npm >/dev/null 2>&1 || { error "npm is required"; exit 1; }
  command -v cargo >/dev/null 2>&1 || { error "cargo is required"; exit 1; }
  if [[ ! -d "$web_root/node_modules" ]]; then
    (cd "$web_root" && npm ci)
  fi
  (cd "$web_root" && npm run build)
  if [[ "$build_profile" == "release" ]]; then
    (cd "$web_root" && CARGO_INCREMENTAL=0 cargo build --locked --release -p open-web-codex-server)
  else
    (cd "$web_root" && CARGO_INCREMENTAL=0 cargo build --locked -p open-web-codex-server)
  fi
  if [[ "$codex_mode" == "real" && "$using_repository_codex" == "1" ]]; then
    if [[ "$build_profile" == "release" ]]; then
      (cd "$runtime_root" && CARGO_INCREMENTAL=0 cargo build --locked --release -p codex-cli --bin codex -p codex-code-mode-host --bin codex-code-mode-host)
    else
      (cd "$runtime_root" && CARGO_INCREMENTAL=0 cargo build --locked -p codex-cli --bin codex -p codex-code-mode-host --bin codex-code-mode-host)
    fi
  fi
}

if [[ "$skip_build" == "0" ]]; then
  build_all
fi
[[ -x "$server_bin" ]] || { error "platform server is missing: $server_bin"; exit 1; }
[[ -f "$web_dist/index.html" ]] || { error "browser build is missing: $web_dist/index.html"; exit 1; }
if [[ "$codex_mode" == "real" ]]; then
  [[ -x "$codex_bin" ]] || { error "Codex binary is missing: $codex_bin"; exit 1; }
  if [[ "$using_repository_codex" == "1" ]]; then
    [[ -x "$code_mode_host_bin" ]] || { error "Codex code-mode host is missing: $code_mode_host_bin"; exit 1; }
    export CODEX_CODE_MODE_HOST_PATH="$code_mode_host_bin"
  fi
  if [[ "${OPEN_WEB_CODEX_SKIP_MAPS_MCP_SETUP:-0}" != "1" ]]; then
    OPEN_WEB_CODEX_MAPS_MCP_VENV="$maps_mcp_venv" \
      OPEN_WEB_CODEX_LOG_DIR="$log_dir" \
      "$script_dir/setup-maps-mcp-env.sh" >/dev/null
  fi
fi

server_command=(
  "$server_bin"
  --bind "$bind_host:$server_port"
  --database-max-connections "$database_max_connections"
  --codex-mode "$codex_mode"
  --runner-root "$runner_root"
  --web-dist "$web_dist"
)
if [[ "$codex_mode" == "real" ]]; then
  server_command+=(--codex-home "$profile_home" --codex-bin "$codex_bin")
fi

existing_pid="$(read_pid || true)"
if is_server_running "$existing_pid"; then
  error "open-web-codex is already running (PID $existing_pid)"
  exit 1
fi
rm -f "$pid_file"

export DATABASE_URL="$database_url"
export DATABASE_MAX_CONNECTIONS="$database_max_connections"
export CODEX_MODE="$codex_mode"
export OPEN_WEB_CODEX_RUNNER_ROOT="$runner_root"
export OPEN_WEB_CODEX_WEB_DIST="$web_dist"
if [[ "$codex_mode" == "real" ]]; then
  export CODEX_HOME="$profile_home"
  export CODEX_BIN="$codex_bin"
  export OPEN_WEB_CODEX_MAPS_MCP_VENV="$maps_mcp_venv"
  export MAPS_MCP_VENV="$maps_mcp_venv"
  export OPEN_WEB_CODEX_LOG_DIR="$log_dir"
else
  unset CODEX_HOME CODEX_BIN
fi

if [[ "$action" == "background" ]]; then
  nohup "${server_command[@]}" >"$server_log" 2>&1 </dev/null &
  server_pid=$!
  printf '%s\n' "$server_pid" >"$pid_file"
  for _ in $(seq 1 150); do
    if ! is_server_running "$server_pid"; then
      rm -f "$pid_file"
      error "open-web-codex exited during startup; inspect $server_log"
      exit 1
    fi
    if health_ok; then
      printf 'open-web-codex running in background (PID %s)\n' "$server_pid"
      printf 'web:  http://%s:%s/\nlogs: %s\n' "$bind_host" "$server_port" "$server_log"
      exit 0
    fi
    sleep 0.2
  done
  kill -TERM "$server_pid" 2>/dev/null || true
  rm -f "$pid_file"
  error "open-web-codex did not become healthy; inspect $server_log"
  exit 1
fi

cleanup() {
  local recorded
  recorded="$(read_pid || true)"
  if [[ "$recorded" == "$$" ]]; then
    rm -f "$pid_file"
  fi
}
trap cleanup EXIT
printf '%s\n' "$$" >"$pid_file"
printf 'Starting open-web-codex at http://%s:%s/ (%s Codex mode)\n' "$bind_host" "$server_port" "$codex_mode"
exec "${server_command[@]}"
