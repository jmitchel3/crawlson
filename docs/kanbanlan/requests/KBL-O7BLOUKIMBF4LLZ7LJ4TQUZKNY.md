# Add secret-safe authenticated browser sessions

- Kanbanlan: `KBL-O7BLOUKIMBF4LLZ7LJ4TQUZKNY`
- Canonical home: `github`
- Canonical request: [#26](https://github.com/jmitchel3/crawlson/issues/26)

## Request

Outcome: run a journey that declares authentication through one replaceable agent-browser state-file provider while keeping target and action authorization fail closed and keeping secret material, state contents, and local credential paths out of reports, logs, evidence metadata, guides, and command provenance. Acceptance: validate a bounded regular state file supplied outside the journey; distinguish missing, unsupported, invalid, load-failed, blocked, and verified outcomes honestly; load state only after pre-browser safety checks; bind provider and role without secret identifiers in report provenance; add schema, fake-driver, real agent-browser loopback, demo, privacy, and failure-mode tests; update architecture, README, TODO, changelog, packaged workflows, and bump the minor version. Out of scope: collecting usernames or passwords, automated login forms, reusable Chrome profiles, arbitrary headers, mutations, production credentials, hosted secret storage, and publishing owner-gated artifacts.

## Decisions

- Journey v4 requires one application-neutral authentication declaration with
  provider, public role, and the visible `check_text` step that verifies that
  role. Run-report v3 binds only those public fields, journey provenance, and
  exact target origin. Journey v1-v3 behavior is preserved.
- The first provider accepts an externally supplied `agent-browser` state file.
  Exact target and action authorization are resolved before the source path is
  accessed. The bounded typed state accepts only exact-origin browser storage;
  it rejects cookie entries, unknown fields, ambiguous entries,
  empty effective state, nonregular files, and links.
- Cookie import remains fail-closed because the pinned driver's hostname-only
  allowlist cannot prevent a browser cookie from reaching another port on the
  same host. This provider supports exact-origin local and session storage until
  an exact-origin request boundary exists.
- Validated state bytes are copied to a private operating-system temporary
  directory outside the run tree under the neutral name `state.json`. The
  adapter loads that copy exactly once before trace capture and deletes it
  immediately after the attempt. Because `agent-browser 0.26` echoes the load
  path, authentication-load stdout and stderr provenance is deliberately
  represented as empty instead of hashing upstream output.
- Driver acceptance is not authentication proof. Only the declared visible UI
  checkpoint changes authentication to `verified`; a mismatch blocks the run
  before later evidence, actions, findings, or guides.
- Authenticated screenshots and traces remain evidence and are not described as
  redacted. The demo and required real-browser test use disposable state and
  scan every retained file for unique source-path and state-value sentinels.

## Verification

- `cargo fmt --all --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --all-targets --all-features` with loopback
  access: 132 passed, one explicitly ignored real-browser test.
- `CRAWLSON_REAL_BROWSER=required cargo test --test real_agent_browser --locked
  -- --ignored --nocapture`: one passed through `agent-browser 0.26.0`, including
  authenticated trace and retained-tree privacy scans.
- `bash tests/test_demo_script.sh`: passed.
- `bash scripts/demo.sh --output-dir <new temporary directory>`: all eight
  passed, failed, and blocked outcomes matched; three-guide collection and
  two-finding review tree checked byte-for-byte.
- Native release build and `crawlson-release package` for
  `aarch64-apple-darwin`: generated the 0.8.0 bundle with the authenticated
  example. The extracted bundle's real-browser eight-outcome demo passed.
- `crawlson version` and `clson version`: both reported `crawlson 0.8.0`.
- Manual inspection of the authenticated focused PNG confirmed the vivid red
  action-area box, unchanged target interior, and near-black dimmed surroundings.
- Repository privacy scan found no private-project identifier.

## Delivered result

Crawlson 0.8.0 can run a read-only, exact-origin browser-storage authenticated
journey through the existing replaceable `agent-browser` boundary without
placing state paths or values in journeys, reports, command provenance,
evidence, rendered guides, or guide collections. The CLI accepts `--auth-state`
through both `crawlson` and `clson`, reports six explicit authentication states,
visibly verifies the declared role, and preserves the existing target/action
safety model and red-box/dimmed guide evidence. The self-contained demo,
real-browser integration, release bundle, schemas, architecture contract,
README, backlog, and changelog cover the new workflow.

Automated login, reusable browser profiles, hosted secret storage, mutating
journeys, and production credentials remain out of scope. Public 0.8.0 signing
and publication remain owner-gated by license, namespace, and production-key
decisions.
