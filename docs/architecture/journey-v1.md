# Read-only journey and report contract v1

Crawlson 0.2 introduces the first executable journey contract. It is a narrow,
deterministic slice of the larger agent-driven product:

> Journey -> agent-run browser session -> evidence -> findings and guides

The v1 document is TOML and rejects unknown fields. `schema_version` identifies
the file shape; `journey.revision` is advanced by the journey author when its
meaning changes. Crawlson also records the SHA-256 digest of the source bytes,
so evidence remains attributable even if a revision was not advanced.

See [`schemas/journey-v1.schema.json`](../../schemas/journey-v1.schema.json) and
[`examples/read-only-journey.toml`](../../examples/read-only-journey.toml).

## Deliberately small action set

V1 permits four ordered operations:

- `navigate` resolves one same-origin path and opens it;
- `check_url` compares the current URL with one declared same-origin path;
- `check_text` performs an exact or substring visible-text checkpoint; and
- `capture` reads a target box and captures evidence without interacting with
  the target.

A document must contain at least one `check_url` or `check_text` checkpoint and
at least one `capture`; a navigate-only document cannot pass. `check_text` and
`capture` require the selected element to be visible, and expected text must be
nonblank. After a deterministic checkpoint is false, Crawlson continues the
remaining declared read-only steps so later evidence requests are retained.
Safety blocks and driver/evidence errors stop further journey steps.

There is no generic click, form input, script evaluation, shell command,
download, upload, authentication execution, or mutation capability. A button
or text input may be the visual target of `capture`, but Crawlson does not
activate or alter it. This distinction prevents an innocent-looking browser
verb from silently mutating application state.

All v1 steps are deterministic. Agent-selected actions and subjective findings
will use the same validated core and report model in a later contract.

## Authorization and exact origins

The journey declares one root HTTP(S) origin. Every run separately requires
`--allow-origin`; the normalized scheme, ASCII hostname, and effective port
must match exactly. Crawlson rejects unsafe documents before browser launch,
checks commanded URLs before dispatch, and checks the observed URL before and
after each step. An observed redirect outside the origin is `blocked`, and no
later journey step runs.

The adapter also passes the hostname to agent-browser's domain filter. That is
defense in depth, not the source of truth: agent-browser 0.26 filters by
hostname and therefore cannot prevent the first contact made by a redirect to
another scheme or port on the same hostname. Crawlson detects that state and
stops immediately. Deployments requiring a pre-contact network boundary must
also enforce egress policy outside the browser.

An `[authentication]` table declares a requirement without containing secrets.
Because authentication adapters are outside the 0.2 scope, its presence
produces `blocked/authentication_unavailable` before browser launch.

## Driver and evidence lifecycle

The agent-browser adapter uses a generated owned session and direct argument
arrays. It loads a Crawlson-owned empty configuration plus a default-deny action
policy allowing launch plus only the exact raw v0.26 observation/evidence
actions needed by the run. Crawlson verifies both owned files again immediately
before launch. Inherited agent-browser, profile, provider, proxy, state, and AI
settings are removed. Every child has asynchronously drained, independently capped stdout
and stderr plus a deadline below the driver's 30-second IPC limit. Normal
execution also has an overall deadline. Trace, diagnostics, and close receive
bounded cleanup grace, and a 60-second daemon idle reaper limits orphan lifetime
after an abrupt interruption. Operating-system signal handling that can still
emit a final report remains an explicit post-0.2 task.

A required, nonempty trace starts before initial navigation and its event count
must agree with the trace document. Each `capture` first verifies visibility,
then binds adjacent bounding-box and screenshot commands into one capture token.
It retains the raw viewport PNG, recorded CSS box, and an offline derivative.
The derivative uses `focus-overlay-v1`: a translucent near-black mask surrounds
the padded target, and a vivid red rectangle outlines the action area. Raw bytes
remain authoritative and unchanged; its digest is checked again before rendering.
Sidecar metadata records capture command provenance, coordinate conversion,
clipping, colors, widths, pinned encoder settings, source and derivative
digests, and bounded accessible alt text. The renderer rejects a PNG whose
dimensions disagree with the confirmed viewport device scale.

Agent-browser 0.26 does not expose an atomic page-identity-plus-box-plus-image
operation. Crawlson makes the box and screenshot commands adjacent and checks
the URL immediately before and after the step, but asynchronous same-origin
navigation or layout movement can still make a target/image pair incomplete.
The raw image and capture provenance remain authoritative; Crawlson never
silently substitutes a guessed box.

Artifacts are accepted only when their canonical paths remain beneath the
fresh run directory. HAR remains disabled because it can expose cookies,
headers, request bodies, and responses.

## Outcomes and stable exits

| Outcome | Exit | Meaning |
| --- | ---: | --- |
| `passed` | 0 | Every declared step and required evidence phase completed, including cleanup. |
| `failed` | 1 | The driver operated correctly, but a deterministic checkpoint was false. |
| usage | 2 | Command-line parsing failed; no run report is promised. |
| `blocked` | 3 | Authorization, authentication, or a safety precondition prevented safe continuation. |
| `error` | 4 | The document, driver, protocol, timeout, artifact, trace, diagnostics, or cleanup failed. |

Every parsed `run` request emits one report object in JSON mode conforming to
[`run-report-v1.schema.json`](../../schemas/run-report-v1.schema.json), including
preflight blocked/error results. `execution_outcome` preserves the primary
journey result and `execution_reason` preserves its reason when evidence
finalization or cleanup later changes the final outcome/reason to `error`.
Driver command `upstream_success` records only generic envelope acceptance;
capability validation and the enclosing step/run outcome remain authoritative.
Missing authentication and cleanup failures can never become a green result.
