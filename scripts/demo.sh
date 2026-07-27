#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
output_dir="$repo_root/crawlson-demo-output"
agent_browser="agent-browser"
crawlson_bin=""
demo_bin=""
demo_pid=""
cleanup_started=0

fail() {
  echo "crawlson demo: $*" >&2
  exit 1
}

cleanup() {
  if [[ "$cleanup_started" -eq 1 ]]; then
    return
  fi
  cleanup_started=1
  if [[ -n "$demo_pid" ]] && kill -0 "$demo_pid" 2>/dev/null; then
    kill -TERM "$demo_pid" 2>/dev/null || true
    for _ in {1..100}; do
      if ! kill -0 "$demo_pid" 2>/dev/null; then
        break
      fi
      sleep 0.05
    done
    if kill -0 "$demo_pid" 2>/dev/null; then
      kill -KILL "$demo_pid" 2>/dev/null || true
    fi
    wait "$demo_pid" 2>/dev/null || true
  fi
}

exit_after_signal() {
  local status="$1"
  trap - EXIT INT TERM
  cleanup
  exit "$status"
}

expect_exit() {
  local expected="$1"
  local stdout_path="$2"
  local stderr_path="$3"
  shift 3
  set +e
  "$@" >"$stdout_path" 2>"$stderr_path"
  local actual=$?
  set -e
  if [[ "$actual" -ne "$expected" ]]; then
    echo "crawlson demo: expected exit $expected, received $actual" >&2
    sed -n '1,120p' "$stdout_path" >&2 || true
    sed -n '1,120p' "$stderr_path" >&2 || true
    exit 1
  fi
}

json_string() {
  local key="$1"
  local path="$2"
  sed -n "s/.*\"$key\":\"\([^\"]*\)\".*/\1/p" "$path" | head -n 1
}

require_json_fragment() {
  local path="$1"
  local fragment="$2"
  local description="$3"
  local document
  document="$(<"$path")"
  [[ "$document" == *"$fragment"* ]] \
    || fail "$description was not present in $path"
}

require_artifact() {
  local path="$1"
  [[ -s "$path" ]] || fail "required artifact is missing or empty: $path"
}

resolve_executable() {
  local option="$1"
  local path="$2"
  [[ -f "$path" ]] || fail "$option path is not a file: $path"
  [[ -x "$path" ]] || fail "$option path is not executable: $path"
  local directory
  directory="$(cd "$(dirname "$path")" && pwd -P)" \
    || fail "could not resolve $option path: $path"
  printf '%s/%s\n' "$directory" "$(basename "$path")"
}

resolve_agent_browser() {
  local candidate="$1"
  if [[ "$candidate" != */* ]]; then
    candidate="$(command -v "$candidate" 2>/dev/null)" \
      || fail "--agent-browser executable was not found on PATH: $1"
  fi
  resolve_executable --agent-browser "$candidate"
}

usage() {
  cat <<'USAGE'
Usage: scripts/demo.sh [--agent-browser PATH] [--output-dir DIRECTORY]
                       [--crawlson-bin PATH --demo-bin PATH]

Runs the passing, failing, and blocked Crawlson demo journeys and preserves all
reports and evidence. The output directory must be absent or empty. By default,
the demo builds both Crawlson binaries from this checkout. Provide both binary
overrides to run a packaged pair without rebuilding it.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --agent-browser)
      [[ $# -ge 2 ]] || fail "--agent-browser requires a path"
      agent_browser="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || fail "--output-dir requires a directory"
      output_dir="$2"
      shift 2
      ;;
    --crawlson-bin)
      [[ $# -ge 2 ]] || fail "--crawlson-bin requires a path"
      [[ -n "$2" ]] || fail "--crawlson-bin requires a non-empty path"
      crawlson_bin="$2"
      shift 2
      ;;
    --demo-bin)
      [[ $# -ge 2 ]] || fail "--demo-bin requires a path"
      [[ -n "$2" ]] || fail "--demo-bin requires a non-empty path"
      demo_bin="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "unknown argument: $1"
      ;;
  esac
done

if [[ -n "$crawlson_bin" || -n "$demo_bin" ]]; then
  [[ -n "$crawlson_bin" && -n "$demo_bin" ]] \
    || fail "--crawlson-bin and --demo-bin must be provided together"
  crawlson_bin="$(resolve_executable --crawlson-bin "$crawlson_bin")"
  demo_bin="$(resolve_executable --demo-bin "$demo_bin")"
fi
agent_browser="$(resolve_agent_browser "$agent_browser")"

trap cleanup EXIT
trap 'exit_after_signal 130' INT
trap 'exit_after_signal 143' TERM

if [[ -e "$output_dir" ]]; then
  [[ -d "$output_dir" ]] || fail "output path is not a directory: $output_dir"
  [[ -z "$(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit)" ]] \
    || fail "output directory is not empty: $output_dir"
else
  mkdir -p "$output_dir"
fi
output_dir="$(cd "$output_dir" && pwd)"

cd "$repo_root"
export CRAWLSON_NO_UPDATE_CHECK=1
export CRAWLSON_OFFLINE=1

if [[ -z "$crawlson_bin" ]]; then
  cargo build --locked --bins
  crawlson_bin="$repo_root/target/debug/crawlson"
  demo_bin="$repo_root/target/debug/crawlson-demo"
fi
[[ -x "$crawlson_bin" ]] || fail "Crawlson binary is not executable: $crawlson_bin"
[[ -x "$demo_bin" ]] || fail "demo binary is not executable: $demo_bin"

"$crawlson_bin" doctor --json --agent-browser "$agent_browser" >"$output_dir/doctor.json"

"$demo_bin" --port 4173 --json >"$output_dir/demo-ready.json" 2>"$output_dir/demo-server.log" &
demo_pid=$!
for _ in {1..200}; do
  if ! kill -0 "$demo_pid" 2>/dev/null; then
    fail "demo server exited before becoming ready; see $output_dir/demo-server.log"
  fi
  if [[ -s "$output_dir/demo-ready.json" ]]; then
    break
  fi
  sleep 0.05
done

origin="$(json_string origin "$output_dir/demo-ready.json")"
[[ "$origin" == "http://127.0.0.1:4173" ]] || fail "demo server emitted an unexpected origin"

runs_dir="$output_dir/runs"
expect_exit 0 "$output_dir/pass-run.json" "$output_dir/pass-run.stderr" \
  "$crawlson_bin" --json run "$repo_root/examples/demo-pass.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"
pass_run_dir="$(json_string run_directory "$output_dir/pass-run.json")"
[[ -n "$pass_run_dir" ]] || fail "passing report omitted its run directory"
expect_exit 0 "$output_dir/pass-render.json" "$output_dir/pass-render.stderr" \
  "$crawlson_bin" --json render "$pass_run_dir" \
  --journey "$repo_root/examples/demo-pass.toml"

expect_exit 1 "$output_dir/fail-run.json" "$output_dir/fail-run.stderr" \
  "$crawlson_bin" --json run "$repo_root/examples/demo-fail.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"
fail_run_dir="$(json_string run_directory "$output_dir/fail-run.json")"
[[ -n "$fail_run_dir" ]] || fail "failing report omitted its run directory"
expect_exit 1 "$output_dir/fail-render.json" "$output_dir/fail-render.stderr" \
  "$crawlson_bin" --json render "$fail_run_dir" \
  --journey "$repo_root/examples/demo-fail.toml"

expect_exit 3 "$output_dir/blocked-run.json" "$output_dir/blocked-run.stderr" \
  "$crawlson_bin" --json run "$repo_root/examples/demo-pass.toml" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"

expect_exit 0 "$output_dir/action-pass-run.json" "$output_dir/action-pass-run.stderr" \
  "$crawlson_bin" --json run "$repo_root/examples/follow-link-pass.toml" \
  --allow-origin "$origin" \
  --allow-action "demo.follow-link-pass@1:follow-continue" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
action_pass_run_dir="$(json_string run_directory "$output_dir/action-pass-run.json")"
[[ -n "$action_pass_run_dir" ]] || fail "action report omitted its run directory"
expect_exit 0 "$output_dir/action-pass-render.json" "$output_dir/action-pass-render.stderr" \
  "$crawlson_bin" --json render "$action_pass_run_dir" \
  --journey "$repo_root/examples/follow-link-pass.toml"

expect_exit 1 "$output_dir/action-fail-run.json" "$output_dir/action-fail-run.stderr" \
  "$crawlson_bin" --json run "$repo_root/examples/follow-link-fail.toml" \
  --allow-origin "$origin" \
  --allow-action "demo.follow-link-fail@1:follow-broken-redirect" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
action_fail_run_dir="$(json_string run_directory "$output_dir/action-fail-run.json")"
[[ -n "$action_fail_run_dir" ]] || fail "failing action report omitted its run directory"
expect_exit 1 "$output_dir/action-fail-render.json" "$output_dir/action-fail-render.stderr" \
  "$crawlson_bin" --json render "$action_fail_run_dir" \
  --journey "$repo_root/examples/follow-link-fail.toml"

expect_exit 3 "$output_dir/action-blocked-run.json" "$output_dir/action-blocked-run.stderr" \
  "$crawlson_bin" --json run "$repo_root/examples/follow-link-pass.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"

require_json_fragment "$output_dir/pass-run.json" '"outcome":"passed"' \
  "passing journey outcome"
require_json_fragment "$output_dir/pass-run.json" '"execution_outcome":"passed"' \
  "passing journey execution outcome"
require_json_fragment "$output_dir/pass-run.json" \
  '"cleanup":{"attempted":true,"status":"passed"}' "passing journey cleanup"
require_json_fragment "$output_dir/pass-render.json" '"status":"guide_ready"' \
  "passing render status"

require_json_fragment "$output_dir/fail-run.json" '"outcome":"failed"' \
  "intentional failure outcome"
require_json_fragment "$output_dir/fail-run.json" '"execution_outcome":"failed"' \
  "intentional failure execution outcome"
require_json_fragment "$output_dir/fail-run.json" \
  '"reason":{"code":"checkpoint_failed"' "intentional failure reason"
require_json_fragment "$output_dir/fail-run.json" \
  '"cleanup":{"attempted":true,"status":"passed"}' "failing journey cleanup"
require_json_fragment "$output_dir/fail-render.json" '"status":"findings_ready"' \
  "failing render status"

require_json_fragment "$output_dir/blocked-run.json" '"outcome":"blocked"' \
  "missing-authorization outcome"
require_json_fragment "$output_dir/blocked-run.json" \
  '"reason":{"code":"target_authorization_missing"' \
  "missing-authorization reason"
require_json_fragment "$output_dir/blocked-run.json" \
  '"driver":{"name":"agent-browser","commands":[]}' \
  "blocked run empty driver command list"
require_json_fragment "$output_dir/blocked-run.json" '"artifacts":[]' \
  "blocked run empty artifact list"

require_json_fragment "$output_dir/action-pass-run.json" '"schema_version":2' \
  "action run report version"
require_json_fragment "$output_dir/action-pass-run.json" '"action_state":"effect_verified"' \
  "verified link action state"
require_json_fragment "$output_dir/action-pass-render.json" '"status":"guide_ready"' \
  "action guide status"
require_json_fragment "$output_dir/action-fail-run.json" '"outcome":"failed"' \
  "post-action mismatch outcome"
require_json_fragment "$output_dir/action-fail-run.json" \
  '"id":"follow-broken-redirect"' "post-action mismatch step"
require_json_fragment "$output_dir/action-fail-run.json" \
  '"action_state":"driver_acknowledged"' "post-action acknowledged state"
require_json_fragment "$output_dir/action-fail-run.json" \
  '"observed_url":"http://127.0.0.1:4173/unexpected"' \
  "post-action observed destination"
require_json_fragment "$output_dir/action-fail-render.json" '"status":"findings_ready"' \
  "post-action findings status"
require_json_fragment "$output_dir/action-blocked-run.json" \
  '"reason":{"code":"action_authorization_mismatch"' \
  "missing action authorization reason"
require_json_fragment "$output_dir/action-blocked-run.json" \
  '"driver":{"name":"agent-browser","commands":[]}' \
  "action preflight empty driver command list"

for run_dir in "$pass_run_dir" "$fail_run_dir"; do
  require_artifact "$run_dir/report.json"
  require_artifact "$run_dir/evidence/trace.json"
  require_artifact "$run_dir/evidence/003-capture-action.raw.png"
  require_artifact "$run_dir/evidence/003-capture-action.focused.png"
  require_artifact "$run_dir/evidence/003-capture-action.focused.json"
done

require_artifact "$pass_run_dir/render/guide.md"
require_artifact "$pass_run_dir/render/001-focused.png"
require_artifact "$fail_run_dir/render/findings.json"
require_artifact "$fail_run_dir/render/findings.md"
cmp -s "$pass_run_dir/evidence/003-capture-action.focused.png" \
  "$pass_run_dir/render/001-focused.png" \
  || fail "passing guide image does not match the verified focused evidence"
require_json_fragment "$pass_run_dir/render/guide.md" '](001-focused.png)' \
  "passing guide local image link"
require_json_fragment "$fail_run_dir/render/findings.md" \
  '../evidence/003-capture-action.focused.png' "finding focused-evidence link"

require_artifact "$action_pass_run_dir/evidence/002-follow-continue.raw.png"
require_artifact "$action_pass_run_dir/evidence/002-follow-continue.focused.png"
require_artifact "$action_pass_run_dir/evidence/002-follow-continue.focused.json"
require_artifact "$action_pass_run_dir/render/guide.md"
require_json_fragment "$action_pass_run_dir/render/guide.md" \
  'executed this highlighted link action once' "executed-action guide claim"
require_artifact "$action_fail_run_dir/render/findings.json"
require_artifact "$action_fail_run_dir/render/findings.md"
require_artifact "$action_fail_run_dir/evidence/003-follow-broken-redirect.raw.png"
require_artifact "$action_fail_run_dir/evidence/003-follow-broken-redirect.focused.png"
require_artifact "$action_fail_run_dir/evidence/003-follow-broken-redirect.focused.json"
require_json_fragment "$action_fail_run_dir/render/findings.md" \
  'Observed: path /unexpected' "post-action observed path finding"

echo "Crawlson demo passed."
echo "Artifacts: $output_dir"
echo "Guide: $pass_run_dir/render/guide.md"
echo "Findings: $fail_run_dir/render/findings.md"
