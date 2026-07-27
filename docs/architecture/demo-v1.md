# Self-contained demo contract

Status: implemented in Crawlson 0.4.0, extended with authorized link actions in
Crawlson 0.6.0, guide collections in Crawlson 0.7.0, and disposable
authenticated sessions in Crawlson 0.8.0.

The demo proves the smallest independently useful Crawlson product loop without
an external application, third-party credentials, or private fixtures:

> Journey -> agent-run browser session -> evidence -> findings and guides

It is a compatibility and contributor fixture, not a second runner. The same
journey parser, safety checks, `agent-browser` adapter, run report, artifact
registry, focused-image renderer, finding renderer, and guide renderer are used
for the demo and for an authorized external target.

## Components

- `crawlson-demo` serves stable start, completion, and authenticated-viewer
  pages.
- `examples/demo-pass.toml` verifies the visible heading and captures the
  Continue action for a guide.
- `examples/demo-fail.toml` declares an intentionally wrong heading and links a
  later focused capture to that checkpoint as finding evidence.
- `examples/follow-link-pass.toml` executes the Continue link once and verifies
  its exact destination before rendering an executed guide step.
- `examples/follow-link-fail.toml` follows a same-origin redirect fixture and
  turns its acknowledged wrong final URL into a link-postcondition finding.
- `examples/authenticated-pass.toml` imports disposable browser state and proves
  the declared viewer role through visible UI before capturing evidence.
- `scripts/demo.sh` coordinates the server, all five journeys, rendering,
  missing target-, action-, and authentication-state safety checks, a
  three-guide public collection, and a two-finding review collection.
- `tests/real_agent_browser.rs` independently validates the full artifact and
  report contract through a real supported browser driver.

The example journeys use `http://127.0.0.1:4173` so their files remain stable
and directly runnable. The integration test copies those fixtures and replaces
only that exact origin with an ephemeral loopback port.

## Safety boundary

The server rejects every non-loopback bind address before opening a listener.
Port `0` is allowed for collision-free tests. It accepts only GET and HEAD,
returns 405 for other methods, embeds every asset in the response, and contains
no form, external request, or state-changing route. The authenticated GET route
uses one static inline script to turn exact-origin disposable browser storage
into a visible role marker; it has no login or credential-collection flow.
Response headers disable caching and cross-origin or embedded execution paths
appropriate for this fixture.

The server does not authorize Crawlson. Every run must still provide the exact
normalized origin through `--allow-origin`, and every link action additionally
requires its exact `--allow-action JOURNEY@REVISION:STEP` grant. Omitting either
produces exit 3 and a `blocked` report before `agent-browser` starts. This makes
the safety behavior part of the demonstration rather than a test-only
assertion.

The authenticated journey additionally requires `--auth-state`. The script
creates its state document in a private operating-system temporary directory,
uses a unique per-run storage value, and removes it before collection generation.
Omitting it produces an explicit `authentication_state_missing` block before
browser launch. Retained demo output is scanned for both the disposable value
and the source path.

The shell command accepts only an absent or empty output directory and never
deletes old runs. It terminates only the `crawlson-demo` process it started,
requests graceful shutdown first, and uses a bounded forced stop only if that
owned process does not exit. SIGINT and SIGTERM stop the script after cleanup
with conventional 130 and 143 exits; they cannot resume the workflow with its
cleanup trap disabled.

## Expected outcomes and artifacts

The read-only, action, and authenticated passing journeys exit 0 and render
`render/guide.md` with local focused images. Their intentional failures exit 1
and render `render/findings.json` and `render/findings.md`. The three
preflight-denied journeys exit 3, report `blocked`, and have neither browser
commands nor fabricated evidence. The action pass additionally proves
`effect_verified`; its guide may
therefore say Crawlson executed the highlighted link exactly once. The
coordinating script validates those JSON outcomes, failure and block reason
codes, successful cleanup, the blocked runs' empty command lists, and the
required evidence files. It exits 0 only when all eight outcomes match the
contract.

The script then copies only the five public journey definitions into its
artifact workspace and writes two runtime manifests. The successful manifest
builds and checks a three-guide public root/topic/guide tree. The failure
manifest builds and checks a separate review tree and exits 1 both times; it must not emit a
partial public root index. Collection generation revalidates raw runs from a
bounded temporary snapshot and ignores the single-run `render/` directories the
demo already produced.

Each executed browser run preserves `report.json`, a trace, a raw viewport PNG,
a focused PNG, and focus metadata. The focused image keeps the selected action
area readable, draws the configured vivid red outline, and dims the surrounding
page with a translucent near-black mask. The raw screenshot remains the
authoritative browser evidence; the focused image is a reproducible guide and
finding derivative, not a redaction.

The public collection preserves each focused PNG byte-for-byte and adds
deterministic root, topic, guide, previous, and next navigation. The review tree
retains the structured findings and only their referenced evidence. Neither
focused UI pixels nor review evidence are automatically safe for public
release; an application should ingest a public tree only when its report is
`ready` and `publishable`.

The real-browser integration rehashes every registered artifact, decodes the
raw and focused PNGs, checks the exact outline color, checks that the action
interior is unchanged, checks that surrounding pixels are dimmed, validates the
guide's local image, verifies deterministic finding provenance, confirms the
real driver executed one link click and observed the exact destination, and
scans the complete authenticated run for its disposable state path and value.

## CI contract

The normal cross-platform suite leaves the browser integration ignored because
not every contributor machine has a supported browser runtime. An explicit
ignored-test invocation is not allowed to silently pass: it requires
`CRAWLSON_REAL_BROWSER=required`, while `skip` is the only explicit portable
opt-out.

The dedicated Linux CI job pins `agent-browser 0.26.0`, installs its browser
runtime, and requires an upstream live-launch diagnostic with ambient `CI`
removed. This matters because Crawlson's driver does not forward `CI` and does
not silently accept the upstream behavior of disabling Chrome's sandbox on CI
runners. The job currently pins Ubuntu 22.04, where downloaded Chrome retains
its sandbox; Ubuntu 24.04 AppArmor blocks that launch before the first command.
The job then runs the artifact-producing documented demo before the independent
integration assertions and uploads logs and evidence with `if: always()`. The
aggregate `CI` check depends on both the cross-platform Rust suite and this
real-browser job.

Release signing and installers are intentionally outside this contract. Until a
signed release exists, the clean supported demonstration begins from a source
checkout with Rust 1.92 and a supported `agent-browser` installation.
