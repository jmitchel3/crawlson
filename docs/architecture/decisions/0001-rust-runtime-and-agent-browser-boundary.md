# ADR 0001: Rust runtime and process-level agent-browser boundary

- Status: Accepted
- Date: 2026-07-27
- Decision owners: Crawlson maintainers

## Context

Crawlson needs a small, independently useful vertical slice that runs an
authorized browser journey, keeps evidence, reports an honest outcome, and can
render a guide only from verified steps. The first browser execution path must
use `agent-browser`, but journey and report contracts must not depend on one
driver's internal implementation.

The owner prefers Rust when it can provide a first-class `agent-browser`
integration, with Python and then TypeScript as fallbacks. End-to-end journey
time will remain dominated by browser, application, model, and network latency;
the runtime choice should improve startup, packaging, resource use, and
correctness without claiming to make web pages intrinsically faster.

The installed `agent-browser` 0.26.0 command-line interface provides the
capabilities needed for the first slice: structured JSON output, JSON batch
input, isolated sessions, bounding boxes, screenshots, traces, HAR and request
capture, console and page errors, domain restrictions, action policies, and
non-zero failures.

## Decision

Crawlson's MVP core and CLI will be written in Rust. The first browser driver
will invoke the supported `agent-browser` executable as a child process. It
will not import private crates, use upstream internal modules, or share Rust
types with `agent-browser`.

The initial compatibility range is `agent-browser >=0.26.0,<0.27.0`, gated by
contract tests. Crawlson will reject other minor or major versions with an
actionable diagnostic until their contract suite passes. Crawlson's own release
version is independent of the browser-driver version, and it never upgrades the
driver implicitly or during a run.

### Runtime comparison

| Constraint | Rust | Python | TypeScript |
| --- | --- | --- | --- |
| Process and JSON adapter | Strong process, I/O, and typed-deserialization support | Strong process support and quickest experiments | Strong process and JSON support |
| Distribution | Single native executable is the intended user experience | Interpreter/environment packaging requires extra care | Requires a Node-compatible runtime or bundled executable |
| Long-running orchestration | Low overhead with explicit concurrency and cancellation | Adequate, with a larger runtime and packaging surface | Adequate, with a larger runtime and packaging surface |
| `agent-browser` compatibility | First-class through its stable CLI; sharing its implementation language is convenient but not required | First-class through the same CLI | First-class through the same CLI |
| Main cost | Compile time and a higher implementation learning curve | Runtime distribution and weaker compile-time modeling | Runtime distribution and source-system inertia |

Rust best fits the desired install and orchestration model. The decision is not
based solely on `agent-browser` also being implemented in Rust; the executable
boundary gives all three languages equivalent access to its public protocol.

## Adapter contract

The core owns a versioned, driver-neutral request and response model. The
`agent-browser` adapter translates between that model and child processes:

1. Discover the executable from explicit configuration and then `PATH`; never
   invoke it through a shell.
2. Run `agent-browser --version`, parse the version strictly, and reject an
   unsupported version before opening a browser. A future `crawlson doctor`
   command will report executable, version, browser-install, and configuration
   status without silently changing either installation.
3. Allocate one sanitized, unguessable `crawlson-<run-id>` session for each
   run. Pass `--session`, `--json`, a Crawlson-owned config path, and
   defense-in-depth safety flags on every invocation. Never operate on the
   default session, use persistent browser state without an explicit auth
   adapter, or call `close --all`. Resolve the executable to an absolute path
   and launch it with a minimal environment so user-level `AGENT_BROWSER_*`,
   proxy, profile, state, and session values cannot silently change a run.
4. Send only output-independent control sequences to `batch --bail --json` as
   a JSON array of argument arrays on standard input. Use individual argument
   arrays when a later action depends on an earlier observation or output must
   be tightly scoped. In the initial range, individual output is a
   `success`/`data`/`error`/`warning` envelope and batch output contains an
   envelope for each command. Journey data is never interpolated into a shell
   command.
5. Treat standard output as the structured protocol. Capture bounded standard
   output and standard error, with the latter retained as redacted diagnostics.
   Missing required data, malformed JSON, an oversized response, or a conflict
   between process status and response status fails visibly. Unknown fields are
   tolerated for forward compatibility. A confirmation-required response is
   not an executed action, even if its outer response reports success.
6. Apply per-action and per-run deadlines. On cancellation or timeout, stop
   accepting actions, terminate the owned child, allow a bounded cleanup grace
   period, attempt evidence finalization, and close only the owned session.
   Because terminating the foreground CLI may not cancel work already accepted
   by its daemon, the adapter must prove bounded session cleanup in its contract
   tests and report a visible cleanup failure when it cannot.
7. Preserve action request, structured response, timing, exit status, and
   redacted diagnostic metadata in execution order.

Implementation status in 0.2: per-command and overall run deadlines, bounded
evidence/cleanup commands, explicit owned-session close, and a daemon idle
reaper are implemented and contract-tested. Graceful operating-system signal
handling is deliberately still open; a forced termination may prevent the
final report from being written, while the daemon reaper still bounds orphaned
session lifetime. `TODO.md` keeps that limitation visible.

The adapter exposes capabilities rather than upstream commands: navigate,
observe, locate, act, capture, start/stop tracing, inspect diagnostics, and
close. Unsupported capabilities are reported explicitly.

### Evidence and focused guide images

Evidence capture keeps the source artifact separate from presentation:

- start a trace before the initial navigation when tracing is requested;
- retain raw viewport screenshots, current URL, target locator, and target
  bounding box as run evidence;
- collect console errors, page errors, and relevant failed requests without
  secrets;
- stop and retain the trace even when later actions fail, where possible; and
- preserve partial evidence for `failed`, `blocked`, and `error` outcomes.

Raw screenshots and traces may contain sensitive application data. HAR files
may additionally contain cookies, headers, request bodies, and responses, so
HAR capture remains opt-in until a tested redaction pipeline exists. Artifact
paths returned by the driver must resolve beneath the current run directory;
each accepted artifact records its size, media type, and digest.

For each guide action, the renderer creates a derivative from the raw
screenshot and recorded bounding box. The derivative dims the surrounding
viewport with a translucent near-black mask, leaves the action area visible,
and draws a high-contrast red rectangular outline around that area. The raw
image remains unchanged and is the authoritative evidence. The derivative
records its source artifact, target box, viewport, device scale, padding, mask
opacity, outline color, and outline width so it can be reproduced. A missing or
invalid target box makes the guide image incomplete; it must not be silently
approximated.

This overlay is Crawlson rendering behavior, not an `agent-browser` feature and
not injected into the tested page.

## Outcomes and errors

The driver never decides the final run outcome. It returns typed observations
and failures for the core to classify:

| Condition | Core classification |
| --- | --- |
| Observable checkpoint not met | `failed` |
| Required credentials, fixture, or authorization absent | `blocked` |
| Target or mutation rejected by declared policy | `blocked` |
| Executable/browser missing or unsupported driver version | `error` |
| Child failure, malformed JSON, timeout, cancellation, or crash | `error` |
| Cleanup failure after otherwise successful actions | visible cleanup failure; never `passed` |

Agent observations may create findings but cannot convert a deterministic
failure or an adapter error into a pass. A guide may be complete only when all
of its included actions and checkpoints were completed in a `passed` run.

## Safety ownership

Crawlson core remains the authority for:

- explicit target authorization, using exact scheme, hostname, and effective
  port, including every redirect and new page;
- read-only default behavior and explicit per-operation mutation capabilities;
- refusal of production mutations without exact target and operation approval;
- disposable fixtures and visible cleanup for mutating journeys;
- authentication/session-provider boundaries and secret redaction;
- allowed artifact paths, downloads, uploads, subprocess arguments, and
  diagnostic retention; and
- action authorization before dispatch and outcome classification afterward.

The adapter also supplies `agent-browser` domain restrictions and an explicit
action policy. These are defense in depth, not substitutes for core checks. Its
domain restriction is hostname-oriented,
so the core separately checks exact origins before navigation and again after
every action that can navigate. Navigation origins and permitted subresource
hosts are distinct. If a safe exact-origin policy cannot be enforced—for
example, a sensitive service shares an allowed hostname on another port—the
run is blocked as unsupported.

The MVP does not depend on driver confirmation prompts. Crawlson core makes the
authorization decision before dispatch, and the driver action policy defaults
to deny with only the exact actions required for that run. A click can mutate
state despite having an innocent command name, so step-level mutation policy
always remains a core responsibility. Crawlson fails closed if either layer
cannot represent the declared policy.

## Fallback trigger

Rust remains the choice unless an implementation spike demonstrates that the
supported CLI cannot reliably provide one of these required properties from a
Rust child process: versioned structured I/O, isolated lifecycle management,
bounded cancellation, required evidence capture, or a portable install on the
supported release platforms. A preference for faster prototyping or a browser
action benchmark alone is not sufficient.

If such a gap exists and a Python spike demonstrably resolves it through a
maintained public interface, use Python for the affected runtime boundary.
Choose TypeScript only if Python also fails and the required maintained public
interface is available only to the JavaScript ecosystem. In every case, keep
the same driver-neutral journey, evidence, finding, and report contracts.

## Consequences

- The first implementation can ship as a small native CLI with explicit types
  for safety policy, driver failures, artifacts, and run outcomes.
- `agent-browser` remains independently installable and upgradeable; Crawlson
  can add drivers without changing public journey or report models.
- The implementation must maintain protocol fixtures and compatibility tests
  for every supported `agent-browser` release line.
- Rust compile time and cross-platform release automation become early project
  costs.
- Journey schema, agent-runtime selection, CLI upgrade behavior, and release
  channels remain separate decisions and implementation cards.
