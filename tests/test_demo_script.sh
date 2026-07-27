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
printf '%s\n' "$*" >"$CRAWLSON_TEST_INVOCATION"
exit 23
SCRIPT
cat >"$test_root/bin/crawlson-demo" <<'SCRIPT'
#!/usr/bin/env bash
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

echo "demo script argument tests passed"
