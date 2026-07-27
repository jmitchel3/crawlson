# Offline render contract v1

Crawlson 0.3 introduced the offline renderer. Crawlson 0.9 extends the same
command contract through deterministic authenticated mutation runs without
launching a browser:

> verified journey + run report + evidence -> findings or guide

The command is `crawlson render RUN_DIRECTORY --journey JOURNEY`, with identical
`clson` behavior. Rendering is local and offline. The run directory supplied on
the command line is the only filesystem authority; the absolute directory saved
in `report.json` is informational so an archived run may be moved safely.

## Validation boundary

Before creating output, the renderer:

1. strictly decodes bounded run-report v1 through v4 and journey v1 through v5
   documents, requiring the report version associated with that journey;
2. requires the journey source digest, ID, revision, and normalized origin to
   match the run provenance;
3. matches executed steps to the journey by contiguous sequence, ID, title, and
   action kind, with outcome-specific observation checks;
4. verifies required diagnostics and trace evidence for completed execution;
5. rejects duplicate or dangling artifact relationships, absolute/nonportable
   paths, `.`/`..`, backslashes, empty components, controls, and symlinks;
6. reads and rehashes every artifact through one bounded file handle; and
7. for v3 and later, verifies the exact action-grant set and its journey digest,
   revision, origin, and step bindings;
8. for v5, verifies every setup/main/cleanup phase and effect classification,
   the exact mutation and production grant sets, disposable authentication and
   fixture lifecycle, recovery state, and each mutation command's per-step
   binding;
9. requires each dispatched v5 fill or click to have a preceding
   `exact_origin_guard` attestation, adjacent pre-action screenshot, correct
   driver capability, and unique command reference; and
10. strictly checks each focus sidecar against the capture observation, declared
   alt text, command token/sequences, box/viewport geometry, raw source,
   derivative, colors, and pinned encoder settings. Executed link evidence also
   requires the click command to immediately follow the screenshot command;
   mutation evidence applies the same adjacency rule to its fill or click.

Component symlinks are rejected even if they resolve back inside the run. The
implementation treats concurrent mutation by another local process during the
short check/open interval as outside the v1 threat model. A future artifact
store can replace this with platform-specific directory-handle/open-beneath
semantics without changing the journey or render models.

The unsigned bundle provides integrity consistency, not authenticity. A party
that can rewrite `report.json` and every artifact can create new matching
hashes. Strong authenticity requires an externally anchored digest or signature
in a later contract.

## Publishability

A guide is eligible only when final and execution outcomes are `passed`, cleanup
passed, every journey step was recorded as passed, and at least one main-journey
evidence action has both `guide_instruction` and a complete focused evidence
chain. Supported guide evidence actions are `capture`, `follow_link`, and, for
v5, `fill_text` and `click_button`. An instruction on another action or missing
focused evidence makes the guide non-publishable; it is never silently omitted.

Read-only capture proves that an action area was visible. It does not prove a
click, input, or submission occurred, so v1 guide output labels authored text as
the reader's next action rather than an executed action.

A v3 link instruction may be labeled executed only when its grant binding,
anchor-and-href-constrained click, adjacent pre-action screenshot, driver
acknowledgement, and exact post-action URL all validate. An unattempted,
unknown-effect, blocked, or mismatched action can never produce that claim.

A v5 mutation instruction may be labeled executed only when authentication was
visibly verified before mutation, every exact grant binding validates, the
guard-attested fill or click immediately followed its focused screenshot, its
deterministic effect is `effect_verified`, fixture cleanup passed, the recovery
barrier was cleared, and browser evidence/session cleanup passed. Setup and
fixture-cleanup steps are never emitted as guide steps. A generated public
fixture token is represented as a value source; its run-specific value is not
copied into guide or findings reproduction output.

Findings are eligible only when final and execution outcomes are `failed`, the
reason is a deterministic checkpoint failure, cleanup/evidence completed, and
every non-checkpoint step passed, except that execution may stop at a failed
`follow_link`. Each false URL, visible-text checkpoint, or deterministic link
failure becomes one ordered finding. Link findings distinguish invisible,
disabled, invalid-target, preflight-destination, and acknowledged postcondition
failures. Severity is `untriaged` with source `not_assessed`; neither schema has
evidence from which to infer business impact.
Reproduction includes only the executed main-journey prefix through the false
checkpoint. Every finding references the verified report and trace. A focused
screenshot is included only through a digest-verified journey-v2 capture
`evidence_for` link to an earlier checkpoint or, for a failed v3 `follow_link`,
through that exact action's verified preflight capture chain.

Findings v3 add main-journey fixture-absence checkpoint, public-fixture input,
mutation-button, and visible postcondition failures. An initial-absence failure
during fixture setup remains blocked and non-publishable. A v5 finding is
eligible only when its main execution ended with a deterministic
`checkpoint_failed`, fixture cleanup passed, `recovery_required` is false, and
the final outcome remains `failed`.
An action with `effect_unknown`, cleanup with a failed or unknown effect,
recovery-finalization failure, or browser cleanup failure makes the final result
an error and produces neither a guide nor a finding. The preserved
`execution_outcome` and `execution_reason` may still describe the earlier main
result; final outcome precedence is defined by the
[`mutation contract`](mutation-v1.md).

Blocked runs are explicit non-publishable results. Error, incomplete,
cleanup-failed, drifted, tampered, missing, or escaping inputs produce no guide
or user-facing finding.

## Deterministic outputs

The renderer builds all bytes in memory, writes a fresh staging directory under
the run root, then renames the complete directory to `render/`. It never changes
`report.json` or `evidence/`. A byte-identical existing render is accepted as an
idempotent no-op; a conflicting directory or symlink is preserved and rejected.

Possible files are:

- `render/render-report.json` for every structurally valid render decision;
- `render/guide.md` plus deterministic focused-image copies for guide-ready
  passes; or
- `render/findings.json` and `render/findings.md` for findings-ready failures.

The result contains no render timestamp, absolute path, target origin, session,
raw driver output, or model-authored explanation. Markdown text is escaped and
artifact links are generated only from validated relative paths. Screenshots
may still contain sensitive UI pixels and are not claimed to be redacted or
automatically safe for public release.

Crawlson 0.7's collection layer does not scrape or trust these persisted
single-run outputs. It snapshots the raw run without `render/`, calls this same
renderer against the snapshot and exact journey source, and composes only the
newly revalidated bytes. See the
[`guide collection contract`](guide-collection-v1.md).

Schemas are published at
[`render-report-v1.schema.json`](../../schemas/render-report-v1.schema.json) and
[`findings-v1.schema.json`](../../schemas/findings-v1.schema.json). Action
findings use
[`findings-v2.schema.json`](../../schemas/findings-v2.schema.json); their input
contract is
[`run-report-v2.schema.json`](../../schemas/run-report-v2.schema.json).
Mutation findings use
[`findings-v3.schema.json`](../../schemas/findings-v3.schema.json), with
[`journey-v5.schema.json`](../../schemas/journey-v5.schema.json) and
[`run-report-v4.schema.json`](../../schemas/run-report-v4.schema.json) as their
input contracts. Authenticated read-only runs continue to use journey v4 and
run-report v3.
