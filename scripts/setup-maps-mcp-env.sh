#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
tools_root="$repo_root/tools/maps-mcp"
data_dir="${OPEN_WEB_CODEX_DATA_DIR:-$repo_root/.local/open-web-codex}"
log_dir="${OPEN_WEB_CODEX_LOG_DIR:-$data_dir/logs}"
venv_dir="${OPEN_WEB_CODEX_MAPS_MCP_VENV:-${MAPS_MCP_VENV:-$data_dir/tool-envs/maps-mcp}}"
log_file="${OPEN_WEB_CODEX_MAPS_MCP_SETUP_LOG:-$log_dir/maps-mcp-env.log}"
python_cmd="${PYTHON:-python3}"

mkdir -p "$log_dir" "$(dirname "$venv_dir")"

log() {
  printf '[%s] %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*" | tee -a "$log_file" >&2
}

run_logged() {
  log "+ $*"
  "$@" >>"$log_file" 2>&1
}

check_imports() {
  "$venv_dir/bin/python" - <<'PY' >>"$log_file" 2>&1
try:
    import maps_mcp.server  # noqa: F401
    import mcp  # noqa: F401
except Exception as exc:
    print(f"maps-mcp import check failed: {type(exc).__name__}: {exc}")
    raise SystemExit(1)
print("maps-mcp imports ok")
PY
}

proxy_state() {
  local names=(HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY http_proxy https_proxy all_proxy no_proxy)
  local name value states=()
  for name in "${names[@]}"; do
    value="${!name:-}"
    if [[ -n "$value" ]]; then
      states+=("$name=set")
    else
      states+=("$name=unset")
    fi
  done
  (IFS=,; printf '%s' "${states[*]}")
}

log "maps-mcp environment setup starting"
log "repo_root=$repo_root"
log "tools_root=$tools_root"
log "venv_dir=$venv_dir"
log "python=$python_cmd"
log "proxy_state=$(proxy_state)"
log "uname=$(uname -a 2>/dev/null || true)"

command -v "$python_cmd" >/dev/null 2>&1 || {
  log "python command not found: $python_cmd"
  exit 127
}
run_logged "$python_cmd" --version
command -v node >/dev/null 2>&1 || {
  log "node is required for official Mapbox Style Spec validation"
  exit 127
}
command -v npm >/dev/null 2>&1 || {
  log "npm is required for official Mapbox Style Spec validation"
  exit 127
}
run_logged node --version
run_logged npm --version
if [[ "${OPEN_WEB_CODEX_REFRESH_MAPS_MCP_ENV:-0}" == "1" ]] \
  || ! node -e '
    const root = process.argv[1];
    const expected = require(`${root}/package.json`).dependencies["@mapbox/mapbox-gl-style-spec"];
    const installed = require(`${root}/node_modules/@mapbox/mapbox-gl-style-spec/package.json`).version;
    if (expected !== installed) process.exit(1);
  ' "$tools_root" >/dev/null 2>&1
then
  log "installing official Mapbox Style Spec validator"
  run_logged npm --prefix "$tools_root" ci --ignore-scripts
else
  log "Mapbox Style Spec validator already installed"
fi

if [[ ! -x "$venv_dir/bin/python" ]]; then
  log "creating shared maps MCP virtualenv"
  run_logged "$python_cmd" -m venv "$venv_dir"
else
  log "shared maps MCP virtualenv already exists"
fi

run_logged "$venv_dir/bin/python" -m pip --version
if [[ "${OPEN_WEB_CODEX_REFRESH_MAPS_MCP_ENV:-0}" == "1" ]] || ! check_imports
then
  log "installing or refreshing maps MCP dependencies"
  run_logged "$venv_dir/bin/python" -m pip install --disable-pip-version-check -e "$tools_root"
else
  log "maps MCP dependencies already import successfully"
fi
log "verifying maps MCP imports after setup"
check_imports
log "maps-mcp environment setup complete"
printf '%s\n' "$venv_dir"
