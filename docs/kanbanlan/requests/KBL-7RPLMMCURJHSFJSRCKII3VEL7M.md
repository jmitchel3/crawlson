# Add fail-closed disposable mutating journeys

- Kanbanlan: `KBL-7RPLMMCURJHSFJSRCKII3VEL7M`
- Canonical home: `github`
- Canonical request: [#28](https://github.com/jmitchel3/crawlson/issues/28)

## Request

Outcome: add one independently useful mutation vertical slice to Crawlson: a versioned mutating journey contract, exact per-step and exact-target authorization, an extra explicit production mutation gate, disposable fixture declaration, deterministic visible-UI form interaction through agent-browser, pre-action focused evidence, no action retries, honest unknown-effect reporting, and a guaranteed cleanup phase whose result is retained without hiding the primary outcome. Include a self-contained loopback fixture and required real-browser, schema, renderer, guide, privacy, cleanup-failure, denial, installed-demo, and four-target release dry-run coverage. Preserve v1-v4 behavior and keep fixture/application mechanics behind replaceable boundaries.

## Decisions

- Keep Rust as the core and invoke the pinned `agent-browser 0.26.x` process
  through the existing typed driver boundary. Mutation support does not expose
  a generic script, click, input, or browser-automation API.
- Version the new contract as journey v5, run-report v4, and findings v3.
  Setup, user-journey, and fixture-cleanup steps share one ordered step model
  with an explicit read-only or mutating effect classification. Authentication
  verification must be a read-only setup step.
- Require an exact digest-bound grant for every mutating main or cleanup step.
  Non-loopback targets require the complete grant set a second time as explicit
  production authorization. Missing, extra, malformed, or duplicate grants
  block before authentication state access or browser launch.
- Require the user to select an extension-capable Chromium or Chrome for
  Testing executable. Materialize and attest an owned Manifest V3 extension
  that permits only the exact scheme, host, and effective port and blocks other
  HTTP(S), WebSocket, and WebTransport traffic. Keep this enforcement in the
  driver adapter rather than the journey model.
- Permit only a generated public fixture token, a simple-ID ordinary text
  field, and an exact same-origin POST submit control. Dispatch each fill or
  click once, never retry an uncertain mutation, and bind every dispatch to an
  immediately preceding raw/focused screenshot with a vivid red outline and
  translucent near-black surrounding mask.
- Make cleanup a declared visible-UI phase. Store the authoritative non-secret
  recovery barrier in user-global Crawlson state, retain a historical copy in
  the originating run, hold an operating-system lock through mutation and
  cleanup, and clear the authority only after verified absence. An exact rerun
  with a pending barrier performs only the authentication prefix and cleanup,
  returns blocked as `recovery_completed`, and never executes main steps.
- Preserve the main execution outcome separately from later cleanup, evidence,
  trace, diagnostics, or browser-close errors. Guides and findings remain
  offline renderer outputs and are publishable only after complete provenance,
  successful fixture cleanup, and a cleared recovery barrier.
- Increment the product version to 0.9.0 because journey/report/finding schemas,
  mutation authorization, the release bundle, and the public CLI contract all
  gained backward-compatible but substantial new behavior.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features --locked` (all portable
  unit and integration suites pass: 172 passed; the explicitly selected
  real-browser test remains ignored in this portable invocation)
- `bash tests/test_demo_script.sh`
- `CRAWLSON_REAL_BROWSER=required cargo test --test real_agent_browser --locked
  -- --ignored --nocapture`; the live integration also sensitivity-checks a
  second loopback trap and proves cross-port `fetch`, `sendBeacon`, and
  WebSocket attempts are blocked while exact-origin mutation and cleanup pass
- `scripts/demo.sh` against local `agent-browser 0.26.0` and Chrome for Testing,
  including read-only, deterministic failure, blocked, authorized-link,
  authenticated, disposable-mutation, render, findings, collection-build, and
  collection-check cases
- Manual inspection of the real mutation field, create-button, and cleanup
  focused screenshots confirmed the red target outline and dimmed surround.
- JSON syntax checks for every published schema, shell syntax and argument
  tests for the packaged demo, YAML parsing for both workflows, CLI/alias
  version parity, offline upgrade behavior, privacy scans, and `git diff
  --check`.
- Optimized native build plus local aarch64 macOS bundle packaging; the archive
  contains both command names, the demo, all documented examples, the v5
  schema, and the packaged demo script with a matching release fragment.

## Delivered result

Crawlson 0.9.0 now has one independently useful, fail-closed mutation vertical
slice and a self-contained disposable loopback demonstration. Successful runs
produce trustworthy focused evidence and guides; deterministic main failures
can produce structured findings only after cleanup; unsafe, unauthenticated,
incomplete, unknown-effect, and pending-recovery states stay visibly blocked or
errored. The release dry-run matrix installs the packaged command pair and runs
the complete browser demo on all four supported targets without publication
authority.

Public release remains separate and owner-gated by the license, namespace, and
production signing-key decisions tracked in the release request. Generic agent
exploration, failed-request capture/redaction, and durable installer crash
recovery remain later independently reviewable outcomes.
