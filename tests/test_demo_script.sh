#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
test_root="$(mktemp -d)"

cleanup() {
  rm -rf "$test_root"
}
trap cleanup EXIT

fail() {
  echo "demo script test: $*" >&2
  exit 1
}

expect_status() {
  local expected="$1"
  shift
  set +e
  "$@"
  local actual=$?
  set -e
  [[ "$actual" -eq "$expected" ]] \
    || fail "expected exit $expected, received $actual"
}

help_output="$(bash "$repo_root/scripts/demo.sh" --help)"
[[ "$help_output" == *"--crawlson-bin PATH --demo-bin PATH"* ]] \
  || fail "help did not document packaged binary overrides"

mkdir -p "$test_root/bin"
cat >"$test_root/bin/crawlson" <<'SCRIPT'
#!/usr/bin/env bash
if [[ "${CRAWLSON_TEST_CLEANUP_MODE:-}" == "1" && "${1:-}" == "doctor" ]]; then
  printf '%s\n' '{"status":"ready"}'
  exit 0
fi
printf '%s\n' "$*" >"$CRAWLSON_TEST_INVOCATION"
exit 23
SCRIPT
cat >"$test_root/bin/crawlson-demo" <<'SCRIPT'
#!/usr/bin/env bash
if [[ "${CRAWLSON_TEST_CLEANUP_MODE:-}" == "1" ]]; then
  printf '%s\n' "$$" >"$CRAWLSON_TEST_DEMO_PID"
  printf '%s\n' "{\"schema_version\":1,\"status\":\"ready\",\"origin\":\"http://127.0.0.1:4173\",\"pid\":$$}"
  trap 'exit 0' TERM
  while true; do
    sleep 1
  done
fi
exit 24
SCRIPT
cat >"$test_root/bin/cargo" <<'SCRIPT'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$CRAWLSON_TEST_CARGO_MARKER"
exit 25
SCRIPT
cat >"$test_root/bin/agent-browser" <<'SCRIPT'
#!/usr/bin/env bash
exit 26
SCRIPT
chmod +x "$test_root/bin/crawlson" "$test_root/bin/crawlson-demo" \
  "$test_root/bin/cargo" "$test_root/bin/agent-browser"

export CRAWLSON_TEST_CARGO_MARKER="$test_root/cargo-invocation"
expect_status 25 env PATH="$test_root/bin:$PATH" \
  bash "$repo_root/scripts/demo.sh" \
  --output-dir "$test_root/source-output" \
  >"$test_root/source.stdout" 2>"$test_root/source.stderr"
grep -Fx -- "build --locked --bins" "$CRAWLSON_TEST_CARGO_MARKER" >/dev/null \
  || fail "the default source path did not run the locked binary build"
rm "$CRAWLSON_TEST_CARGO_MARKER"

expect_status 1 bash "$repo_root/scripts/demo.sh" \
  --crawlson-bin "$test_root/bin/crawlson" \
  --output-dir "$test_root/missing-pair" \
  >"$test_root/missing-pair.stdout" 2>"$test_root/missing-pair.stderr"
grep -F -- "--crawlson-bin and --demo-bin must be provided together" \
  "$test_root/missing-pair.stderr" >/dev/null \
  || fail "a partial packaged override did not fail closed"

cp "$test_root/bin/crawlson-demo" "$test_root/non-executable-demo"
chmod -x "$test_root/non-executable-demo"
if [[ ! -x "$test_root/non-executable-demo" ]]; then
  expect_status 1 bash "$repo_root/scripts/demo.sh" \
    --crawlson-bin "$test_root/bin/crawlson" \
    --demo-bin "$test_root/non-executable-demo" \
    --output-dir "$test_root/non-executable" \
    >"$test_root/non-executable.stdout" 2>"$test_root/non-executable.stderr"
  grep -F -- "--demo-bin path is not executable" \
    "$test_root/non-executable.stderr" >/dev/null \
    || fail "a non-executable packaged override did not fail closed"
fi

export CRAWLSON_TEST_INVOCATION="$test_root/crawlson-invocation"
resolved_agent_browser="$(cd "$test_root/bin" && pwd -P)/agent-browser"
expect_status 23 env PATH="$test_root/bin:$PATH" \
  bash "$repo_root/scripts/demo.sh" \
  --crawlson-bin "$test_root/bin/crawlson" \
  --demo-bin "$test_root/bin/crawlson-demo" \
  --agent-browser agent-browser \
  --output-dir "$test_root/packaged-output" \
  >"$test_root/packaged.stdout" 2>"$test_root/packaged.stderr"
[[ ! -e "$CRAWLSON_TEST_CARGO_MARKER" ]] \
  || fail "packaged binary overrides unexpectedly invoked cargo"
grep -F -- "doctor --json --agent-browser $resolved_agent_browser" \
  "$CRAWLSON_TEST_INVOCATION" >/dev/null \
  || fail "packaged Crawlson override was not invoked with the resolved driver"

mkdir -p "$test_root/auth-tmp"
export CRAWLSON_TEST_DEMO_PID="$test_root/cleanup-demo.pid"
expect_status 1 env CRAWLSON_TEST_CLEANUP_MODE=1 \
  TMPDIR="$test_root/auth-tmp" PATH="$test_root/bin:$PATH" \
  bash "$repo_root/scripts/demo.sh" \
  --crawlson-bin "$test_root/bin/crawlson" \
  --demo-bin "$test_root/bin/crawlson-demo" \
  --agent-browser agent-browser \
  --output-dir "$test_root/cleanup-output" \
  >"$test_root/cleanup.stdout" 2>"$test_root/cleanup.stderr"
[[ -z "$(find "$test_root/auth-tmp" -mindepth 1 -print -quit)" ]] \
  || fail "failure-path cleanup retained private authentication state"
cleanup_demo_pid="$(<"$CRAWLSON_TEST_DEMO_PID")"
if kill -0 "$cleanup_demo_pid" 2>/dev/null; then
  fail "failure-path cleanup did not reap the owned demo process"
fi
[[ "$(<"$test_root/cleanup.stderr")" != *"$test_root/auth-tmp"* ]] \
  || fail "failure-path diagnostics exposed the private authentication path"

mkdir -p "$test_root/failing-rm-bin" "$test_root/auth-tmp-failure"
cat >"$test_root/failing-rm-bin/rm" <<'SCRIPT'
#!/usr/bin/env bash
printf 'raw deletion failure: %s\n' "$*" >&2
exit 1
SCRIPT
chmod +x "$test_root/failing-rm-bin/rm"
export CRAWLSON_TEST_DEMO_PID="$test_root/cleanup-failure-demo.pid"
expect_status 1 env CRAWLSON_TEST_CLEANUP_MODE=1 \
  TMPDIR="$test_root/auth-tmp-failure" \
  PATH="$test_root/failing-rm-bin:$test_root/bin:$PATH" \
  bash "$repo_root/scripts/demo.sh" \
  --crawlson-bin "$test_root/bin/crawlson" \
  --demo-bin "$test_root/bin/crawlson-demo" \
  --agent-browser agent-browser \
  --output-dir "$test_root/cleanup-failure-output" \
  >"$test_root/cleanup-failure.stdout" 2>"$test_root/cleanup-failure.stderr"
cleanup_failure_stderr="$(<"$test_root/cleanup-failure.stderr")"
[[ "$cleanup_failure_stderr" == *"could not remove private authentication state"* ]] \
  || fail "authentication cleanup failure was not reported generically"
[[ "$cleanup_failure_stderr" != *"$test_root/auth-tmp-failure"* ]] \
  || fail "authentication cleanup failure exposed the private path"
cleanup_failure_demo_pid="$(<"$CRAWLSON_TEST_DEMO_PID")"
if kill -0 "$cleanup_failure_demo_pid" 2>/dev/null; then
  fail "cleanup failure did not reap the owned demo process"
fi

echo "demo script argument tests passed"
