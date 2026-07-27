# Reference Project guide lifecycle inventory

- Status: source-system case study, not a Crawlson architecture specification
- Reviewed: 2026-07-26

## Purpose and scope

This inventory reduces the Reference Project guide system to the behavior Crawlson
needs to preserve or replace. It follows the lifecycle from definition through
publication, traces three representative workflows, and identifies the
smallest independently useful Crawlson slice.

The review intentionally does not select a language, framework, package layout,
or hosted-service model. It also does not copy Reference Project credentials,
user identities, production data, or application-specific fixtures.

The review covered the guide-authoring skill and planning documents, published
wiki material, paired capture/verification suites and their safety controls,
and the publication pipeline. Exact private paths, fixture identities, routes,
counts, and application data are intentionally omitted. The system contains
enough paired specifications, Markdown, and images that duplication and
implicit state are architectural concerns rather than isolated cleanup work.

## Current lifecycle

| Phase | What the Reference Project does now | Consequence for Crawlson |
| --- | --- | --- |
| Define | An author reads application code and writes a Playwright capture spec, a separate Playwright verification spec, and reader-facing Markdown. The goal document defines style and safety conventions, but there is no executable journey record shared by all three artifacts. | One versioned journey must be the source for execution, evidence, findings, and optional guide steps. |
| Authenticate | Browser setup may obtain a testing session from environment-supplied secrets. Specs then sign in disposable role users through application-specific helpers. Many authenticated specs skip when the secret or fixture is unavailable. | Authentication belongs behind an adapter, but its requirement and outcome belong in the core result model. Missing required authentication must produce `blocked` or `error`, never a green skip. |
| Execute | Capture and verification run through separate Playwright configs. Capture is sequential and deterministic. Routine verification uses Playwright label assertions as a stand-in for the guide-driven agent-browser pass; agent-browser also appears in manual and auxiliary capture workflows. | Use `agent-browser` for the first Crawlson driver, behind a replaceable boundary. A run should execute one journey representation once rather than paired hard-coded paths. |
| Verify | Verification specs open live pages and assert the visible labels named by a guide. The verify config retries once and disables screenshots, video, and traces. It emits Playwright pass/fail/skip output, not Crawlson-style outcomes or findings. | Checkpoints should be attached to executed steps, failures should retain first-attempt evidence, and terminal outcomes must include `passed`, `failed`, `blocked`, and `error`. |
| Capture | Capture specs call a shared spotlight helper that waits for UI settling, injects a dim/ring overlay, and writes deterministic PNG paths beside the guide. Playwright's own trace and failure capture are disabled. | Evidence capture is reusable, but spotlight styling and output layout are renderer policy. Core should retain ordinary screenshots and a trace or equivalent before guide-specific annotation. |
| Render | Reader prose is manually authored after capture. Separately, application publication code parses Markdown, rewrites links and image URLs, renders HTML, and hashes the Markdown plus referenced images. | Generating a guide is an optional renderer over completed, verified steps. Crawlson must not infer that a documented final action ran merely because its button was visible. |
| Index | Authors manually link guides from topic `index.md` files and link topics from the master index. Deploy-time ingestion derives ordering and stable slugs from those files and prunes removed database rows. | Index generation and stale-artifact checks can be renderer concerns; journey identity and lifecycle status remain core data. |
| Publish | Deployment copies the wiki tree. Application startup runs a hash-gated content sync, then serves role-filtered guide HTML and assets. Sync failure is explicitly non-fatal. | Hosted publication is an adapter outside the first slice. A publication failure must still be visible in a publication result if Crawlson later owns that operation. |

The durable drift skill runs the two Playwright suites locally against stage.
No reviewed GitHub Actions workflow makes guide verification a required signal.

## Representative workflow traces

### 1. Public, read-only onboarding workflow

The representative verifier visits a public onboarding route, uses a sanitized
fixture state to reach later screens, and checks visible labels plus the final
commit control without activating it. This is safely read-only and needs no
authentication.

The capture specification follows a separate hard-coded path. Its safe default
stops before submission, while an opt-in variant may perform the final action
against an authorized non-production fixture to capture the success screen.
The published guide can include that outcome even when the current verification
did not perform it.

What it demonstrates:

- visible-label checkpoints are useful deterministic observations;
- a safe public journey is a good first executable fixture;
- paired capture and verify definitions can drift; and
- seeing a commit button is not evidence that the stated user outcome occurred.

### 2. Authenticated, role-specific read-only workflow

The representative workflow signs in as multiple disposable roles and checks
that each reaches its expected landing view. One capture path also opens a
non-mutating dialog without submitting it. The visible behavior is read-only
and role-dependent.

The suite self-skips if its identity-provider test secret is missing.
Authentication uses Reference Project-specific session and disposable-user
helpers. Browser-suite output can therefore be green while no authenticated
landing view was exercised.

What it demonstrates:

- role and authentication requirements are journey inputs;
- authentication implementation is application-specific;
- a missing required capability needs an explicit blocked result; and
- a report must say which role and checkpoints actually ran.

### 3. Mutating disposable fixture with cleanup

The representative capture walks a visible create dialog and stops before
saving. To reach an edit state for evidence, its fixture layer creates a
temporary record, returns to the UI, captures the edit control, and deletes the
record in a guaranteed cleanup block.

This fixture is disposable and cleanup is attempted even after capture fails.
However, the representative workflow does not consistently call the available
exact-target mutation guard. Setup and cleanup also bypass the visible UI, and
cleanup is not represented as a named phase in a structured report.

What it demonstrates:

- fixture setup and cleanup are first-class phases distinct from user steps;
- cleanup must remain visible and may not erase the original failure;
- every mutation needs centrally enforced target and capability checks; and
- application fixtures may use private APIs, but journey success must still be
  based on observable user behavior through the UI.

## Reusable concepts and application-specific mechanics

| Keep as an application-independent contract | Keep behind an adapter or leave in the Reference Project |
| --- | --- |
| Journey identity, version, purpose, and expected user outcome | Reference Project routes, labels, roles, and page-specific locators |
| Explicit target plus exact allowed origins | The Reference Project stage hostname and environment naming |
| Authentication requirement and requested role | Identity-provider test sessions and disposable identities |
| Ordered user actions and observable checkpoints | Playwright fixtures and Reference Project sign-in helpers |
| Deterministic versus judgment-based step policy | Reference Project API response shapes and database fixture details |
| Read-only default and per-step mutation declaration | The existing opt-in environment flags |
| Fixture setup, cleanup, and cleanup outcome | Direct Reference Project fixture API calls |
| `passed`, `failed`, `blocked`, and `error` outcomes | Playwright pass/fail/skip as the public result model |
| Evidence references, timing, findings, and reproducible steps | Application-specific spotlight styling and wiki image paths |
| Journey/run provenance for every artifact | Application guide tables, role grouping, and deploy-time ingestion |
| Report schema and exit-code contract | The Reference Project deployment and hosting topology |

## Failure modes Crawlson must address deliberately

1. **Separate sources drift.** Guide prose, capture actions, and verification
   assertions encode the same workflow independently.
2. **Missing authentication looks green.** Authenticated specs self-skip rather
   than fail closed or report a blocked run.
3. **Mutation policy is uneven.** An exact-stage guard exists but is imported by
   only some mutating helpers and specs.
4. **Evidence is incomplete.** Routine verification disables screenshots and
   traces, and retries do not establish a durable first-attempt evidence record.
5. **Status is implicit.** Playwright status, backlog notes, creation logs, and
   guide prose collectively imply completion; no run model distinguishes
   blocked, incomplete, unauthenticated, cleanup-failed, or publication-failed.
6. **Guide success can exceed executed success.** Some guides describe the final
   mutation while routine capture and verification only prove the button exists.
7. **Verification is not a required delivery signal.** The drift skill is an
   on-demand workflow rather than a CI requirement.
8. **Publishing can silently retain stale content.** Deploy startup treats guide
   sync failure as non-fatal so the application can remain available.

## Smallest independently useful Crawlson slice

The first executable slice should be narrower than the full public MVP:

1. Accept one versioned, public, read-only journey against an explicitly
   allowlisted local or staging origin.
2. Validate safety and journey structure before launching a browser.
3. Execute visible UI actions through an `agent-browser` driver adapter.
4. Record every attempted action and checkpoint in order.
5. Retain at least one screenshot plus a browser trace or equivalent evidence,
   including partial evidence on failure.
6. Produce deterministic JSON and a concise Markdown report with one of
   `passed`, `failed`, `blocked`, or `error`.
7. Exit non-zero for `failed` and `error`.

Authentication, mutations, fixture APIs, autonomous exploration, guide
generation, CI summaries, and hosted publication are intentionally outside
this first slice. The later public MVP can add them without changing the core
journey or run result.

### Core responsibilities

- validate the versioned journey and report contracts;
- enforce target, redirect, and mutation policy independently of the driver;
- orchestrate steps, checkpoints, cancellation, and terminal outcomes;
- preserve ordered events, findings, evidence metadata, and provenance; and
- define stable report data and exit-code semantics.

### Replaceable boundaries

- **Browser driver:** `agent-browser` first; later drivers implement the same
  navigation, interaction, observation, and evidence operations.
- **Authentication provider:** supplies a session or returns an explicit blocked
  or error result without exposing secrets.
- **Fixture provider:** prepares and cleans disposable application state under a
  declared mutation capability.
- **Agent runtime:** proposes actions or observations when a journey permits
  judgment; deterministic policy remains in core.
- **Artifact store:** local filesystem first, with hosted storage later.
- **Report and guide renderers:** JSON and local Markdown first; optional guide,
  PR, and hosted-publication renderers later.

## Follow-up decisions

The next architecture request should select the MVP language/runtime and use
this boundary as an evaluation constraint. After that decision, the first
implementation request can define the minimal journey and run-report schemas,
add a self-contained demo target, and implement the read-only vertical slice.
