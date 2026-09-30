#!/usr/bin/env bash

set -euo pipefail
umask 077

export RUST_LOG="${RUST_LOG:-warn}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
web_root="$repo_root/apps/web"
runtime_root="$repo_root/codex/codex-rs"
cargo_cache_lib="$script_dir/cargo-build-cache.sh"
cargo_fingerprint_tool="$script_dir/cargo-dep-fingerprint.mjs"

# shellcheck source=scripts/cargo-build-cache.sh
source "$cargo_cache_lib"

action="foreground"
rebuild_development_database_requested="0"
skip_build="${OPEN_WEB_CODEX_SKIP_BUILD:-0}"
codex_mode="${CODEX_MODE:-real}"
build_profile="${OPEN_WEB_CODEX_BUILD_PROFILE:-debug}"
bind_host="${OPEN_WEB_CODEX_BIND_HOST:-127.0.0.1}"
server_port="${OPEN_WEB_CODEX_SERVER_PORT:-4800}"
data_dir="${OPEN_WEB_CODEX_DATA_DIR:-$repo_root/.local/open-web-codex}"
database_url="${DATABASE_URL:-}"
database_url_environment_set="${DATABASE_URL+x}"
database_url_file=""
database_url_option_set="0"
database_url_file_option_set="0"
default_database_url_file="$data_dir/database-url"
database_max_connections="${DATABASE_MAX_CONNECTIONS:-10}"

run_dir="$data_dir/run"
log_dir="$data_dir/logs"
pid_file="$run_dir/server.pid"
server_log="$log_dir/server.log"
launcher_log="$log_dir/run-local.log"
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
  --restart                 Build, then restart the background platform.
  --rebuild-development-database
                            Rebuild the default local development database
                            after the current build. It preserves encrypted
                            Provider credentials/models and the active maps
                            credential.
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
  CODEX_HOME                         Persistent Profile home
  OPEN_WEB_CODEX_IMPORT_CODEX_AUTH_FROM
                                     Single-Profile transition: import
                                     file-backed auth.json from this Codex home
                                     when CODEX_HOME has no auth.json
  OPEN_WEB_CODEX_MASTER_KEY          Base64-encoded 32-byte key; a local key is
                                     generated under the data directory if absent
  OPEN_WEB_CODEX_RUNNER_ROOT         Private mirror/workspace root
  OPEN_WEB_CODEX_DATA_DIR            Runtime data and logs directory
  OPEN_WEB_CODEX_BIND_HOST           Bind host
  OPEN_WEB_CODEX_SERVER_PORT         HTTP/WebSocket port
  OPEN_WEB_CODEX_SKIP_BUILD          1 to reuse build outputs
  OPEN_WEB_CODEX_BUILD_PROFILE       debug (default) or release
  OPEN_WEB_CODEX_SCCACHE_MODE        auto (default), required, or off
  SCCACHE_CACHE_SIZE                 Bounded compiler cache size (default: 8G)
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

is_tty="0"
if [[ -t 1 ]]; then
  is_tty="1"
fi
if [[ "$is_tty" == "1" && -z "${NO_COLOR:-}" ]]; then
  color_green=$'\033[32m'
  color_red=$'\033[31m'
  color_cyan=$'\033[36m'
  color_dim=$'\033[2m'
  color_reset=$'\033[0m'
else
  color_green=""
  color_red=""
  color_cyan=""
  color_dim=""
  color_reset=""
fi

show_launch_header() {
  printf '\n%sopen-web-codex · Local Runtime%s\n' "$color_cyan" "$color_reset"
  printf '  Mode: %s · Profile: %s · Port: %s\n\n' \
    "$codex_mode" "$build_profile" "$server_port"
}

show_step_skipped() {
  printf '  %s○%s %-30s %s%s%s\n' \
    "$color_dim" "$color_reset" "$1" "$color_dim" "$2" "$color_reset"
}

show_failure_log() {
  printf '\n%sLast local-runner log lines (%s):%s\n' \
    "$color_red" "$launcher_log" "$color_reset" >&2
  tail -n 40 "$launcher_log" >&2 || true
}

cargo_progress_width() {
  local width="${CARGO_TERM_PROGRESS_WIDTH:-}"
  if [[ "$width" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s' "$width"
  else
    printf '120'
  fi
}

run_step() {
  local stream_output="0"
  if [[ "${1:-}" == "--stream-output" ]]; then
    stream_output="1"
    shift
  fi
  local label="$1" started result elapsed
  local -a pipeline_status
  shift
  started="$SECONDS"
  if [[ "$stream_output" == "1" && "$is_tty" == "1" ]]; then
    printf '  %s→%s %-30s\n' "$color_cyan" "$color_reset" "$label"
    printf '     Cargo build progress:\n'
  elif [[ "$is_tty" == "1" ]]; then
    printf '  %s→%s %-30s' "$color_cyan" "$color_reset" "$label"
  else
    printf '  → %s\n' "$label"
  fi

  if [[ "$stream_output" == "1" && "$is_tty" == "1" ]]; then
    local progress_width
    progress_width="$(cargo_progress_width)"
    if CARGO_TERM_PROGRESS_WHEN=always \
      CARGO_TERM_PROGRESS_WIDTH="$progress_width" "$@" 2>&1 | tee -a "$launcher_log"; then
      result=0
    else
      pipeline_status=("${PIPESTATUS[@]}")
      result="${pipeline_status[0]}"
      if ((result == 0)); then
        result="${pipeline_status[1]}"
      fi
    fi
  elif "$@" >>"$launcher_log" 2>&1; then
    result=0
  else
    result=$?
  fi
  elapsed=$((SECONDS - started))
  if ((result == 0)); then
    if [[ "$is_tty" == "1" ]]; then
      if [[ "$stream_output" == "1" ]]; then
        printf '\n'
      else
        printf '\r'
      fi
      printf '  %s✓%s %-30s %s%ss%s\n' \
          "$color_green" "$color_reset" "$label" "$color_dim" "$elapsed" "$color_reset"
    else
      printf '  ✓ %-30s %ss\n' "$label" "$elapsed"
    fi
    return 0
  fi
  if [[ "$is_tty" == "1" ]]; then
    if [[ "$stream_output" == "1" ]]; then
      printf '\n'
    else
      printf '\r'
    fi
    printf '  %s✗%s %-30s failed\n' "$color_red" "$color_reset" "$label" >&2
  else
    printf '  ✗ %-30s failed\n' "$label" >&2
  fi
  show_failure_log
  return "$result"
}

run_progress_test() {
  local command_path="${OPEN_WEB_CODEX_RUN_LOCAL_TEST_COMMAND:-}"
  [[ -x "$command_path" ]] || {
    error "OPEN_WEB_CODEX_RUN_LOCAL_TEST_COMMAND must name an executable"
    return 2
  }
  is_tty="1"
  launcher_log="/dev/null"
  if [[ "${OPEN_WEB_CODEX_RUN_LOCAL_TEST_STREAM:-1}" == "1" ]]; then
    run_step --stream-output "progress environment test" "$command_path"
  else
    run_step "non-stream environment test" "$command_path"
  fi
}

while (($# > 0)); do
  case "$1" in
    --background)
      action="background"
      ;;
    --restart)
      action="restart"
      ;;
    --rebuild-development-database)
      rebuild_development_database_requested="1"
      ;;
    --stop)
      action="stop"
      ;;
    --status)
      action="status"
      ;;
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
      database_url_option_set="1"
      shift
      ;;
    --database-url-file)
      (($# >= 2)) || { error "$1 requires a value"; exit 2; }
      database_url_file="$2"
      database_url_file_option_set="1"
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

if [[ "$rebuild_development_database_requested" == "1" \
  && "$action" != "foreground" && "$action" != "background" ]]
then
  error "--rebuild-development-database can only be used when starting the service"
  exit 2
fi

if [[ "${OPEN_WEB_CODEX_RUN_LOCAL_TEST:-}" == "progress" ]]; then
  run_progress_test
  exit $?
fi

case "$skip_build" in 0|1) ;; *) error "OPEN_WEB_CODEX_SKIP_BUILD must be 0 or 1"; exit 2 ;; esac
case "$codex_mode" in real|fake) ;; *) error "CODEX_MODE must be real or fake"; exit 2 ;; esac
case "$build_profile" in debug|release) ;; *) error "OPEN_WEB_CODEX_BUILD_PROFILE must be debug or release"; exit 2 ;; esac
[[ "$server_port" =~ ^[1-9][0-9]*$ ]] || { error "port must be a positive integer"; exit 2; }
[[ "$database_max_connections" =~ ^[1-9][0-9]*$ ]] || { error "database pool size must be a positive integer"; exit 2; }

cargo_profile="dev-small"
cargo_profile_dir="dev-small"
if [[ "$build_profile" == "release" ]]; then
  cargo_profile="release"
  cargo_profile_dir="release"
fi

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  web_target_dir="$web_root/target"
  runtime_target_dir="$runtime_root/target"
elif [[ "$CARGO_TARGET_DIR" == /* ]]; then
  web_target_dir="$CARGO_TARGET_DIR"
  runtime_target_dir="$CARGO_TARGET_DIR"
else
  web_target_dir="$web_root/$CARGO_TARGET_DIR"
  runtime_target_dir="$runtime_root/$CARGO_TARGET_DIR"
fi

server_bin="$web_target_dir/$cargo_profile_dir/open-web-codex-server"
dev_server_bin="$web_target_dir/dev-small/open-web-codex-server"
release_server_bin="$web_target_dir/release/open-web-codex-server"
health_host="$bind_host"
case "$health_host" in
  0.0.0.0|"::"|"[::]") health_host="127.0.0.1" ;;
esac
health_url_host="$health_host"
if [[ "$health_url_host" == *:* && "$health_url_host" != \[*\] ]]; then
  health_url_host="[$health_url_host]"
fi
bind_address="$bind_host:$server_port"
if [[ "$bind_host" == *:* && "$bind_host" != \[*\] ]]; then
  bind_address="[$bind_host]:$server_port"
fi
health_url="http://$health_url_host:$server_port/api/health"
web_url="http://$health_url_host:$server_port/web"

validate_rebuild_development_database_authority() {
  local expected_data_dir="$repo_root/.local/open-web-codex"
  [[ -z "${OPEN_WEB_CODEX_DATA_DIR+x}" && "$data_dir" == "$expected_data_dir" ]] || {
    error "--rebuild-development-database only accepts the launcher default data directory: $expected_data_dir"
    return 2
  }
  [[ ! -L "$repo_root/.local" && ! -L "$data_dir" ]] || {
    error "--rebuild-development-database refuses symlinked launcher data directories"
    return 2
  }
  [[ -z "$database_url_environment_set" && "$database_url_option_set" == "0" \
    && "$database_url_file_option_set" == "0" \
    && ! -e "$default_database_url_file" && ! -L "$default_database_url_file" ]] || {
    error "--rebuild-development-database only rebuilds the default local PostgreSQL database"
    return 2
  }
  [[ "$codex_mode" == "real" ]] || {
    error "--rebuild-development-database requires the real Codex mode"
    return 2
  }
}

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
  [[ "$command" == "$dev_server_bin" || "$command" == "$dev_server_bin "* \
    || "$command" == "$release_server_bin" || "$command" == "$release_server_bin "* ]]
}

health_ok() {
  command -v curl >/dev/null 2>&1 || return 1
  curl --silent --fail --connect-timeout 1 --max-time 2 "$health_url" 2>/dev/null \
    | grep -Eq '"ok"[[:space:]]*:[[:space:]]*true'
}

box_width=76
box_rule() {
  local left="$1" fill="$2" right="$3" line="" i
  for ((i = 0; i < box_width; i++)); do
    line+="$fill"
  done
  printf '%s%s%s\n' "$left" "$line" "$right"
}

box_line() {
  local value="$1" content_width=$((box_width - 2))
  if ((${#value} > content_width)); then
    value="${value:0:$((content_width - 3))}..."
  fi
  printf '│ %-*s │\n' "$content_width" "$value"
}

show_service_box() {
  local status="$1" pid="${2:---}"
  printf '\n'
  box_rule '╭' '─' '╮'
  box_line ' open-web-codex · Local Service'
  box_rule '├' '─' '┤'
  box_line " Status  : $status"
  box_line " Web     : $web_url"
  box_line " API     : $health_url"
  box_line " Runtime : $codex_mode / $build_profile"
  box_line " Process : $pid"
  box_line " Logs    : $server_log"
  box_rule '╰' '─' '╯'
}

show_status() {
  local pid status
  pid="$(read_pid || true)"
  if is_server_running "$pid" && health_ok; then
    status="HEALTHY"
  elif is_server_running "$pid"; then
    status="STARTING OR UNHEALTHY"
  else
    status="STOPPED"
    pid="--"
  fi
  show_service_box "$status" "$pid"
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
  stop)
    stop_server >/dev/null
    show_service_box "STOPPED" "--"
    exit 0
    ;;
  status) show_status; exit 0 ;;
esac

if [[ "$rebuild_development_database_requested" == "1" ]]; then
  validate_rebuild_development_database_authority || exit $?
fi

existing_pid="$(read_pid || true)"
if [[ "$action" != "restart" ]] \
  && [[ "$rebuild_development_database_requested" == "0" ]] \
  && is_server_running "$existing_pid"
then
  error "open-web-codex is already running (PID $existing_pid); use --restart to rebuild and replace it"
  exit 1
fi
if ! is_server_running "$existing_pid"; then
  rm -f "$pid_file"
fi

if [[ -z "$database_url" && -z "$database_url_file" && -r "$default_database_url_file" ]]; then
  database_url_file="$default_database_url_file"
fi
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
copilots_root="$repo_root/copilots"
copilot_tool_registry_root="$repo_root/tools"
copilot_prepared_root="$data_dir/tool-environments"
copilot_build_store_root="$data_dir/tool-builds"
copilot_sdk_environment_root="$data_dir/sdk-environments/copilot"
copilot_sdk_bootstrap="$script_dir/copilot-sdk-bootstrap.sh"
copilot_sdk_python=""

if [[ "$codex_mode" == "real" && -z "${OPEN_WEB_CODEX_MASTER_KEY:-}" ]]; then
  if [[ ! -f "$master_key_file" ]]; then
    command -v openssl >/dev/null 2>&1 || { error "openssl is required to create the local Secret Store key"; exit 1; }
    umask 077
    openssl rand -base64 32 >"$master_key_file"
  fi
  OPEN_WEB_CODEX_MASTER_KEY="$(tr -d '\r\n' <"$master_key_file")"
  export OPEN_WEB_CODEX_MASTER_KEY
fi

if [[ "$codex_mode" == "real" && -n "${CODEX_BIN:-}" ]]; then
  error "run-local requires the Codex binary built from this checkout; CODEX_BIN is not supported"
  exit 2
fi

codex_bin=""
using_repository_codex="0"
if [[ "$codex_mode" == "real" ]]; then
  codex_bin="$runtime_target_dir/$cargo_profile_dir/codex"
  using_repository_codex="1"
fi
code_mode_host_bin="$runtime_target_dir/$cargo_profile_dir/codex-code-mode-host"
server_dep_info="$web_target_dir/$cargo_profile_dir/open-web-codex-server.d"
codex_dep_info="$runtime_target_dir/$cargo_profile_dir/codex.d"
code_mode_host_dep_info="$runtime_target_dir/$cargo_profile_dir/codex-code-mode-host.d"
server_stamp_dir="$data_dir/build-stamps/platform-server/$cargo_profile_dir"
runtime_stamp_dir="$data_dir/build-stamps/codex-runtime/$cargo_profile_dir"
server_stamp="$server_stamp_dir/open-web-codex-server.json"
codex_stamp="$runtime_stamp_dir/codex.json"
code_mode_host_stamp="$runtime_stamp_dir/codex-code-mode-host.json"
if [[ "$build_profile" == "release" ]]; then
  cargo_build_prefix=(cargo build --locked --release)
else
  cargo_build_prefix=(cargo build --locked --profile dev-small)
fi
server_build_args=("${cargo_build_prefix[@]}" -p open-web-codex-server)
codex_build_args=("${cargo_build_prefix[@]}" -p codex-cli --bin codex)
code_mode_host_build_args=(
  "${cargo_build_prefix[@]}"
  -p codex-code-mode-host
  --bin codex-code-mode-host
)
combined_runtime_build_args=(
  "${cargo_build_prefix[@]}"
  -p codex-cli
  --bin codex
  -p codex-code-mode-host
  --bin codex-code-mode-host
)
python_cmd="${PYTHON:-python3}"
codex_cargo_adapter=(
  "$python_cmd"
  "$repo_root/scripts/run-codex-cargo-with-v8.py"
)
printf -v server_build_command '%q ' "${server_build_args[@]}"
printf -v codex_build_command '%q ' "${codex_build_args[@]}"
printf -v code_mode_host_build_command '%q ' "${code_mode_host_build_args[@]}"

install_web_dependencies() {
  (cd "$web_root" && npm ci)
}

build_browser() {
  (cd "$web_root" && npm run build)
}

build_platform_server() {
  (cd "$web_root" && "${server_build_args[@]}")
}

build_codex_runtime() {
  (cd "$runtime_root" && "${codex_cargo_adapter[@]}" "${combined_runtime_build_args[@]}")
}

build_codex_cli() {
  (cd "$runtime_root" && "${codex_cargo_adapter[@]}" "${codex_build_args[@]}")
}

build_codex_code_mode_host() {
  (cd "$runtime_root" && "${codex_cargo_adapter[@]}" "${code_mode_host_build_args[@]}")
}

check_cargo_component_fingerprint() {
  local workspace="$1" component="$2" dep_info="$3" artifact="$4" stamp="$5"
  local build_command="$6"
  local output status
  if output="$(
    node "$cargo_fingerprint_tool" check \
      --workspace "$workspace" \
      --dep-info "$dep_info" \
      --artifact "$artifact" \
      --stamp "$stamp" \
      --component "$component" \
      --profile "$cargo_profile" \
      --build-command "$build_command" 2>&1
  )"; then
    printf '%s\n' "$output" >>"$launcher_log"
    return 0
  else
    status=$?
  fi
  printf '%s\n' "$output" >>"$launcher_log"
  if ((status == 1)); then
    return 1
  fi
  error "Cargo fingerprint check failed for $component"
  printf '%s\n' "$output" >&2
  return "$status"
}

record_cargo_component_fingerprint() {
  local workspace="$1" component="$2" dep_info="$3" artifact="$4" stamp="$5"
  local build_command="$6"
  node "$cargo_fingerprint_tool" record \
    --workspace "$workspace" \
    --dep-info "$dep_info" \
    --artifact "$artifact" \
    --stamp "$stamp" \
    --component "$component" \
    --profile "$cargo_profile" \
    --build-command "$build_command"
}

build_platform_server_and_record() {
  build_platform_server
  record_cargo_component_fingerprint \
    "$web_root" "platform-server" "$server_dep_info" "$server_bin" "$server_stamp" \
    "$server_build_command"
}

build_stale_platform_server() {
  if check_cargo_component_fingerprint \
    "$web_root" "platform-server" "$server_dep_info" "$server_bin" "$server_stamp" \
    "$server_build_command"
  then
    show_step_skipped "Platform server" "exact fingerprint matched"
  else
    run_step --stream-output "Platform server" build_platform_server_and_record
  fi
}

assert_reusable_outputs_current() {
  local stale_server stale_browser
  if [[ ! -x "$server_bin" || ! -f "$web_dist/index.html" ]]; then
    printf '{"code":"stale_build_output","message":"Current source or migration is newer than the reusable build output. Restart without --no-build."}\n' >&2
    exit 1
  fi
  stale_server="$(find \
    "$web_root/server" \
    "$web_root/crates" \
    "$web_root/migrations" \
    "$web_root/Cargo.toml" \
    "$web_root/Cargo.lock" \
    "$repo_root/capabilities/agents" \
    "$repo_root/capabilities/supervisors" \
    "$repo_root/capabilities/supervisor-instruction-policies" \
    "$repo_root/capabilities/plugins" \
    -type f -newer "$server_bin" -print -quit 2>/dev/null || true)"
  stale_browser="$(find \
    "$web_root/src" \
    "$web_root/browser" \
    "$web_root/package.json" \
    "$web_root/package-lock.json" \
    -type f -newer "$web_dist/index.html" -print -quit 2>/dev/null || true)"
  if [[ -n "$stale_server" || -n "$stale_browser" ]]; then
    printf '{"code":"stale_build_output","message":"Current source or migration is newer than the reusable build output. Restart without --no-build."}\n' >&2
    exit 1
  fi
}

build_both_codex_runtime_components() {
  build_codex_runtime
  record_cargo_component_fingerprint \
    "$runtime_root" "codex" "$codex_dep_info" "$codex_bin" "$codex_stamp" \
    "$codex_build_command"
  record_cargo_component_fingerprint \
    "$runtime_root" "codex-code-mode-host" "$code_mode_host_dep_info" \
    "$code_mode_host_bin" "$code_mode_host_stamp" "$code_mode_host_build_command"
}

build_codex_cli_and_record() {
  build_codex_cli
  record_cargo_component_fingerprint \
    "$runtime_root" "codex" "$codex_dep_info" "$codex_bin" "$codex_stamp" \
    "$codex_build_command"
}

build_codex_code_mode_host_and_record() {
  build_codex_code_mode_host
  record_cargo_component_fingerprint \
    "$runtime_root" "codex-code-mode-host" "$code_mode_host_dep_info" \
    "$code_mode_host_bin" "$code_mode_host_stamp" "$code_mode_host_build_command"
}

build_stale_codex_runtime_components() {
  local codex_fresh=0 code_mode_host_fresh=0
  if check_cargo_component_fingerprint \
    "$runtime_root" "codex" "$codex_dep_info" "$codex_bin" "$codex_stamp" \
    "$codex_build_command"
  then
    codex_fresh=1
  fi
  if check_cargo_component_fingerprint \
    "$runtime_root" "codex-code-mode-host" "$code_mode_host_dep_info" \
    "$code_mode_host_bin" "$code_mode_host_stamp" "$code_mode_host_build_command"
  then
    code_mode_host_fresh=1
  fi

  if ((codex_fresh == 1 && code_mode_host_fresh == 1)); then
    show_step_skipped "Codex Runtime" "exact fingerprints matched"
  elif ((codex_fresh == 0 && code_mode_host_fresh == 0)); then
    run_step --stream-output "Codex Runtime" build_both_codex_runtime_components
  elif ((codex_fresh == 0)); then
    run_step --stream-output "Codex CLI Runtime" build_codex_cli_and_record
    show_step_skipped "Codex code-mode host" "exact fingerprint matched"
  else
    show_step_skipped "Codex CLI Runtime" "exact fingerprint matched"
    run_step --stream-output "Codex code-mode host" build_codex_code_mode_host_and_record
  fi
}

bootstrap_copilot_sdk() {
  copilot_sdk_python="$(
    "$copilot_sdk_bootstrap" --environment-root "$copilot_sdk_environment_root"
  )"
  [[ "$copilot_sdk_python" == /* && -x "$copilot_sdk_python" ]] || {
    error "Copilot SDK bootstrap returned an invalid Python executable"
    return 1
  }
}

prepare_copilot_environment() {
  local package_root="$1" environment_root="$2"
  "$copilot_sdk_python" -m copilot_sdk prepare "$package_root" \
    --tool-registry-root "$copilot_tool_registry_root" \
    --output-root "$environment_root" \
    --build-store-root "$copilot_build_store_root" \
    --json
}

prepare_copilot_environments() {
  local manifest package_root package_id environment_root prepared_count=0
  local existing_id
  local -a active_package_ids
  while IFS= read -r -d '' manifest; do
    package_root="$(dirname "$manifest")"
    package_id="$(
      "$copilot_sdk_python" -c \
        'import pathlib, sys, tomllib; print(tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))["id"])' \
        "$manifest"
    )"
    [[ "$package_id" =~ ^[a-z0-9]([a-z0-9_-]{0,94}[a-z0-9])?$ ]] || {
      error "invalid Copilot package id in $manifest"
      return 1
    }
    environment_root="$copilot_prepared_root/$package_id"
    prepare_copilot_environment "$package_root" "$environment_root"
    [[ -f "$environment_root/copilot-sdk/prepared-tools.v1.json" ]] || {
      error "Copilot prepared descriptor is missing for $package_id"
      return 1
    }
    if ((prepared_count > 0)); then
      for existing_id in "${active_package_ids[@]}"; do
        if [[ "$existing_id" == "$package_id" ]]; then
          error "duplicate active Copilot package id: $package_id"
          return 1
        fi
      done
    fi
    active_package_ids[$prepared_count]="$package_id"
    prepared_count=$((prepared_count + 1))
  done < <(find "$copilots_root" -mindepth 2 -maxdepth 2 -type f -name copilot.toml -print0 | sort -z)
  ((prepared_count > 0)) || {
    error "no Copilot packages were discovered under $copilots_root"
    return 1
  }
  local -a gc_args=(
    -m copilot_sdk gc-builds
    --prepared-root "$copilot_prepared_root"
    --build-store-root "$copilot_build_store_root"
    --packages-root "$copilots_root"
  )
  for package_id in "${active_package_ids[@]}"; do
    gc_args+=(--active-package-id "$package_id")
  done
  gc_args+=(--json)
  "$copilot_sdk_python" "${gc_args[@]}"
}

rebuild_development_database() {
  DATABASE_URL="$database_url" \
    "$script_dir/rebuild-development-database.sh" \
      --confirm-development-only \
      --server-bin "$server_bin"
}

prepare_build_tools() {
  command -v npm >/dev/null 2>&1 || { error "npm is required"; exit 1; }
  command -v cargo >/dev/null 2>&1 || { error "cargo is required"; exit 1; }
  command -v "$python_cmd" >/dev/null 2>&1 || { error "$python_cmd is required"; exit 1; }
}

show_launch_header
: >"$launcher_log"
cargo_build_cache_configure "$repo_root"
cargo_build_cache_describe >>"$launcher_log"
if [[ "$skip_build" == "0" ]]; then
  prepare_build_tools
  if [[ ! -d "$web_root/node_modules" \
    || ! -f "$web_root/node_modules/.package-lock.json" \
    || "$web_root/package-lock.json" -nt "$web_root/node_modules/.package-lock.json" ]]
  then
    run_step "Browser dependencies" install_web_dependencies
  else
    show_step_skipped "Browser dependencies" "ready"
  fi
  run_step "Browser application" build_browser
  build_stale_platform_server
  if [[ "$codex_mode" == "real" && "$using_repository_codex" == "1" ]]; then
    build_stale_codex_runtime_components
  fi
else
  assert_reusable_outputs_current
  show_step_skipped "Build outputs" "reused (--no-build)"
fi
[[ -x "$server_bin" ]] || { error "platform server is missing: $server_bin"; exit 1; }
[[ -f "$web_dist/index.html" ]] || { error "browser build is missing: $web_dist/index.html"; exit 1; }
if [[ "$codex_mode" == "real" ]]; then
  [[ -x "$codex_bin" ]] || { error "Codex binary is missing: $codex_bin"; exit 1; }
  if [[ "$using_repository_codex" == "1" ]]; then
    [[ -x "$code_mode_host_bin" ]] || { error "Codex code-mode host is missing: $code_mode_host_bin"; exit 1; }
    export CODEX_CODE_MODE_HOST_PATH="$code_mode_host_bin"
  fi
  run_step "Copilot SDK" bootstrap_copilot_sdk
  run_step "Copilot environments" prepare_copilot_environments
fi

if [[ "$rebuild_development_database_requested" == "1" ]]; then
  run_step "Stop current service" stop_server
  run_step "Rebuild development database" rebuild_development_database
fi

server_command=(
  "$server_bin"
  --bind "$bind_address"
  --database-max-connections "$database_max_connections"
  --codex-mode "$codex_mode"
  --runner-root "$runner_root"
  --web-dist "$web_dist"
)
if [[ "$codex_mode" == "real" ]]; then
  server_command+=(
    --codex-home "$profile_home"
    --codex-bin "$codex_bin"
    --copilots-root "$copilots_root"
    --copilot-prepared-root "$copilot_prepared_root"
    --copilot-build-store-root "$copilot_build_store_root"
  )
fi

if [[ "$action" == "restart" ]]; then
  run_step "Stop current service" stop_server
  action="background"
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
else
  unset CODEX_HOME CODEX_BIN
fi

start_background_server() {
  local server_pid healthy_samples=0
  nohup "${server_command[@]}" >"$server_log" 2>&1 </dev/null &
  server_pid=$!
  printf '%s\n' "$server_pid" >"$pid_file"
  for _ in $(seq 1 150); do
    if ! is_server_running "$server_pid"; then
      rm -f "$pid_file"
      error "open-web-codex exited during startup; inspect $server_log"
      printf '\nServer log tail (%s):\n' "$server_log" >>"$launcher_log"
      tail -n 40 "$server_log" >>"$launcher_log" 2>&1 || true
      return 1
    fi
    if health_ok; then
      healthy_samples=$((healthy_samples + 1))
      if ((healthy_samples >= 3)); then
        return 0
      fi
    else
      healthy_samples=0
    fi
    sleep 0.2
  done
  kill -TERM "$server_pid" 2>/dev/null || true
  rm -f "$pid_file"
  printf '\nServer log tail (%s):\n' "$server_log" >>"$launcher_log"
  tail -n 40 "$server_log" >>"$launcher_log" 2>&1 || true
  return 1
}

if [[ "$action" == "background" ]]; then
  run_step "Platform service" start_background_server
  server_pid="$(read_pid)"
  show_service_box "HEALTHY" "$server_pid"
  printf '\n%sDetailed startup output: %s%s\n' \
    "$color_dim" "$launcher_log" "$color_reset"
  exit 0
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
show_service_box "FOREGROUND" "$$"
printf '\n%sDetailed startup output: %s%s\n\n' \
  "$color_dim" "$launcher_log" "$color_reset"
exec "${server_command[@]}"
