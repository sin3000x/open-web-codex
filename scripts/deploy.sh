#!/usr/bin/env bash

# Single-host production deployment entrypoint.
# Builds optimized artifacts, keeps verbose output in a bounded log, performs a
# health-checked rollout, and leaves the development-only Vite process stopped.

set -Eeuo pipefail
umask 077

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
web_root="$repo_root/apps/web"
runtime_root="$repo_root/codex/codex-rs"
run_local="$script_dir/run-local.sh"
start_all="$script_dir/start-all.sh"

action="deploy"
codex_mode="${CODEX_MODE:-real}"
bind_host="${OPEN_WEB_CODEX_BIND_HOST:-127.0.0.1}"
server_port="${OPEN_WEB_CODEX_SERVER_PORT:-4800}"
database_url="${DATABASE_URL:-}"
database_url_file=""
database_name="open_web_codex"
database_max_connections="${DATABASE_MAX_CONNECTIONS:-10}"
data_dir="${OPEN_WEB_CODEX_DATA_DIR:-$repo_root/.local/open-web-codex}"
public_url="${OPEN_WEB_CODEX_PUBLIC_URL:-}"
target_limit_gb="${OPEN_WEB_CODEX_TARGET_LIMIT_GB:-24}"
deploy_commit="$(git -C "$repo_root" rev-parse --short=12 HEAD 2>/dev/null || printf 'unknown')"
reuse_build="0"
bind_was_set="0"
port_was_set="0"
public_url_was_set="0"

usage() {
  cat <<'EOF'
Usage: ./scripts/deploy.sh [options]

Actions:
  (no action)               Build and deploy the real Codex service.
  --status                  Show the deployed service status.
  --stop                    Stop the deployed service.
  --check                   Validate prerequisites and database connectivity.

Options:
  --fake                    Deploy the deterministic fake Runtime.
  --reuse-build             Reuse existing release artifacts.
  --bind HOST               Bind host (default: 127.0.0.1).
  --port PORT               Server port (default: 4800).
  --public-url URL          Address displayed after deployment.
  --database-url URL        PostgreSQL connection URL.
  --database-url-file PATH  Read the PostgreSQL URL from a protected file.
  --database-max-connections COUNT
                            PostgreSQL pool size (default: 10).
  -h, --help                Show this help.

Environment:
  CODEX_BIN                         Compatible external Codex binary
  CODEX_HOME                        Persistent Profile home
  OPEN_WEB_CODEX_MASTER_KEY         Stable Base64-encoded 32-byte key
  OPEN_WEB_CODEX_DATA_DIR           Runtime state and log directory
  OPEN_WEB_CODEX_PUBLIC_URL         Reverse-proxy/public Web URL
  OPEN_WEB_CODEX_TARGET_LIMIT_GB    Target high-water mark (default: 24; 0 disables)
  DATABASE_URL                      PostgreSQL connection URL

If no database configuration exists, an interactive deployment asks whether
to use an existing PostgreSQL database or create one. The database name is
always open_web_codex. Generated credentials are stored with mode 600 under
the deployment data directory. Non-interactive deployments must provide a URL
or a readable URL file.

Verbose install and compilation output is written to:
  .local/open-web-codex/logs/deploy.log
EOF
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

while (($# > 0)); do
  case "$1" in
    --status) action="status" ;;
    --stop) action="stop" ;;
    --check) action="check" ;;
    --fake) codex_mode="fake" ;;
    --reuse-build) reuse_build="1" ;;
    --bind)
      (($# >= 2)) || fail "$1 requires a value"
      bind_host="$2"
      bind_was_set="1"
      shift
      ;;
    --port)
      (($# >= 2)) || fail "$1 requires a value"
      server_port="$2"
      port_was_set="1"
      shift
      ;;
    --public-url)
      (($# >= 2)) || fail "$1 requires a value"
      public_url="$2"
      public_url_was_set="1"
      shift
      ;;
    --database-url)
      (($# >= 2)) || fail "$1 requires a value"
      database_url="$2"
      shift
      ;;
    --database-url-file)
      (($# >= 2)) || fail "$1 requires a value"
      database_url_file="$2"
      shift
      ;;
    --database-max-connections)
      (($# >= 2)) || fail "$1 requires a value"
      database_max_connections="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
  shift
done

case "$codex_mode" in real|fake) ;; *) fail "CODEX_MODE must be real or fake" ;; esac
case "$reuse_build" in 0|1) ;; *) fail "reuse-build state is invalid" ;; esac
[[ "$server_port" =~ ^[1-9][0-9]*$ ]] || fail "port must be a positive integer"
[[ "$database_max_connections" =~ ^[1-9][0-9]*$ ]] || fail "database pool size must be a positive integer"
[[ "$target_limit_gb" =~ ^[0-9]+$ ]] || fail "OPEN_WEB_CODEX_TARGET_LIMIT_GB must be zero or a positive integer"
[[ "$bind_host" != *$'\n'* && "$public_url" != *$'\n'* ]] || fail "host and URL values must be single-line"

run_dir="$data_dir/run"
log_dir="$data_dir/logs"
deploy_log="$log_dir/deploy.log"
server_log="$log_dir/server.log"
pid_file="$run_dir/server.pid"
state_file="$run_dir/deploy-state"
managed_database_url_file="$data_dir/database-url"

mkdir -p "$data_dir" "$run_dir" "$log_dir"
chmod 700 "$data_dir" "$run_dir" "$log_dir" 2>/dev/null || true

state_value() {
  local key="$1"
  [[ -r "$state_file" ]] || return 0
  sed -n "s/^${key}=//p" "$state_file" | tail -n 1
}

if [[ "$action" == "status" || "$action" == "stop" ]]; then
  if [[ "$bind_was_set" == "0" ]]; then
    stored_bind="$(state_value BIND_HOST)"
    [[ -z "$stored_bind" ]] || bind_host="$stored_bind"
  fi
  if [[ "$port_was_set" == "0" ]]; then
    stored_port="$(state_value SERVER_PORT)"
    [[ -z "$stored_port" ]] || server_port="$stored_port"
  fi
  if [[ "$public_url_was_set" == "0" ]]; then
    stored_public_url="$(state_value PUBLIC_URL)"
    [[ -z "$stored_public_url" ]] || public_url="$stored_public_url"
  fi
  stored_codex_mode="$(state_value CODEX_MODE)"
  [[ -z "$stored_codex_mode" ]] || codex_mode="$stored_codex_mode"
  stored_deploy_commit="$(state_value DEPLOY_COMMIT)"
  [[ -z "$stored_deploy_commit" ]] || deploy_commit="$stored_deploy_commit"
fi

health_host="$bind_host"
case "$health_host" in 0.0.0.0|"::"|"[::]") health_host="127.0.0.1" ;; esac

default_public_url() {
  local normalized_host
  case "$bind_host" in
    0.0.0.0|"::"|"[::]") printf 'http://<server-address>:%s/web' "$server_port" ;;
    *:*)
      normalized_host="${bind_host#[}"
      normalized_host="${normalized_host%]}"
      printf 'http://[%s]:%s/web' "$normalized_host" "$server_port"
      ;;
    *) printf 'http://%s:%s/web' "$bind_host" "$server_port" ;;
  esac
}

if [[ -z "$public_url" ]]; then
  public_url="$(default_public_url)"
fi

box_width=76
box_rule() {
  local left="$1" fill="$2" right="$3" line="" i
  for ((i = 0; i < box_width; i++)); do line+="$fill"; done
  printf '%s%s%s\n' "$left" "$line" "$right"
}

box_line() {
  local value="$1" content_width=$((box_width - 2))
  if ((${#value} > content_width)); then
    value="${value:0:$((content_width - 3))}..."
  fi
  printf '│ %-*s │\n' "$content_width" "$value"
}

read_server_pid() {
  [[ -r "$pid_file" ]] && tr -d '[:space:]' <"$pid_file"
}

process_running() {
  local pid="${1:-}"
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null
}

server_process_running() {
  local pid="${1:-}" command release_server
  process_running "$pid" || return 1
  release_server="$web_root/target/release/open-web-codex-server"
  command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command" == "$release_server" || "$command" == "$release_server "* ]]
}

health_ok() {
  curl --silent --fail "http://$health_host:$server_port/api/health" 2>/dev/null \
    | grep -Eq '"ok"[[:space:]]*:[[:space:]]*true'
}

show_service_box() {
  local status="$1" pid="${2:---}"
  box_rule '╭' '─' '╮'
  box_line ' open-web-codex · Production Service'
  box_rule '├' '─' '┤'
  box_line " Status  : $status"
  box_line " Web     : $public_url"
  box_line " API     : http://$health_host:$server_port/api/health"
  box_line " Runtime : $codex_mode / release"
  box_line " Build   : $deploy_commit"
  box_line " Process : $pid"
  box_line " Logs    : $server_log"
  box_rule '╰' '─' '╯'
}

show_status() {
  local pid status
  pid="$(read_server_pid || true)"
  if server_process_running "$pid" && health_ok; then
    status='HEALTHY'
  elif server_process_running "$pid"; then
    status='STARTING OR UNHEALTHY'
  else
    status='STOPPED'
    pid='--'
  fi
  show_service_box "$status" "$pid"
  [[ "$status" == 'HEALTHY' ]]
}

if [[ "$action" == "status" ]]; then
  show_status
  exit $?
fi

if [[ "$action" == "stop" ]]; then
  OPEN_WEB_CODEX_DATA_DIR="$data_dir" "$run_local" --stop >/dev/null
  show_service_box 'STOPPED' '--'
  exit 0
fi

validate_database_url_value() {
  local value="$1" remainder path
  case "$value" in
    postgres://?*@*|postgresql://?*@*) ;;
    *)
      printf 'an explicit PostgreSQL URL must include a user before @\n'
      return 1
      ;;
  esac
  remainder="${value#*://}"
  [[ "$remainder" == */* ]] || {
    printf 'the PostgreSQL URL must name the %s database\n' "$database_name"
    return 1
  }
  path="${remainder#*/}"
  path="${path%%\?*}"
  path="${path%%#*}"
  [[ "$path" == "$database_name" ]] || {
    printf 'the PostgreSQL database name must be %s\n' "$database_name"
    return 1
  }
}

read_configured_database_url() {
  if [[ -n "$database_url_file" ]]; then
    IFS= read -r REPLY <"$database_url_file" || true
  else
    REPLY="$database_url"
  fi
}

percent_decode() {
  local input="$1" output="" prefix hex character
  while [[ "$input" == *%??* ]]; do
    prefix="${input%%\%??*}"
    output+="$prefix"
    input="${input:${#prefix}}"
    hex="${input:1:2}"
    [[ "$hex" =~ ^[0-9A-Fa-f]{2}$ ]] || return 1
    printf -v character '%b' "\\x$hex"
    output+="$character"
    input="${input:3}"
  done
  REPLY="$output$input"
}

parse_database_url() {
  local value="$1" connection authority userinfo host_port encoded_user encoded_password remainder query parameter
  validate_database_url_value "$value" || return 1
  connection="${value#*://}"
  authority="${connection%%/*}"
  userinfo="${authority%@*}"
  host_port="${authority##*@}"
  encoded_user="${userinfo%%:*}"
  if [[ "$userinfo" == *:* ]]; then encoded_password="${userinfo#*:}"; else encoded_password=""; fi
  percent_decode "$encoded_user" || return 1
  PARSED_DB_USER="$REPLY"
  percent_decode "$encoded_password" || return 1
  PARSED_DB_PASSWORD="$REPLY"

  if [[ "$host_port" == \[*\]* ]]; then
    remainder="${host_port#\[}"
    PARSED_DB_HOST="${remainder%%\]*}"
    remainder="${remainder#*\]}"
    if [[ "$remainder" == :* ]]; then PARSED_DB_PORT="${remainder#:}"; else PARSED_DB_PORT="5432"; fi
  else
    if [[ "$host_port" == *:* ]]; then
      PARSED_DB_HOST="${host_port%:*}"
      PARSED_DB_PORT="${host_port##*:}"
    else
      PARSED_DB_HOST="$host_port"
      PARSED_DB_PORT="5432"
    fi
  fi
  [[ -n "$PARSED_DB_USER" && -n "$PARSED_DB_HOST" && "$PARSED_DB_PORT" =~ ^[1-9][0-9]*$ ]] || return 1

  PARSED_DB_SSLMODE="prefer"
  if [[ "$value" == *\?* ]]; then
    query="${value#*\?}"
    query="${query%%#*}"
    while [[ -n "$query" ]]; do
      parameter="${query%%&*}"
      if [[ "$parameter" == sslmode=* ]]; then PARSED_DB_SSLMODE="${parameter#sslmode=}"; fi
      if [[ "$query" == *"&"* ]]; then query="${query#*&}"; else query=""; fi
    done
  fi
  case "$PARSED_DB_SSLMODE" in disable|allow|prefer|require|verify-ca|verify-full) ;; *) return 1 ;; esac
}

verify_database_url_connection() {
  local value="$1" current_database
  parse_database_url "$value" || return 1
  current_database="$(
    PGAPPNAME=open-web-codex-deploy \
    PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-5}" \
    PGHOST="$PARSED_DB_HOST" \
    PGPORT="$PARSED_DB_PORT" \
    PGUSER="$PARSED_DB_USER" \
    PGPASSWORD="$PARSED_DB_PASSWORD" \
    PGDATABASE="$database_name" \
    PGSSLMODE="$PARSED_DB_SSLMODE" \
      psql --no-psqlrc --quiet --tuples-only --no-align \
        --command 'SELECT current_database()'
  )" || return 1
  PARSED_DB_PASSWORD=''
  [[ "$current_database" == "$database_name" ]]
}

prompt_value() {
  local label="$1" default_value="$2" input
  printf '%s [%s]: ' "$label" "$default_value" >/dev/tty
  IFS= read -r input </dev/tty
  REPLY="${input:-$default_value}"
}

prompt_secret() {
  local label="$1"
  printf '%s: ' "$label" >/dev/tty
  IFS= read -r -s REPLY </dev/tty
  printf '\n' >/dev/tty
}

percent_encode() {
  local input="$1" output="" character hex index
  local LC_ALL=C
  for ((index = 0; index < ${#input}; index++)); do
    character="${input:index:1}"
    case "$character" in
      [a-zA-Z0-9.~_-]) output+="$character" ;;
      *)
        printf -v hex '%02X' "'$character"
        output+="%$hex"
        ;;
    esac
  done
  REPLY="$output"
}

build_database_url() {
  local host="$1" port="$2" username="$3" password="$4" encoded_user encoded_password url_host ssl_mode
  percent_encode "$username"
  encoded_user="$REPLY"
  percent_encode "$password"
  encoded_password="$REPLY"
  url_host="$host"
  if [[ "$url_host" == *:* && "$url_host" != \[*\] ]]; then
    url_host="[$url_host]"
  fi
  ssl_mode="${PGSSLMODE:-prefer}"
  case "$ssl_mode" in disable|allow|prefer|require|verify-ca|verify-full) ;; *) ssl_mode="prefer" ;; esac
  REPLY="postgresql://$encoded_user:$encoded_password@$url_host:$port/$database_name?sslmode=$ssl_mode"
}

persist_database_url() {
  local value="$1" temporary="$managed_database_url_file.tmp"
  printf '%s\n' "$value" >"$temporary"
  chmod 600 "$temporary"
  mv "$temporary" "$managed_database_url_file"
  database_url=""
  database_url_file="$managed_database_url_file"
}

validate_database_url_file_permissions() {
  local path="$1" mode
  [[ -f "$path" && -r "$path" ]] || return 1
  mode="$(stat -f '%Lp' "$path" 2>/dev/null || stat -c '%a' "$path" 2>/dev/null || true)"
  [[ "$mode" =~ ^[0-7]{3,4}$ ]] || return 1
  (( (8#$mode & 077) == 0 ))
}

validate_database_endpoint() {
  local host="$1" port="$2" username="$3"
  [[ -n "$host" && "$host" != *[[:space:]]* ]] || fail 'PostgreSQL host is invalid'
  [[ "$port" =~ ^[1-9][0-9]*$ ]] || fail 'PostgreSQL port must be a positive integer'
  [[ -n "$username" ]] || fail 'PostgreSQL username cannot be empty'
}

configure_existing_database() {
  local db_host db_port db_user db_password configured_url
  prompt_value 'PostgreSQL host' '127.0.0.1'
  db_host="$REPLY"
  prompt_value 'PostgreSQL port' '5432'
  db_port="$REPLY"
  prompt_value 'PostgreSQL username' "${USER:-open_web_codex}"
  db_user="$REPLY"
  prompt_secret 'PostgreSQL password'
  db_password="$REPLY"
  [[ -n "$db_password" ]] || fail 'PostgreSQL password cannot be empty'
  validate_database_endpoint "$db_host" "$db_port" "$db_user"
  build_database_url "$db_host" "$db_port" "$db_user" "$db_password"
  configured_url="$REPLY"
  printf 'Checking existing %s database...\n' "$database_name" >/dev/tty
  verify_database_url_connection "$configured_url" || fail "cannot connect to the existing $database_name database"
  persist_database_url "$configured_url"
  db_password=''
  configured_url=''
}

create_database() {
  local db_host db_port admin_user admin_password app_user app_password password_confirmation configured_url ssl_mode
  prompt_value 'PostgreSQL host' '127.0.0.1'
  db_host="$REPLY"
  prompt_value 'PostgreSQL port' '5432'
  db_port="$REPLY"
  prompt_value 'PostgreSQL administrator' "${USER:-postgres}"
  admin_user="$REPLY"
  prompt_secret 'Administrator password (empty for local peer/trust authentication)'
  admin_password="$REPLY"
  prompt_value 'Application username' 'open_web_codex'
  app_user="$REPLY"
  prompt_secret 'Application password'
  app_password="$REPLY"
  [[ -n "$app_password" ]] || fail 'application password cannot be empty'
  prompt_secret 'Confirm application password'
  password_confirmation="$REPLY"
  [[ "$app_password" == "$password_confirmation" ]] || fail 'application passwords do not match'
  validate_database_endpoint "$db_host" "$db_port" "$admin_user"
  validate_database_endpoint "$db_host" "$db_port" "$app_user"
  ssl_mode="${PGSSLMODE:-prefer}"
  case "$ssl_mode" in disable|allow|prefer|require|verify-ca|verify-full) ;; *) ssl_mode="prefer" ;; esac

  printf 'Creating or validating %s...\n' "$database_name" >/dev/tty
  PGAPPNAME=open-web-codex-deploy \
  PGCONNECT_TIMEOUT="${PGCONNECT_TIMEOUT:-5}" \
  PGHOST="$db_host" \
  PGPORT="$db_port" \
  PGUSER="$admin_user" \
  PGPASSWORD="$admin_password" \
  PGDATABASE=postgres \
  PGSSLMODE="$ssl_mode" \
  OPEN_WEB_CODEX_APP_USER="$app_user" \
  OPEN_WEB_CODEX_APP_PASSWORD="$app_password" \
    psql --no-psqlrc --quiet --set ON_ERROR_STOP=1 >/dev/null <<'SQL'
\getenv app_user OPEN_WEB_CODEX_APP_USER
\getenv app_password OPEN_WEB_CODEX_APP_PASSWORD
SELECT format('CREATE ROLE %I LOGIN PASSWORD %L', :'app_user', :'app_password')
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_user');
\gexec
SELECT format('CREATE DATABASE %I OWNER %I', 'open_web_codex', :'app_user')
WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = 'open_web_codex');
\gexec
SQL

  build_database_url "$db_host" "$db_port" "$app_user" "$app_password"
  configured_url="$REPLY"
  verify_database_url_connection "$configured_url" || fail "created database cannot be accessed as $app_user"
  persist_database_url "$configured_url"
  admin_password=''
  app_password=''
  password_confirmation=''
  configured_url=''
}

prompt_database_configuration() {
  local choice
  printf '\nPostgreSQL is not configured. Database name: %s\n' "$database_name" >/dev/tty
  printf '  1) Use an existing database (recommended for production)\n' >/dev/tty
  printf '  2) Create the database and an application user\n' >/dev/tty
  while true; do
    printf 'Select 1 or 2: ' >/dev/tty
    IFS= read -r choice </dev/tty
    case "$choice" in
      1) configure_existing_database; return 0 ;;
      2) create_database; return 0 ;;
      *) printf 'Please enter 1 or 2.\n' >/dev/tty ;;
    esac
  done
}

resolve_database_configuration() {
  local allow_prompt="$1"
  if [[ -n "$database_url" && -n "$database_url_file" ]]; then
    fail 'use only one of DATABASE_URL/--database-url and --database-url-file'
  fi
  if [[ -z "$database_url" && -z "$database_url_file" && -r "$managed_database_url_file" ]]; then
    database_url_file="$managed_database_url_file"
  fi
  if [[ -n "$database_url_file" ]]; then
    validate_database_url_file_permissions "$database_url_file" \
      || fail "database URL file must be a readable regular file with no group/other permissions: $database_url_file"
  fi
  if [[ -n "$database_url" || -n "$database_url_file" ]]; then
    read_configured_database_url
    validate_database_url_value "$REPLY" || fail 'invalid PostgreSQL configuration'
    REPLY=''
    return 0
  fi
  if [[ "$allow_prompt" == 'yes' && -t 0 && -r /dev/tty && -w /dev/tty ]]; then
    command -v psql >/dev/null 2>&1 || fail 'psql is required for interactive database setup'
    prompt_database_configuration
    return 0
  fi
  fail "PostgreSQL is not configured; provide --database-url-file or DATABASE_URL for $database_name"
}

verify_configured_database() {
  local configured_url
  read_configured_database_url
  configured_url="$REPLY"
  REPLY=''
  validate_database_url_value "$configured_url" || return 1
  verify_database_url_connection "$configured_url"
  configured_url=''
}

validate_prerequisites() {
  local command_name node_major
  for command_name in npm node cargo rustc git curl openssl psql; do
    command -v "$command_name" >/dev/null 2>&1 || {
      printf 'missing required command: %s\n' "$command_name"
      return 1
    }
  done
  node_major="$(node -p 'process.versions.node.split(".")[0]')"
  [[ "$node_major" =~ ^[0-9]+$ && "$node_major" -ge 20 ]] || {
    printf 'Node.js 20 or newer is required\n'
    return 1
  }
  [[ -f "$web_root/package-lock.json" ]] || return 1
  [[ -f "$web_root/Cargo.lock" ]] || return 1
  [[ -f "$runtime_root/Cargo.lock" ]] || return 1
  if [[ "$codex_mode" == "real" && -n "${CODEX_BIN:-}" ]]; then
    [[ -x "$CODEX_BIN" ]] || {
      printf 'CODEX_BIN is not executable\n'
      return 1
    }
  fi
}

if [[ "$action" == "check" ]]; then
  validate_prerequisites || fail 'deployment prerequisites failed'
  resolve_database_configuration no
  verify_configured_database || fail "cannot connect to the configured $database_name database"
  printf 'Deployment prerequisites and PostgreSQL: OK\n'
  exit 0
fi

: >"$deploy_log"
chmod 600 "$deploy_log"
printf 'open-web-codex deploy started at %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" >>"$deploy_log"

is_tty="0"
if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then is_tty="1"; fi
step_done=0
step_total=4
if [[ "$reuse_build" == "0" ]]; then
  step_total=$((step_total + 3))
  if [[ "$codex_mode" == "real" && -z "${CODEX_BIN:-}" ]]; then
    step_total=$((step_total + 1))
  fi
fi

render_progress() {
  local label="$1" width=28 filled empty i bar='' display_step
  display_step=$((step_done + 1))
  filled=$((display_step * width / step_total))
  empty=$((width - filled))
  for ((i = 0; i < filled; i++)); do bar+='#'; done
  for ((i = 0; i < empty; i++)); do bar+='-'; done
  printf '[%s] %2d/%-2d %-34s' "$bar" "$display_step" "$step_total" "$label"
}

show_failure_log() {
  printf '\nDeployment failed. Last log lines (%s):\n' "$deploy_log" >&2
  tail -n 40 "$deploy_log" >&2 || true
}

run_step() {
  local label="$1" started pid result elapsed spinner_index=0
  shift
  started="$(date +%s)"
  if [[ "$is_tty" == "1" ]]; then
    printf '\r'
    render_progress "$label"
  else
    render_progress "$label"
    printf ' ... '
  fi

  "$@" >>"$deploy_log" 2>&1 &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    if [[ "$is_tty" == "1" ]]; then
      case $((spinner_index % 4)) in 0) spinner='|' ;; 1) spinner='/' ;; 2) spinner='-' ;; *) spinner='\\' ;; esac
      printf ' %s' "$spinner"
      sleep 0.12
      printf '\b\b'
      spinner_index=$((spinner_index + 1))
    else
      sleep 0.2
    fi
  done
  if wait "$pid"; then result=0; else result=$?; fi
  elapsed=$(($(date +%s) - started))
  if [[ "$result" -ne 0 ]]; then
    if [[ "$is_tty" == "1" ]]; then printf '\n'; else printf 'FAILED\n'; fi
    show_failure_log
    exit "$result"
  fi
  if [[ "$is_tty" == "1" ]]; then
    printf '\r'
    render_progress "$label"
    printf ' OK (%ss)\n' "$elapsed"
  else
    printf 'OK (%ss)\n' "$elapsed"
  fi
  step_done=$((step_done + 1))
}

install_web_dependencies() {
  cd "$web_root"
  npm ci
}

build_browser() {
  cd "$web_root"
  npm run build
}

build_platform_server() {
  cd "$web_root"
  CARGO_INCREMENTAL=0 cargo build --locked --release -p open-web-codex-server
}

build_codex_runtime() {
  cd "$runtime_root"
  CARGO_INCREMENTAL=0 cargo build --locked --release \
    -p codex-cli --bin codex \
    -p codex-code-mode-host --bin codex-code-mode-host
}

target_size_kb() {
  local total=0 value path
  for path in "$web_root/target" "$runtime_root/target"; do
    if [[ -d "$path" ]]; then
      value="$(du -sk "$path" | awk '{print $1}')"
      total=$((total + value))
    fi
  done
  printf '%s\n' "$total"
}

enforce_target_high_watermark() {
  local limit_kb before_kb after_kb
  [[ "$target_limit_gb" != "0" ]] || return 0
  limit_kb=$((target_limit_gb * 1024 * 1024))
  before_kb="$(target_size_kb)"
  if ((before_kb <= limit_kb)); then
    printf 'Cargo targets: %s MiB (limit %s GiB)\n' "$((before_kb / 1024))" "$target_limit_gb"
    return 0
  fi

  printf 'Cargo targets exceeded %s GiB; pruning incremental-only caches\n' "$target_limit_gb"
  rm -rf \
    "$web_root/target/debug/incremental" \
    "$web_root/target/release/incremental" \
    "$runtime_root/target/debug/incremental" \
    "$runtime_root/target/release/incremental"
  after_kb="$(target_size_kb)"
  printf 'Cargo targets after incremental pruning: %s MiB\n' "$((after_kb / 1024))"
  if ((after_kb > limit_kb)); then
    printf 'warning: non-incremental Cargo artifacts still exceed the configured high-water mark\n'
  fi
}

rollout_service() {
  local -a args
  OPEN_WEB_CODEX_DATA_DIR="$data_dir" "$start_all" --stop || true
  args=(--release --background --no-build --bind "$bind_host" --port "$server_port" \
    --database-max-connections "$database_max_connections")
  if [[ "$codex_mode" == "fake" ]]; then args+=(--fake); fi
  if [[ -n "$database_url_file" ]]; then
    args+=(--database-url-file "$database_url_file")
  elif [[ -n "$database_url" ]]; then
    args+=(--database-url "$database_url")
  fi
  OPEN_WEB_CODEX_DATA_DIR="$data_dir" "$run_local" "${args[@]}"
}

verify_deployment() {
  local attempt pid
  for attempt in $(seq 1 100); do
    pid="$(read_server_pid || true)"
    if server_process_running "$pid" && health_ok; then return 0; fi
    sleep 0.2
  done
  return 1
}

write_deploy_state() {
  local temporary="$state_file.tmp"
  {
    printf 'BIND_HOST=%s\n' "$bind_host"
    printf 'SERVER_PORT=%s\n' "$server_port"
    printf 'PUBLIC_URL=%s\n' "$public_url"
    printf 'CODEX_MODE=%s\n' "$codex_mode"
    printf 'BUILD_PROFILE=release\n'
    printf 'DEPLOY_COMMIT=%s\n' "$deploy_commit"
  } >"$temporary"
  chmod 600 "$temporary"
  mv "$temporary" "$state_file"
}

run_step 'Validate prerequisites' validate_prerequisites
resolve_database_configuration yes
run_step 'Verify PostgreSQL database' verify_configured_database
if [[ "$reuse_build" == "0" ]]; then
  run_step 'Install exact Web dependencies' install_web_dependencies
  run_step 'Build browser application' build_browser
  run_step 'Build platform Server (release)' build_platform_server
  if [[ "$codex_mode" == "real" && -z "${CODEX_BIN:-}" ]]; then
    run_step 'Build Codex Runtime (release)' build_codex_runtime
  fi
fi
run_step 'Apply health-checked rollout' rollout_service
run_step 'Verify service health' verify_deployment

enforce_target_high_watermark >>"$deploy_log" 2>&1
write_deploy_state
printf 'open-web-codex deploy completed at %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" >>"$deploy_log"

printf '\n'
show_service_box 'HEALTHY' "$(read_server_pid)"
printf '\nDetailed build output: %s\n' "$deploy_log"
