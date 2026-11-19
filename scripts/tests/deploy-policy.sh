#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/open-web-codex-deploy-policy.XXXXXX")"
fixture_root="$(cd "$fixture_root" && pwd)"
args_log="$fixture_root/run-local-args"
data_dir="$fixture_root/data"
fake_bin="$fixture_root/bin"

cleanup() {
  local pid=""
  if [[ -r "$data_dir/run/server.pid" ]]; then
    pid="$(tr -d '[:space:]' <"$data_dir/run/server.pid")"
  fi
  if [[ "$pid" =~ ^[0-9]+$ ]]; then
    kill -TERM "$pid" 2>/dev/null || true
  fi
  rm -rf "$fixture_root"
}
trap cleanup EXIT

mkdir -p \
  "$fixture_root/scripts" \
  "$fixture_root/apps/web/target/release" \
  "$fixture_root/codex/codex-rs" \
  "$fake_bin"
cp "$repo_root/scripts/deploy.sh" "$fixture_root/scripts/deploy.sh"
touch \
  "$fixture_root/apps/web/package-lock.json" \
  "$fixture_root/apps/web/Cargo.lock" \
  "$fixture_root/codex/codex-rs/Cargo.lock"
ln -s /bin/sleep \
  "$fixture_root/apps/web/target/release/open-web-codex-server"

cat >"$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '{"ok":true}\n'
EOF

cat >"$fake_bin/psql" <<'EOF'
#!/usr/bin/env bash
printf 'open_web_codex\n'
EOF

cat >"$fake_bin/ps" <<'EOF'
#!/usr/bin/env bash
if [[ "$*" == *"-o command="* ]]; then
  printf '%s 300\n' "${TEST_RELEASE_SERVER:?}"
  exit 0
fi
exec /bin/ps "$@"
EOF

cat >"$fixture_root/scripts/run-local.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data_dir="${OPEN_WEB_CODEX_DATA_DIR:?}"
pid_file="$data_dir/run/server.pid"
printf '%s\n' "$@" >>"${TEST_RUN_ARGS:?}"

if [[ "${FAKE_RUN_LOCAL_FAIL:-0}" == "1" ]]; then
  exit 42
fi

old_pid=""
if [[ -r "$pid_file" ]]; then
  old_pid="$(tr -d '[:space:]' <"$pid_file")"
fi
if [[ "$old_pid" =~ ^[0-9]+$ ]]; then
  kill -TERM "$old_pid" 2>/dev/null || true
fi

mkdir -p "$data_dir/run" "$data_dir/logs"
nohup "$repo_root/apps/web/target/release/open-web-codex-server" 300 \
  >"$data_dir/logs/server.log" 2>&1 </dev/null &
printf '%s\n' "$!" >"$pid_file"
EOF

chmod +x \
  "$fake_bin/curl" \
  "$fake_bin/ps" \
  "$fake_bin/psql" \
  "$fixture_root/scripts/deploy.sh" \
  "$fixture_root/scripts/run-local.sh"

run_deploy() {
  PATH="$fake_bin:$PATH" \
  TEST_RUN_ARGS="$args_log" \
  TEST_RELEASE_SERVER="$fixture_root/apps/web/target/release/open-web-codex-server" \
  OPEN_WEB_CODEX_DATA_DIR="$data_dir" \
  DATABASE_URL="postgresql://tester@127.0.0.1:5432/open_web_codex" \
    "$fixture_root/scripts/deploy.sh" "$@"
}

assert_arg_present() {
  grep -Fx -- "$1" "$args_log" >/dev/null || {
    printf 'missing delegated run-local argument: %s\n' "$1" >&2
    exit 1
  }
}

assert_arg_absent() {
  if grep -Fx -- "$1" "$args_log" >/dev/null; then
    printf 'unexpected delegated run-local argument: %s\n' "$1" >&2
    exit 1
  fi
}

: >"$args_log"
run_deploy >/dev/null
assert_arg_present "--release"
assert_arg_present "--restart"
assert_arg_absent "--background"
assert_arg_absent "--no-build"

: >"$args_log"
run_deploy --reuse-build >/dev/null
assert_arg_present "--release"
assert_arg_present "--restart"
assert_arg_present "--no-build"

old_pid="$(tr -d '[:space:]' <"$data_dir/run/server.pid")"
if FAKE_RUN_LOCAL_FAIL=1 run_deploy --reuse-build >/dev/null 2>&1; then
  printf 'delegated build failure was accepted\n' >&2
  exit 1
fi
kill -0 "$old_pid" 2>/dev/null || {
  printf 'existing service was stopped before the delegated build succeeded\n' >&2
  exit 1
}

# The managed database URL file must pass the permission gate with both stat
# dialects. GNU `stat -f` prints file-system status and fails instead of
# rejecting the option, so its output must never be mixed into the mode.
real_stat="$(command -v stat)"
stat_bin="$fixture_root/stat-bin"
mkdir -p "$stat_bin"
write_fake_stat() {
  local dialect="$1"
  cat >"$stat_bin/stat" <<STAT
#!/usr/bin/env bash
mode="\$("$real_stat" -c '%a' "\$3" 2>/dev/null || "$real_stat" -f '%Lp' "\$3")"
case "$dialect:\$1" in
  gnu:-c|bsd:-f) printf '%s\n' "\$mode" ;;
  gnu:-f) printf '  File: "%s"\n    ID: 0 Namelen: 255 Type: ext2/ext3\n' "\$3"; exit 1 ;;
  *) printf 'stat: illegal option\n' >&2; exit 1 ;;
esac
STAT
  chmod +x "$stat_bin/stat"
}
check_managed_url_file() {
  PATH="$stat_bin:$fake_bin:$PATH" \
  OPEN_WEB_CODEX_DATA_DIR="$data_dir" \
    "$fixture_root/scripts/deploy.sh" --check
}
printf 'postgresql://tester:secret@127.0.0.1:5432/open_web_codex\n' >"$data_dir/database-url"
for dialect in gnu bsd; do
  write_fake_stat "$dialect"
  chmod 600 "$data_dir/database-url"
  check_managed_url_file >/dev/null || {
    printf 'private database URL file was rejected with %s stat\n' "$dialect" >&2
    exit 1
  }
  chmod 644 "$data_dir/database-url"
  if check_managed_url_file >/dev/null 2>&1; then
    printf 'group/other-readable database URL file was accepted with %s stat\n' "$dialect" >&2
    exit 1
  fi
done
rm -f "$data_dir/database-url"

printf 'deploy policy tests passed\n'
