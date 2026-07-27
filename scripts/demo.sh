#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
output_dir="$repo_root/crawlson-demo-output"
agent_browser="agent-browser"
browser_executable=""
crawlson_bin=""
demo_bin=""
demo_pid=""
auth_state_dir=""
auth_state_path=""
auth_state_scan_path=""
cleanup_started=0

fail() {
  echo "crawlson demo: $*" >&2
  exit 1
}

remove_auth_state() {
  local failed=0
  if [[ -n "$auth_state_path" && -e "$auth_state_path" ]]; then
    rm -f "$auth_state_path" 2>/dev/null || failed=1
  fi
  if [[ -n "$auth_state_dir" && -d "$auth_state_dir" ]]; then
    rmdir "$auth_state_dir" 2>/dev/null || failed=1
  fi
  if [[ "$failed" -ne 0 ]]; then
    return 1
  fi
  auth_state_path=""
  auth_state_dir=""
}

cleanup() {
  if [[ "$cleanup_started" -eq 1 ]]; then
    return
  fi
  cleanup_started=1
  remove_auth_state \
    || echo "crawlson demo: could not remove private authentication state" >&2
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

require_private_value_absent() {
  local value="$1"
  [[ -n "$value" ]] || fail "privacy scan sentinel was missing"
  set +e
  grep -R -a -F "$value" "$output_dir" >/dev/null 2>&1
  local status=$?
  set -e
  case "$status" in
    0) fail "private authentication material leaked into retained demo output" ;;
    1) ;;
    *) fail "could not complete retained-output privacy scan" ;;
  esac
}

require_focus_overlay() {
  local path="$1"
  local compact
  compact="$(tr -d '[:space:]' <"$path")"
  [[ "$compact" == *'"renderer_algorithm":"focus-overlay-v1"'* ]] \
    || fail "focused evidence did not use the expected renderer: $path"
  [[ "$compact" == *'"mask_rgba":[0,0,0,166]'* ]] \
    || fail "focused evidence did not dim its surroundings: $path"
  [[ "$compact" == *'"outline_rgba":[255,45,45,255]'* ]] \
    || fail "focused evidence did not use the vivid red action outline: $path"
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

browser_is_extension_capable() {
  local version
  version="$("$1" --version 2>/dev/null || true)"
  [[ "$version" == *"Chrome for Testing"* || "$version" == *"Chromium"* ]]
}

resolve_browser_executable() {
  local candidate="$1"
  [[ ! -L "$candidate" ]] \
    || fail "--browser-executable path must not be a symbolic link: $candidate"
  candidate="$(resolve_executable --browser-executable "$candidate")"
  browser_is_extension_capable "$candidate" \
    || fail "--browser-executable must identify Chromium or Chrome for Testing"
  printf '%s\n' "$candidate"
}

discover_browser_executable() {
  local roots=()
  local root candidate
  if [[ -n "${PLAYWRIGHT_BROWSERS_PATH:-}" ]]; then
    roots+=("$PLAYWRIGHT_BROWSERS_PATH")
  fi
  if [[ -n "${HOME:-}" ]]; then
    roots+=(
      "$HOME/.cache/ms-playwright"
      "$HOME/Library/Caches/ms-playwright"
    )
  fi
  if [[ -n "${LOCALAPPDATA:-}" ]]; then
    roots+=("$LOCALAPPDATA/ms-playwright")
  fi

  for root in "${roots[@]}"; do
    [[ -d "$root" ]] || continue
    while IFS= read -r candidate; do
      case "$candidate" in
        */chromium-*/chrome-linux*/chrome|\
        */chromium-*/chrome-mac*/Google\ Chrome\ for\ Testing.app/Contents/MacOS/Google\ Chrome\ for\ Testing|\
        */chromium-*/chrome-win*/chrome.exe)
          if [[ -f "$candidate" && -x "$candidate" ]] \
            && browser_is_extension_capable "$candidate"; then
            resolve_browser_executable "$candidate"
            return 0
          fi
          ;;
      esac
    done < <(find "$root" -type f -print 2>/dev/null | sort -r)
  done
  return 1
}

materialize_journey() {
  local source="$1"
  local destination="$2"
  sed "s|origin = \"http://127.0.0.1:4173\"|origin = \"$origin\"|" \
    "$source" >"$destination"
  grep -F "origin = \"$origin\"" "$destination" >/dev/null \
    || fail "could not bind journey to the ephemeral demo origin: $source"
}

usage() {
  cat <<'USAGE'
Usage: scripts/demo.sh [--agent-browser PATH] [--browser-executable PATH]
                       [--output-dir DIRECTORY]
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
    --browser-executable)
      [[ $# -ge 2 ]] || fail "--browser-executable requires a path"
      [[ -n "$2" ]] || fail "--browser-executable requires a non-empty path"
      browser_executable="$2"
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
if [[ -n "$browser_executable" ]]; then
  browser_executable="$(resolve_browser_executable "$browser_executable")"
else
  browser_executable="$(discover_browser_executable)" \
    || fail "no extension-capable Chromium or Chrome for Testing executable was found; provide --browser-executable"
fi

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
export CRAWLSON_HOME="$output_dir/crawlson-state"

if [[ -z "$crawlson_bin" ]]; then
  cargo build --locked --bins
  crawlson_bin="$repo_root/target/debug/crawlson"
  demo_bin="$repo_root/target/debug/crawlson-demo"
fi
[[ -x "$crawlson_bin" ]] || fail "Crawlson binary is not executable: $crawlson_bin"
[[ -x "$demo_bin" ]] || fail "demo binary is not executable: $demo_bin"

"$crawlson_bin" doctor --json --agent-browser "$agent_browser" >"$output_dir/doctor.json"

"$demo_bin" --port 0 --json >"$output_dir/demo-ready.json" 2>"$output_dir/demo-server.log" &
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
[[ "$origin" =~ ^http://127\.0\.0\.1:[0-9]+$ ]] \
  || fail "demo server emitted an unexpected origin"

journeys_dir="$output_dir/journeys"
mkdir -p "$journeys_dir"
for journey_name in \
  authenticated-pass \
  demo-fail \
  demo-pass \
  follow-link-fail \
  follow-link-pass \
  mutating-pass; do
  materialize_journey "$repo_root/examples/$journey_name.toml" \
    "$journeys_dir/$journey_name.toml"
done

auth_storage_value="crawlson-demo-fixture-$demo_pid"
auth_state_dir="$(mktemp -d "${TMPDIR:-/tmp}/crawlson-auth-demo.XXXXXX")"
chmod 700 "$auth_state_dir"
auth_state_path="$auth_state_dir/state.json"
auth_state_scan_path="$auth_state_path"
printf '%s\n' "{\"cookies\":[],\"origins\":[{\"origin\":\"$origin\",\"localStorage\":[{\"name\":\"crawlson_demo_session\",\"value\":\"$auth_storage_value\"}]}]}" >"$auth_state_path"
chmod 600 "$auth_state_path"

runs_dir="$output_dir/runs"
expect_exit 0 "$output_dir/pass-run.json" "$output_dir/pass-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/demo-pass.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"
pass_run_dir="$(json_string run_directory "$output_dir/pass-run.json")"
[[ -n "$pass_run_dir" ]] || fail "passing report omitted its run directory"
expect_exit 0 "$output_dir/pass-render.json" "$output_dir/pass-render.stderr" \
  "$crawlson_bin" --json render "$pass_run_dir" \
  --journey "$journeys_dir/demo-pass.toml"

expect_exit 1 "$output_dir/fail-run.json" "$output_dir/fail-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/demo-fail.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"
fail_run_dir="$(json_string run_directory "$output_dir/fail-run.json")"
[[ -n "$fail_run_dir" ]] || fail "failing report omitted its run directory"
expect_exit 1 "$output_dir/fail-render.json" "$output_dir/fail-render.stderr" \
  "$crawlson_bin" --json render "$fail_run_dir" \
  --journey "$journeys_dir/demo-fail.toml"

expect_exit 3 "$output_dir/blocked-run.json" "$output_dir/blocked-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/demo-pass.toml" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"

expect_exit 0 "$output_dir/action-pass-run.json" "$output_dir/action-pass-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/follow-link-pass.toml" \
  --allow-origin "$origin" \
  --allow-action "demo.follow-link-pass@1:follow-continue" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
action_pass_run_dir="$(json_string run_directory "$output_dir/action-pass-run.json")"
[[ -n "$action_pass_run_dir" ]] || fail "action report omitted its run directory"
expect_exit 0 "$output_dir/action-pass-render.json" "$output_dir/action-pass-render.stderr" \
  "$crawlson_bin" --json render "$action_pass_run_dir" \
  --journey "$journeys_dir/follow-link-pass.toml"

expect_exit 1 "$output_dir/action-fail-run.json" "$output_dir/action-fail-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/follow-link-fail.toml" \
  --allow-origin "$origin" \
  --allow-action "demo.follow-link-fail@1:follow-broken-redirect" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
action_fail_run_dir="$(json_string run_directory "$output_dir/action-fail-run.json")"
[[ -n "$action_fail_run_dir" ]] || fail "failing action report omitted its run directory"
expect_exit 1 "$output_dir/action-fail-render.json" "$output_dir/action-fail-render.stderr" \
  "$crawlson_bin" --json render "$action_fail_run_dir" \
  --journey "$journeys_dir/follow-link-fail.toml"

expect_exit 3 "$output_dir/action-blocked-run.json" "$output_dir/action-blocked-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/follow-link-pass.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"

expect_exit 0 "$output_dir/auth-pass-run.json" "$output_dir/auth-pass-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/authenticated-pass.toml" \
  --allow-origin "$origin" --auth-state "$auth_state_path" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
auth_pass_run_dir="$(json_string run_directory "$output_dir/auth-pass-run.json")"
[[ -n "$auth_pass_run_dir" ]] || fail "authenticated report omitted its run directory"
expect_exit 0 "$output_dir/auth-pass-render.json" "$output_dir/auth-pass-render.stderr" \
  "$crawlson_bin" --json render "$auth_pass_run_dir" \
  --journey "$journeys_dir/authenticated-pass.toml"
expect_exit 3 "$output_dir/auth-blocked-run.json" "$output_dir/auth-blocked-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/authenticated-pass.toml" \
  --allow-origin "$origin" --output-dir "$runs_dir" --agent-browser "$agent_browser"

mutation_grants=(
  "demo.mutating-pass@1:fill-fixture-name"
  "demo.mutating-pass@1:create-fixture"
  "demo.mutating-pass@1:ensure-fixture-absent"
)
mutation_arguments=()
for grant in "${mutation_grants[@]}"; do
  mutation_arguments+=(--allow-mutation "$grant")
done
expect_exit 0 "$output_dir/mutation-pass-run.json" "$output_dir/mutation-pass-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/mutating-pass.toml" \
  --allow-origin "$origin" "${mutation_arguments[@]}" \
  --auth-state "$auth_state_path" --browser-executable "$browser_executable" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
mutation_pass_run_dir="$(json_string run_directory "$output_dir/mutation-pass-run.json")"
[[ -n "$mutation_pass_run_dir" ]] || fail "mutation report omitted its run directory"
expect_exit 0 "$output_dir/mutation-pass-render.json" "$output_dir/mutation-pass-render.stderr" \
  "$crawlson_bin" --json render "$mutation_pass_run_dir" \
  --journey "$journeys_dir/mutating-pass.toml"

expect_exit 3 "$output_dir/mutation-grant-blocked-run.json" \
  "$output_dir/mutation-grant-blocked-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/mutating-pass.toml" \
  --allow-origin "$origin" --auth-state "$auth_state_path" \
  --browser-executable "$browser_executable" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"
expect_exit 3 "$output_dir/mutation-auth-blocked-run.json" \
  "$output_dir/mutation-auth-blocked-run.stderr" \
  "$crawlson_bin" --json run "$journeys_dir/mutating-pass.toml" \
  --allow-origin "$origin" "${mutation_arguments[@]}" \
  --browser-executable "$browser_executable" \
  --output-dir "$runs_dir" --agent-browser "$agent_browser"

remove_auth_state || fail "could not remove private authentication state"

cat >"$output_dir/guide-collection.toml" <<EOF
schema_version = 1

[collection]
id = "crawlson-demo-help"
title = "Crawlson Demo Help"
description = "Verified workflows produced by the self-contained demo."

[[topics]]
id = "getting-started"
title = "Getting started"
description = "Complete the demo through its visible interface."
order = 10
audience = ["visitors"]

[[topics.guides]]
key = "review-continue"
order = 10
run = "runs/$(basename "$pass_run_dir")"
journey = "journeys/demo-pass.toml"

[[topics.guides]]
key = "follow-continue"
order = 20
run = "runs/$(basename "$action_pass_run_dir")"
journey = "journeys/follow-link-pass.toml"

[[topics.guides]]
key = "authenticated-viewer"
order = 30
run = "runs/$(basename "$auth_pass_run_dir")"
journey = "journeys/authenticated-pass.toml"

[[topics.guides]]
key = "create-disposable-fixture"
order = 40
run = "runs/$(basename "$mutation_pass_run_dir")"
journey = "journeys/mutating-pass.toml"
EOF

expect_exit 0 "$output_dir/collection-build.json" "$output_dir/collection-build.stderr" \
  "$crawlson_bin" --json guides build "$output_dir/guide-collection.toml" \
  --output "$output_dir/guide-site"
expect_exit 0 "$output_dir/collection-check.json" "$output_dir/collection-check.stderr" \
  "$crawlson_bin" --json guides check "$output_dir/guide-collection.toml" \
  --output "$output_dir/guide-site"

cat >"$output_dir/finding-collection.toml" <<EOF
schema_version = 1

[collection]
id = "crawlson-demo-review"
title = "Crawlson Demo Findings"
description = "Deterministic failures retained for review."

[[topics]]
id = "known-failures"
title = "Known failures"
description = "Intentional fixtures that demonstrate honest bug reporting."
order = 10
audience = ["reviewers"]

[[topics.guides]]
key = "wrong-heading"
order = 10
run = "runs/$(basename "$fail_run_dir")"
journey = "journeys/demo-fail.toml"

[[topics.guides]]
key = "broken-redirect"
order = 20
run = "runs/$(basename "$action_fail_run_dir")"
journey = "journeys/follow-link-fail.toml"
EOF

expect_exit 1 "$output_dir/review-build.json" "$output_dir/review-build.stderr" \
  "$crawlson_bin" --json guides build "$output_dir/finding-collection.toml" \
  --output "$output_dir/guide-review"
expect_exit 1 "$output_dir/review-check.json" "$output_dir/review-check.stderr" \
  "$crawlson_bin" --json guides check "$output_dir/finding-collection.toml" \
  --output "$output_dir/guide-review"

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
  "\"observed_url\":\"$origin/unexpected\"" \
  "post-action observed destination"
require_json_fragment "$output_dir/action-fail-render.json" '"status":"findings_ready"' \
  "post-action findings status"
require_json_fragment "$output_dir/action-blocked-run.json" \
  '"reason":{"code":"action_authorization_mismatch"' \
  "missing action authorization reason"
require_json_fragment "$output_dir/action-blocked-run.json" \
  '"driver":{"name":"agent-browser","commands":[]}' \
  "action preflight empty driver command list"
require_json_fragment "$output_dir/auth-pass-run.json" '"schema_version":3' \
  "authenticated run report version"
require_json_fragment "$output_dir/auth-pass-run.json" '"status":"verified"' \
  "authenticated verification status"
require_json_fragment "$output_dir/auth-pass-render.json" '"status":"guide_ready"' \
  "authenticated guide status"
require_json_fragment "$output_dir/auth-blocked-run.json" \
  '"reason":{"code":"authentication_state_missing"' \
  "missing authentication state reason"
require_json_fragment "$output_dir/auth-blocked-run.json" \
  '"driver":{"name":"agent-browser","commands":[]}' \
  "authentication preflight empty driver command list"
require_json_fragment "$output_dir/mutation-pass-run.json" '"schema_version":4' \
  "mutation run report version"
require_json_fragment "$output_dir/mutation-pass-run.json" '"outcome":"passed"' \
  "mutation outcome"
require_json_fragment "$output_dir/mutation-pass-run.json" \
  '"execution_outcome":"passed"' "mutation execution outcome"
require_json_fragment "$output_dir/mutation-pass-run.json" \
  '"setup_status":"passed"' "mutation fixture setup status"
require_json_fragment "$output_dir/mutation-pass-run.json" \
  '"mutation_attempted":true' "mutation fixture dispatch status"
require_json_fragment "$output_dir/mutation-pass-run.json" \
  '"cleanup_status":"passed"' "mutation fixture cleanup status"
require_json_fragment "$output_dir/mutation-pass-run.json" \
  '"recovery_required":false' "mutation recovery status"
require_json_fragment "$output_dir/mutation-pass-render.json" '"status":"guide_ready"' \
  "mutation guide status"
require_json_fragment "$output_dir/mutation-grant-blocked-run.json" \
  '"reason":{"code":"mutation_authorization_mismatch"' \
  "missing mutation authorization reason"
require_json_fragment "$output_dir/mutation-grant-blocked-run.json" \
  '"driver":{"name":"agent-browser","commands":[]}' \
  "mutation authorization preflight empty driver command list"
require_json_fragment "$output_dir/mutation-auth-blocked-run.json" \
  '"reason":{"code":"authentication_state_missing"' \
  "missing disposable authentication state reason"
require_json_fragment "$output_dir/mutation-auth-blocked-run.json" \
  '"driver":{"name":"agent-browser","commands":[]}' \
  "mutation authentication preflight empty driver command list"
require_json_fragment "$output_dir/collection-build.json" '"status":"ready"' \
  "guide collection build status"
require_json_fragment "$output_dir/collection-build.json" '"guides":4' \
  "guide collection guide count"
cmp -s "$output_dir/collection-build.json" "$output_dir/collection-check.json" \
  || fail "guide collection build and check reports differ"
require_json_fragment "$output_dir/review-build.json" '"status":"findings"' \
  "guide review build status"
require_json_fragment "$output_dir/review-build.json" '"findings":2' \
  "guide review finding count"
cmp -s "$output_dir/review-build.json" "$output_dir/review-check.json" \
  || fail "guide review build and check reports differ"

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

require_artifact "$auth_pass_run_dir/evidence/003-capture-viewer-access.raw.png"
require_artifact "$auth_pass_run_dir/evidence/003-capture-viewer-access.focused.png"
require_artifact "$auth_pass_run_dir/evidence/003-capture-viewer-access.focused.json"
require_artifact "$auth_pass_run_dir/render/guide.md"

for stem in 005-fill-fixture-name 006-create-fixture 008-ensure-fixture-absent; do
  require_artifact "$mutation_pass_run_dir/evidence/$stem.raw.png"
  require_artifact "$mutation_pass_run_dir/evidence/$stem.focused.png"
  require_artifact "$mutation_pass_run_dir/evidence/$stem.focused.json"
  require_focus_overlay "$mutation_pass_run_dir/evidence/$stem.focused.json"
  cmp -s "$mutation_pass_run_dir/evidence/$stem.raw.png" \
    "$mutation_pass_run_dir/evidence/$stem.focused.png" \
    && fail "focused mutation evidence was identical to its raw screenshot: $stem"
done
require_artifact "$mutation_pass_run_dir/render/guide.md"
require_artifact "$mutation_pass_run_dir/render/001-focused.png"
require_artifact "$mutation_pass_run_dir/render/002-focused.png"
cmp -s "$mutation_pass_run_dir/evidence/005-fill-fixture-name.focused.png" \
  "$mutation_pass_run_dir/render/001-focused.png" \
  || fail "mutation field guide image does not match verified focused evidence"
cmp -s "$mutation_pass_run_dir/evidence/006-create-fixture.focused.png" \
  "$mutation_pass_run_dir/render/002-focused.png" \
  || fail "mutation button guide image does not match verified focused evidence"
if [[ -d "$CRAWLSON_HOME/.crawlson-recovery" ]] \
  && [[ -n "$(find "$CRAWLSON_HOME/.crawlson-recovery" -type f -print -quit)" ]]; then
  fail "successful fixture cleanup retained a pending recovery record"
fi

require_artifact "$output_dir/guide-site/index.md"
require_artifact "$output_dir/guide-site/topics/getting-started/index.md"
require_artifact "$output_dir/guide-site/topics/getting-started/review-continue/index.md"
require_artifact "$output_dir/guide-site/topics/getting-started/review-continue/001-focused.png"
require_artifact "$output_dir/guide-site/topics/getting-started/follow-continue/index.md"
require_artifact "$output_dir/guide-site/topics/getting-started/follow-continue/001-focused.png"
require_artifact "$output_dir/guide-site/topics/getting-started/authenticated-viewer/index.md"
require_artifact "$output_dir/guide-site/topics/getting-started/authenticated-viewer/001-focused.png"
require_artifact "$output_dir/guide-site/topics/getting-started/create-disposable-fixture/index.md"
require_artifact "$output_dir/guide-site/topics/getting-started/create-disposable-fixture/001-focused.png"
require_artifact "$output_dir/guide-site/topics/getting-started/create-disposable-fixture/002-focused.png"

require_private_value_absent "$auth_storage_value"
require_private_value_absent "$auth_state_scan_path"
cmp -s "$pass_run_dir/evidence/003-capture-action.focused.png" \
  "$output_dir/guide-site/topics/getting-started/review-continue/001-focused.png" \
  || fail "collection guide image does not match verified focused evidence"
cmp -s "$action_pass_run_dir/evidence/002-follow-continue.focused.png" \
  "$output_dir/guide-site/topics/getting-started/follow-continue/001-focused.png" \
  || fail "collection action image does not match verified focused evidence"
cmp -s "$mutation_pass_run_dir/evidence/005-fill-fixture-name.focused.png" \
  "$output_dir/guide-site/topics/getting-started/create-disposable-fixture/001-focused.png" \
  || fail "collection mutation field image does not match verified focused evidence"
cmp -s "$mutation_pass_run_dir/evidence/006-create-fixture.focused.png" \
  "$output_dir/guide-site/topics/getting-started/create-disposable-fixture/002-focused.png" \
  || fail "collection mutation button image does not match verified focused evidence"
require_artifact "$output_dir/guide-review/review/index.md"
require_artifact "$output_dir/guide-review/review/known-failures/wrong-heading/render/findings.json"
require_artifact "$output_dir/guide-review/review/known-failures/broken-redirect/render/findings.json"
[[ ! -e "$output_dir/guide-review/index.md" ]] \
  || fail "findings collection emitted a partial public guide index"

echo "Crawlson demo passed."
echo "Artifacts: $output_dir"
echo "Guide: $pass_run_dir/render/guide.md"
echo "Findings: $fail_run_dir/render/findings.md"
echo "Guide collection: $output_dir/guide-site/index.md"
echo "Guide review: $output_dir/guide-review/review/index.md"
