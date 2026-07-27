# Offline render contract v1

Crawlson 0.3 consumes one completed read-only run without launching a browser:

> verified journey + run report + evidence -> findings or guide

The command is `crawlson render RUN_DIRECTORY --journey JOURNEY`, with identical
`clson` behavior. Rendering is local and offline. The run directory supplied on
the command line is the only filesystem authority; the absolute directory saved
in `report.json` is informational so an archived run may be moved safely.

## Validation boundary

Before creating output, the renderer:

1. strictly decodes bounded run-report v1 and journey v1 or v2 documents;
2. requires the journey source digest, ID, revision, and normalized origin to
   match the run provenance;
3. matches executed steps to the journey by contiguous sequence, ID, title, and
   action kind, with outcome-specific observation checks;
4. verifies required diagnostics and trace evidence for completed execution;
5. rejects duplicate or dangling artifact relationships, absolute/nonportable
   paths, `.`/`..`, backslashes, empty components, controls, and symlinks;
6. reads and rehashes every artifact through one bounded file handle; and
7. strictly checks each focus sidecar against the capture observation, declared
   alt text, command token/sequences, box/viewport geometry, raw source,
   derivative, colors, and pinned encoder settings.

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
passed, every journey step was recorded as passed, and at least one capture step
has both `guide_instruction` and a complete focused evidence chain. An
instruction on a non-capture or missing focused evidence makes the guide
non-publishable; it is never silently omitted.

Read-only capture proves that an action area was visible. It does not prove a
click, input, or submission occurred, so v1 guide output labels authored text as
the reader's next action rather than an executed action.

Findings are eligible only when final and execution outcomes are `failed`, the
reason is a deterministic checkpoint failure, cleanup/evidence completed, and
every non-checkpoint step passed. Each false URL or visible-text checkpoint
becomes one ordered finding. Severity is `untriaged` with source
`not_assessed`; v1 has no evidence from which to infer business impact.
Reproduction includes only the executed prefix through the false checkpoint.
Every finding references the verified report and trace. A focused screenshot is
included only through a digest-verified journey-v2 capture `evidence_for` link
to that earlier checkpoint.

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

Schemas are published at
[`render-report-v1.schema.json`](../../schemas/render-report-v1.schema.json) and
[`findings-v1.schema.json`](../../schemas/findings-v1.schema.json).
