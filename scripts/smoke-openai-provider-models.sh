#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
codex_bin="${CODEX_BIN:-$repo_root/codex/codex-rs/target/debug/codex}"
timeout_sec="${OPENAI_PROVIDER_SMOKE_TIMEOUT_SEC:-90}"

if [[ ! -x "$codex_bin" ]]; then
  echo "Codex binary not found. Build it first or set CODEX_BIN: $codex_bin" >&2
  exit 1
fi

tmp_home="$(mktemp -d)"
trap 'rm -rf "$tmp_home"' EXIT

source_home="${OPEN_WEB_CODEX_IMPORT_CODEX_AUTH_FROM:-${CODEX_AUTH_SOURCE_HOME:-$HOME/.codex}}"
if [[ -f "$source_home/auth.json" ]]; then
  install -m 600 "$source_home/auth.json" "$tmp_home/auth.json"
else
  echo "No file-backed auth.json found at $source_home/auth.json. Run codex login with file-backed auth or set OPEN_WEB_CODEX_IMPORT_CODEX_AUTH_FROM." >&2
  exit 2
fi

python3 - "$codex_bin" "$tmp_home" "$timeout_sec" <<'PY'
import json
import os
import select
import subprocess
import sys
import time

codex_bin, codex_home, timeout_sec = sys.argv[1], sys.argv[2], int(sys.argv[3])
env = os.environ.copy()
env["CODEX_HOME"] = codex_home
proc = subprocess.Popen(
    [codex_bin, "app-server", "--stdio"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1,
    env=env,
)
next_id = 1

def send(method, params=None, request=True):
    global next_id
    message = {"method": method}
    request_id = None
    if request:
        request_id = next_id
        next_id += 1
        message["id"] = request_id
    if params is not None:
        message["params"] = params
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    return request_id

def read_response(request_id, timeout):
    deadline = time.time() + timeout
    while time.time() < deadline:
        readable, _, _ = select.select([proc.stdout], [], [], 0.2)
        if not readable:
            continue
        line = proc.stdout.readline()
        if not line:
            break
        message = json.loads(line)
        if message.get("id") == request_id:
            return message
    raise TimeoutError(f"Timed out waiting for response {request_id}")

try:
    request_id = send("initialize", {"clientInfo": {"name": "open-web-codex-openai-provider-smoke", "version": "0"}, "capabilities": {"experimentalApi": True}})
    init = read_response(request_id, 30)
    if "error" in init:
        raise SystemExit(init["error"])
    send("initialized", request=False)
    request_id = send("modelProvider/list", {})
    providers = read_response(request_id, timeout_sec)
    if "error" in providers:
        raise SystemExit(providers["error"])
    request_id = send("model/list", {"forceRefresh": True})
    models = read_response(request_id, timeout_sec)
    if "error" in models:
        raise SystemExit(models["error"])
    model_count = len(models.get("result", {}).get("data", []))
    if model_count == 0:
        raise SystemExit("OpenAI provider model list is empty")
    print(json.dumps({"ok": True, "modelCount": model_count}))
finally:
    try:
        proc.terminate()
        proc.wait(timeout=5)
    except Exception:
        proc.kill()
PY
