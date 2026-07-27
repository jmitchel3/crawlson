# Disposable mutation contract v1

Crawlson 0.9 adds the first deliberately narrow mutating journey. The public
model remains:

> journey -> agent-run browser session -> evidence -> findings and guides

Journey schema v5 and run-report schema v4 add visible-UI fixture setup,
mutation, and cleanup without turning the browser adapter into a general-purpose
automation API. Journey schemas v1 through v4 remain unchanged: v1 and v2 are
read-only, while v3 adds one narrowly authorized link action and v4 adds
authentication to that same bounded action model.

## Scope

The v1 mutation contract supports one disposable authenticated actor, one
`self_expiring_ui` fixture, ordinary text input, exact POST form submission, and
idempotent visible-UI cleanup. A journey declares three ordered phases:

1. `setup_steps` are read-only. They must visibly verify the declared disposable
   actor and prove that the fixture begins absent.
2. `steps` contain the user-facing workflow. Every step declares `effect =
   "read_only"` or `effect = "mutating"`; the classification must agree with
   its action type.
3. `cleanup_steps` are always separate from the main result. They include an
   idempotent `ensure_absent` action which either observes the fixture already
   absent or removes it through one bounded form submission and verifies the
   absence marker.

The fixture also declares a maximum lifetime from 1 to 3,600 seconds. That
self-expiry is a backstop, not evidence of cleanup. Crawlson reports cleanup as
passed only after the declared visible UI proves absence.

The generated `$fixture_token` is intentionally public fixture data. It is not
a credential and may appear in form values, screenshots, traces, or server
logs. Journey authors must never use this field for secrets or customer data.
Authentication remains the separate state-file contract introduced with
journey v4: the source is private, the actor must be disposable, and visible
role verification must pass before any mutation.

## Authorization

The target origin and journey declaration are not authorization. Before opening
authentication state or starting a browser, `crawlson run` requires:

- one exact `--allow-origin SCHEME://HOST:PORT` grant;
- one exact `--allow-mutation JOURNEY@REVISION:STEP` grant for every mutating
  main and cleanup step; and
- for every non-literal-loopback target, the same complete set repeated as
  `--allow-production-mutation JOURNEY@REVISION:STEP`.

Supplied sets must be valid, unique, complete, and contain no unexpected
entries. The report binds required and granted sets to the journey digest,
revision, normalized exact origin, and production classification. A normal
`--allow-action` grant cannot authorize a mutation. Literal-loopback runs reject
production grants rather than silently ignoring them.

The extra production grant records deliberate confirmation; it does not make a
target safe, disposable, or suitable for testing. Operators remain responsible
for authorizing that exact target and operation. Crawlson never weakens
authentication, authorization, anti-abuse, or application security controls to
make a journey pass.

## Exact-origin network enforcement

`agent-browser 0.26` exposes a hostname allowlist, which is too broad for an
exact scheme/host/effective-port mutation boundary. A v5 run therefore requires
an explicit `--browser-executable` pointing to a regular, non-symlink Chromium
or Chrome for Testing executable. Branded Chrome and an implicit driver-selected
browser are rejected because loading an unpacked guard extension cannot be
relied on there.

Crawlson materializes an owned Manifest V3 Declarative Net Request extension in
the run's private control directory. Its higher-priority rule allows the exact
HTTP(S) scheme, host, and effective port; its default rule blocks every other
HTTP, HTTPS, WebSocket, and WebTransport request across all Chrome resource
types. WebSockets remain blocked even when their hostname and port otherwise
match. The driver passes this extension and explicit browser executable on each
`agent-browser` command.

The extension also injects a per-run marker only when `location.origin` is the
normalized target. Crawlson verifies the materialized extension bytes and
directory shape before launch, then queries that unguessable marker immediately
before each mutating dispatch. The report records this query as the
`exact_origin_guard` capability. A missing marker, changed extension, unsafe
path, unsupported browser, or failed query stops before the mutation.

This is layered with current-URL checks, exact form inspection, the driver
action policy, and post-action origin checks. It is an enforcement and
attestation boundary for browser network requests, not a claim that arbitrary
page script or browser extensions are generally trustworthy.

## Deterministic actions and evidence

`fill_text` accepts one simple `#id` selector and only `$fixture_token`. Crawlson
requires exactly one ordinary text input, checks visibility and enabled state,
captures the field, fills once, reads the value back, and verifies that both the
value and current URL are unchanged.

`click_button` accepts simple `#id` selectors for one form and its submit
button. Before one click it requires exactly one matching form/button pair, a
visible enabled submit control, `method=POST`, the declared exact same-origin
action, no `formaction` or `formmethod` override, and no new browsing context.
After the click it verifies the exact declared URL plus visible deterministic
text.

`ensure_absent` first checks the declared absence marker. If cleanup is already
complete it records a verified effect without clicking. Otherwise it applies
the same exact POST preflight, one-click rule, and visible postcondition as
`click_button`.

Every dispatched fill or click is preceded by a raw viewport screenshot. The
deterministic derivative places a vivid red outline around the exact input or
button and a translucent near-black mask over the surrounding page. The box,
viewport, raw image, derivative, encoder settings, and adjacent command
sequences are digest-bound in focus metadata. These images focus review on the
action area while preserving the raw screenshot as authoritative evidence.

Mutation commands are never retried. The state machine distinguishes:

- `not_attempted`: preflight rejected the operation;
- `driver_acknowledged`: the driver accepted it but postconditions are not yet
  established;
- `effect_verified`: the declared effect was independently observed;
- `effect_unverified`: the operation completed but a deterministic
  postcondition was false; and
- `effect_unknown`: dispatch may have occurred, but Crawlson cannot establish
  its effect.

Only `effect_verified` can support an executed guide claim. A deterministic
`effect_unverified` checkpoint may support a finding after cleanup succeeds.
`effect_unknown` is an error, is never retried, and forces cleanup plus recovery
handling.

## Cleanup, recovery, and outcome precedence

Immediately before main mutations, Crawlson creates a durable, non-secret
recovery barrier keyed by the normalized exact origin. The authoritative
barrier lives in Crawlson's user-global application state directory (or the
explicit `CRAWLSON_HOME`) rather than beneath `--output-dir`, so changing run
destinations cannot bypass it. The authority is created before its run-directory
copy. It contains journey/run provenance and declared cleanup step IDs, but no
authentication state, fixture value, control token, or provider output.

After setup succeeds, cleanup is attempted after a main pass, deterministic
failure, block, error, timeout, or handled interruption. Cleanup receives a
separate bounded 60-second grace period. The barrier is removed only after
`ensure_absent` visibly verifies absence; removal deletes the run copy first and
the origin authority last. A crash may therefore leave an extra barrier but
must not silently clear one before verified cleanup.

The report preserves `execution_outcome` and `execution_reason` as the main
workflow result. Final `outcome` and `reason` apply this precedence:

1. fixture cleanup failure, unknown cleanup effect, or recovery-finalization
   failure makes the final outcome `error` and leaves `recovery_required =
   true`, while retaining the main execution result;
2. browser evidence/session cleanup failure also makes the final outcome
   `error` without erasing the main result;
3. otherwise the final outcome equals the main `passed`, `failed`, `blocked`,
   or `error` result.

Guides require a passed main workflow, verified mutation effects, verified
fixture cleanup, a cleared recovery barrier, successful trace/diagnostics and
browser-session cleanup, and complete focused evidence. Deterministic mutation
findings require a failed main checkpoint and the same successful cleanup and
evidence conditions. Setup failure, missing grants, missing authentication,
unknown effects, interrupted cleanup, a pending recovery barrier, or any
contradictory report is non-publishable.

If the authority survives a crash or forced termination, rerunning the exact
same journey digest, origin, authentication input, browser selection, and full
mutation grant set enters recovery-only mode. Crawlson visibly verifies the
disposable actor, skips setup assertions that require the fixture to be absent,
executes only the declared cleanup phase, and clears the global authority only
after absence is visibly verified. That invocation returns blocked with
`recovery_completed` and performs no main-journey mutation; a subsequent run is
required to start new work. A mismatched journey or cleanup contract remains
blocked. The original run's recovery marker remains historical evidence even
after a later recovery run clears the global authority.

## Compatibility and boundaries

Journey v5 maps to run-report v4 and mutation findings v3. Earlier journey,
run-report, and findings schemas stay published and continue to render under
their original contracts. Setup and cleanup never contribute guide steps;
guide prose and findings reproduction come from the main journey representation
that was executed.

The core owns journey validation, authorization bindings, lifecycle and outcome
models, evidence provenance, recovery policy, and renderer validation. The
initial `agent-browser` process implementation, state-file authentication,
Chromium extension transport, and Markdown output remain adapters. A later
driver may replace those adapters only if it preserves the same fail-closed
journey and report semantics.

Published schemas:

- [`journey-v5.schema.json`](../../schemas/journey-v5.schema.json)
- [`run-report-v4.schema.json`](../../schemas/run-report-v4.schema.json)
- [`findings-v3.schema.json`](../../schemas/findings-v3.schema.json)
