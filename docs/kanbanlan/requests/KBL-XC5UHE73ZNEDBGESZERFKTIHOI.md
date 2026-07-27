# Execute and verify authorized same-origin link actions

- Kanbanlan: `KBL-XC5UHE73ZNEDBGESZERFKTIHOI`
- Canonical home: `github`
- Canonical request: [#22](https://github.com/jmitchel3/crawlson/issues/22)

## Request

Outcome: deliver Crawlson 0.6.0 journey schema v3 with one real, deterministic agent-browser action: follow a declared same-origin link and prove its exact postcondition.

Acceptance criteria:
- Keep journey v1/v2 behavior immutable; v3 adds follow_link with selector, exact expected path, alt text, and guide instruction.
- Require an exact runtime grant bound to journey ID, revision, and step before browser launch; missing or mismatched grants are blocked.
- Before dispatch, verify current origin, target visibility, target enabled state, and exact same-origin href; capture raw and focused red-box evidence immediately before the action.
- Use a default-deny per-run driver policy containing only the capabilities declared by the validated journey; do not expose generic fill, type, script, upload, submit, or tabs.
- Execute click through shell-free arguments, never retry an attempted click, and independently verify the exact resulting URL. Off-origin or uncertain execution must never pass.
- Version the run/report evidence needed to distinguish unattempted, completed, and unknown action state and bind the pre-action capture to the click command sequence.
- Render guide prose as executed only when the click and postcondition passed; preserve reproducible evidence/findings on failure.
- Extend the self-contained demo, schemas, docs, fake-driver contract tests, and required real agent-browser CI proof. Keep authentication and arbitrary mutation explicitly blocked.

## Decisions

- Journey v3 adds only `follow_link`; it does not expose a generic click or
  input primitive. V1 and v2 documents retain their existing behavior and
  report schema.
- An action declaration and target-origin allowlist are both necessary but not
  sufficient alone. Execution additionally requires an exact
  `JOURNEY@REVISION:STEP` grant, whose report binding covers the source digest,
  target origin, required set, and granted set.
- The real driver receives a per-run default-deny policy. Action-capable runs
  add only attribute inspection, enabled-state inspection, and click to the
  read-only command set. Dispatch uses the declared CSS selector intersected
  with `a[href]` and the exact observed href, so buttons and changed hrefs do
  not satisfy the click selector.
- The runner captures focused evidence before dispatch, clicks once, never
  retries after dispatch, and independently checks the exact final URL. A
  post-dispatch timeout or malformed acknowledgement is `effect_unknown`, not
  a deterministic failure or pass.
- Run report v2 and findings v2 represent action provenance without changing
  the immutable v1 schemas. A guide may call a link action executed only when
  the click acknowledgement and exact destination are verified.
- Authentication, form input, buttons, script execution, uploads, arbitrary
  agent actions, and data mutation remain unavailable and fail closed.
- The pinned driver can prevent network activity by hostname, but not by exact
  scheme and port. Crawlson detects full-origin escapes before and after each
  step; it does not claim to prevent an unobserved transient same-host redirect.
  V3 is limited to targets where that documented upstream constraint is
  acceptable.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --locked --workspace --all-targets --all-features` (51 library,
  3 demo-server, 34 CLI, 10 installer, 8 release-library, and 2 release-CLI
  tests passed; the separately required real-browser test is ignored here by
  contract)
- `bash tests/test_demo_script.sh`
- `CRAWLSON_REAL_BROWSER=required cargo test --locked --test real_agent_browser
  -- --ignored --nocapture` (passed with `agent-browser 0.26.0`)
- `bash scripts/demo.sh --output-dir <new temporary directory>` (all six
  outcomes passed through the real browser)
- Manually inspected both real pre-action derivatives: the selected link
  remained readable inside a vivid red outline and the surrounding page was
  dimmed by the near-black mask.
- Three independent read-only audits covered action safety/state, renderer and
  schema integrity, evidence truthfulness, demo/release behavior, documentation,
  and repository policy. All actionable findings were fixed and re-reviewed;
  the driver's hostname-only preventive boundary remains an explicitly accepted
  and documented upstream limitation.
- `git diff --check` and repository privacy/path scans passed.
- Hosted aggregate CI and the non-publishing release matrix remain required
  PR gates and will be appended before merge.

## Delivered result

Crawlson 0.6.0 defines, executes, reports, and renders one real UI action. The
loopback examples demonstrate both a verified link and a broken postcondition,
with raw screenshots, vivid red-box/dimmed focused derivatives, trace and
command provenance, a verified guide, and reproducible findings. Broader
actions, authenticated sessions, disposable mutation fixtures, and public
release signing remain separate follow-up outcomes.
