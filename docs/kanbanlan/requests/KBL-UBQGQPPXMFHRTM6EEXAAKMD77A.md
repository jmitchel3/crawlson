# Ship the self-contained real-browser MVP demo

- Kanbanlan: `KBL-UBQGQPPXMFHRTM6EEXAAKMD77A`
- Canonical home: `github`
- Canonical request: [#14](https://github.com/jmitchel3/crawlson/issues/14)

## Request

## Outcome

Deliver Crawlson 0.4.0 as a credential-free local demonstration of the complete read-only product loop: authorized journey -> real `agent-browser` session -> raw/focused evidence -> honest outcome -> guide or findings.

## Acceptance criteria

- Add a small self-contained loopback demo application with stable visible UI and no third-party service or credentials.
- Include one passing guide journey and one intentionally failing finding journey, both strict journey v2 and application-independent.
- Provide one documented command that starts the demo, runs both journeys through real `agent-browser 0.26.x`, renders the outputs, and leaves inspectable artifacts.
- Demonstrate exact-origin authorization and an explicit blocked unsafe/missing-authorization result; no mutation or authentication shortcuts.
- Make the real-browser loop portable in CI, with reports/evidence uploaded even when an expected-failure or infrastructure step fails.
- Verify raw screenshots, red-box focused screenshots, dimmed surroundings, trace, findings, and guide links end to end.
- Add clean local installation/use instructions and predictable exit-code handling.
- Increment the package version to 0.4.0 and update README, TODO, changelog, architecture notes, examples, tests, and the durable delivery record.
- Keep release signing, public package publication, authentication, mutation, model judgment, and hosted guide publication out of scope.

## Safety

The demo must bind to loopback only, use read-only HTTP behavior, reject non-loopback binding, and require Crawlson exact-origin authorization like every other target.

## Decisions

- Ship the demo server as a separate `crawlson-demo` binary so the production
  CLI and journey contract do not acquire application-fixture behavior.
- Bind only explicit IPv4 or IPv6 loopback addresses and reject non-loopback
  configuration before listening. Serve only GET and HEAD; the UI has no
  external assets, scripts, forms, or mutating behavior.
- Keep the documented journeys at the stable `127.0.0.1:4173` origin for a
  copy-paste local workflow. The integration test copies those same fixtures and
  substitutes an ephemeral loopback port so parallel test hosts cannot collide.
- Treat pass, intentional failure, and missing authorization as three required
  demonstrations. The blocked case must contain no driver commands or invented
  browser evidence.
- Keep the full browser test ignored in the portable Rust suite, but make an
  explicit invocation fail unless `CRAWLSON_REAL_BROWSER=required` is set. CI
  has a dedicated required job and may opt out only by changing reviewed
  workflow code.
- Preserve all demo outputs and refuse non-empty destinations. The demo command
  owns and terminates only the server process it starts. Interrupt and terminate
  signals clean up and exit with 130 and 143 rather than resuming the workflow.
- Pin the CI npm package to `agent-browser 0.26.0`, use its exact native binary,
  and retain install, test, demo, report, evidence, and server logs as one CI
  artifact.

## Verification

- `cargo check --workspace --all-targets --all-features --locked`
- `cargo test --bin crawlson-demo --locked`
- `CRAWLSON_REAL_BROWSER=required AGENT_BROWSER_REAL_BIN=... cargo test --test real_agent_browser --locked -- --ignored --nocapture`
- `scripts/demo.sh --agent-browser ... --output-dir <new-directory>`
- An owned-process SIGTERM test observed exit 143 and confirmed that no failing
  or blocked journey began after the signal.
- `crawlson-demo --bind 0.0.0.0` rejected before listening
- The real-browser assertions independently rehashed raw screenshot, focused
  screenshot, focus metadata, and trace artifacts; decoded both PNGs; found the
  exact red outline; confirmed an unchanged action interior and dimmed page
  surroundings; and followed the generated guide image link.
- The one-command demo independently checks the pass/fail/blocked JSON semantics,
  reason codes, cleanup, blocked preflight emptiness, required evidence files,
  and guide/finding links before printing success.
- Full formatting, lint, portable test, release build, workflow, privacy, and
  shell checks are recorded in the delivery commit and pull request.

## Delivered result

Crawlson 0.4.0 includes a self-contained, credential-free demonstration of its
complete read-only product loop. A contributor can run one command to exercise
real `agent-browser` sessions, see an honest pass, an evidence-backed visible UI
failure, and a preflight safety block, then inspect the generated guide,
findings, screenshots, metadata, trace, and reports. CI requires the same
real-browser path and retains evidence even on failure.

Release signing, installers, public package publication, authentication,
mutation, model judgment, and hosted guide publication remain separate work.
