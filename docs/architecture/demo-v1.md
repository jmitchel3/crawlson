# Self-contained demo contract

Status: implemented in Crawlson 0.4.0.

The demo proves the smallest independently useful Crawlson product loop without
an external application, credentials, or private fixtures:

> Journey -> agent-run browser session -> evidence -> findings and guides

It is a compatibility and contributor fixture, not a second runner. The same
journey parser, safety checks, `agent-browser` adapter, run report, artifact
registry, focused-image renderer, finding renderer, and guide renderer are used
for the demo and for an authorized external target.

## Components

- `crawlson-demo` serves one stable visible page.
- `examples/demo-pass.toml` verifies the visible heading and captures the
  Continue action for a guide.
- `examples/demo-fail.toml` declares an intentionally wrong heading and links a
  later focused capture to that checkpoint as finding evidence.
- `scripts/demo.sh` coordinates the server, both journeys, rendering, and a
  missing-authorization safety check.
- `tests/real_agent_browser.rs` independently validates the full artifact and
  report contract through a real supported browser driver.

The example journeys use `http://127.0.0.1:4173` so their files remain stable
and directly runnable. The integration test copies those fixtures and replaces
only that exact origin with an ephemeral loopback port.

## Safety boundary

The server rejects every non-loopback bind address before opening a listener.
Port `0` is allowed for collision-free tests. It accepts only GET and HEAD,
returns 405 for other methods, embeds every asset in the response, and contains
no form, script, credential, external request, or state-changing route. Response
headers disable caching and cross-origin or embedded execution paths appropriate
for this fixture.

The server does not authorize Crawlson. Every run must still provide the exact
normalized origin through `--allow-origin`. Omitting it produces exit 3 and a
`blocked` report before `agent-browser` starts. This makes the safety behavior
part of the demonstration rather than a test-only assertion.

The shell command accepts only an absent or empty output directory and never
deletes old runs. It terminates only the `crawlson-demo` process it started,
requests graceful shutdown first, and uses a bounded forced stop only if that
owned process does not exit. SIGINT and SIGTERM stop the script after cleanup
with conventional 130 and 143 exits; they cannot resume the workflow with its
cleanup trap disabled.

## Expected outcomes and artifacts

The passing journey exits 0 and renders `render/guide.md` with a local focused
image. The failing journey exits 1 and renders `render/findings.json` and
`render/findings.md`. The unauthorized journey exits 3, reports `blocked`, and
has neither browser commands nor fabricated evidence. The coordinating script
validates those JSON outcomes, failure and block reason codes, successful
cleanup, the blocked run's empty command and artifact lists, and the required
evidence files. It exits 0 only when all checks match the contract.

Each executed browser run preserves `report.json`, a trace, a raw viewport PNG,
a focused PNG, and focus metadata. The focused image keeps the selected action
area readable, draws the configured vivid red outline, and dims the surrounding
page with a translucent near-black mask. The raw screenshot remains the
authoritative browser evidence; the focused image is a reproducible guide and
finding derivative, not a redaction.

The real-browser integration rehashes every registered artifact, decodes the
raw and focused PNGs, checks the exact outline color, checks that the action
interior is unchanged, checks that surrounding pixels are dimmed, validates the
guide's local image, and verifies deterministic finding provenance.

## CI contract

The normal cross-platform suite leaves the browser integration ignored because
not every contributor machine has a supported browser runtime. An explicit
ignored-test invocation is not allowed to silently pass: it requires
`CRAWLSON_REAL_BROWSER=required`, while `skip` is the only explicit portable
opt-out.

The dedicated Linux CI job pins `agent-browser 0.26.0`, installs its browser
runtime, runs the artifact-producing documented demo before the independent
integration assertions, and uploads logs and evidence with `if: always()`. The
aggregate `CI` check depends on both the cross-platform Rust suite and this
real-browser job.

Release signing and installers are intentionally outside this contract. Until a
signed release exists, the clean supported demonstration begins from a source
checkout with Rust 1.92 and a supported `agent-browser` installation.
