# Authorized-link journey contract v3

Journey v3 adds one deliberately narrow user interaction to the existing
read-only contract: following a declared same-origin link. It preserves every
v1 and v2 action and validation rule. Older readers continue to reject the new
document version instead of silently treating an interaction as a capture.

The new action is:

```toml
[[steps]]
id = "continue"
title = "Continue to completion"
guide_instruction = "Select the highlighted Continue link."
action = { type = "follow_link", selector = "#continue", expected_path = "/complete", alt_text = "Continue link highlighted in red" }
```

`selector` and `alt_text` use the same bounded validation as a focused capture.
`expected_path` is resolved against the journey target and must identify an
exact same-origin HTTP(S) URL. It cannot contain credentials, a query, a
fragment, a leading authority (`//`), or a backslash. A `follow_link` must have
a nonempty, bounded `guide_instruction`; the action's pre-interaction focused
image is guide evidence for that exact instruction.

## Evidence and checkpoints

A valid v3 journey still requires at least one explicit deterministic
`check_url` or `check_text` step. `expected_path` is the required postcondition
of the link action, but it does not replace a separately declared checkpoint.
This keeps a journey from claiming useful success solely because a driver
acknowledged a click.

V3 also requires at least one focused evidence action. Either `capture` or
`follow_link` satisfies that requirement because link execution captures the
target immediately before interaction. `evidence_for` remains exclusive to
`capture` and may still refer only to unique earlier checkpoints. V3 does not
silently attach a link's action image to an unrelated finding.

## Execution boundary

The document is a declaration, not authorization by itself. The runner must
obtain a separate grant for the exact `follow_link` step, retain the existing
exact-origin grant, and block before browser launch when either is absent. The
runner exposes a typed link capability backed by a narrow action policy. Its
internal click is constrained to the declared CSS selector intersected with an
`a[href]` element and the exact href observed during preflight; buttons and
custom elements cannot satisfy the dispatch selector.

Before interaction, execution must confirm that the target is visible and
enabled and that its actual link destination resolves to `expected_path`.
Execution then preserves the raw viewport image, focused red-box/dimmed image,
focus metadata, and action command provenance before following the link. A
successful step requires the observed post-action URL to equal the resolved
expected URL and remain inside the authorized origin.

A missing, hidden, disabled, or mismatched link is a deterministic failed
outcome when the driver can establish that state. An off-origin destination is
blocked. A timeout or protocol failure after dispatch cannot prove whether the
interaction occurred and must be an explicit error with an unknown action
effect. Such an action must never be retried automatically. Evidence and owned
session cleanup remain required, and cleanup failure cannot become a pass.

Following a link can still invoke application code. The v3 contract does not
authorize form submission, generic buttons, input, authentication, fixture
setup, data mutation, script evaluation, upload, download, or arbitrary agent
actions. Those capabilities require separate versioned safety contracts,
including disposable fixtures and visible application cleanup where data can
change.

`agent-browser 0.26.x` can restrict network activity by hostname but cannot
prevent a transient redirect to another scheme or port on the same hostname.
Crawlson validates exact origins before and after the action and blocks an
observed escape, but does not claim preventive exact-origin interception. Runs
must target environments where that limitation is explicitly acceptable until
a stricter driver boundary exists.

## Compatibility

- Journey v1 keeps its legacy URL behavior and cannot use `follow_link`.
- Journey v2 keeps query/fragment exclusion and `evidence_for`; it cannot use
  `follow_link`.
- Journey v3 uses the same query/fragment-safe paths as v2 and adds only the
  action described here.
- Authentication declarations remain fail-closed until an authentication
  adapter is configured; v3 does not weaken that boundary.

See [`journey-v3.schema.json`](../../schemas/journey-v3.schema.json). The
versioned run-report and renderer contracts must represent action dispatch,
effect verification, unknown effect, and pre-action evidence before a v3 run
can be published as a verified guide.
