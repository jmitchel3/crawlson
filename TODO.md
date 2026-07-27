# Crawlson implementation guide

This is a starting backlog, not a frozen specification. Update it as the design
becomes concrete. Preserve the product contract in `README.md`: Crawlson runs
real user journeys, reports evidence-backed bugs, and may render successful
journeys as guides.

## First milestone: one honest vertical slice

Build the smallest end-to-end run before designing a broad platform.

A user should be able to define one deterministic journey, run it against an
explicitly allowlisted local or staging target, and receive:

- an unambiguous result (`passed`, `failed`, `blocked`, or `error`);
- the browser actions and observations in execution order;
- a screenshot and browser trace or equivalent evidence;
- a structured machine-readable report;
- a concise human-readable report with reproducible steps; and
- a non-zero process exit when the journey fails or the runner errors.

The first slice does not need autonomous exploration, hosted infrastructure, or
automatic code repair. It does need to prove that an agent can use a real UI and
produce a trustworthy result. Use `agent-browser` for this first execution path.

## Phase 0: understand the source system

- [x] Inventory the Reference Project guide lifecycle: define, authenticate, execute,
      verify, capture, render, index, and publish.
      See [`docs/architecture/reference-project-guide-lifecycle.md`](docs/architecture/reference-project-guide-lifecycle.md).
- [x] Trace three representative journeys:
  - [x] one read-only workflow;
  - [x] one authenticated, role-specific workflow; and
  - [x] one mutating workflow with fixture setup and cleanup.
- [x] Record what is genuinely reusable versus Reference Project-specific.
- [x] Preserve a few sanitized examples as design fixtures; do not copy secrets,
      production identifiers, or customer data.
- [x] Write a short architecture decision describing the chosen MVP language and
      runtime. Rust was selected with a process-level `agent-browser` boundary;
      see [`ADR 0001`](docs/architecture/decisions/0001-rust-runtime-and-agent-browser-boundary.md).

## CLI and release foundation

- [x] Start the Rust package at 0.1.0 with `Cargo.toml` as the only product
      version source.
- [x] Provide `crawlson` and the equivalent `clson` launcher.
- [x] Add human and JSON `version` and `doctor` output, including a strict
      `agent-browser >=0.26.0,<0.27.0` probe.
- [x] Add `crawlson upgrade` and `clson upgrade` with signed immutable release
      metadata, exact managed-install ownership, stable-only version policy,
      verified atomic replacement on supported Unix installs, a fail-closed
      Windows installer requirement, and package-manager guardrails.
- [x] Add weekly jittered background updates for managed installs, including
      offline, CI, privacy, and policy opt-outs. Development builds remain
      disabled until the release public key and signed assets exist.
- [x] Add cross-platform CI with a stable aggregate `CI` check.
- [x] Define the 0.9.0 four-target bundle, signed release inventory, raw-payload
      update manifest, managed installer, and non-publishing dry-run contracts.
- [x] Prove a clean managed install and packaged demo-application HTTP startup
      from every 0.5.1 dry-run bundle without using a production key or
      publishing a release.
- [ ] Add a durable installer transaction journal and deterministic crash
      recovery before claiming rollback across process or machine termination.
- [ ] Re-prove the clean managed install and complete packaged authenticated
      mutation-and-guide demo from every 0.9.0 dry-run bundle.
- [ ] Publish signed, immutable 0.9.0 bundles and raw update payloads after the
      license, namespace, and production signing-key decisions are complete.

## Phase 1: define the contracts

- [x] Define a versioned journey schema that can express:
  - [x] journey identity, purpose, and expected user outcome;
  - [x] target and exact authorized origin;
  - [x] user role and authentication requirements without embedding secrets;
  - [x] ordered read-only actions and observable checkpoints;
  - [x] one explicitly authorized, deterministic same-origin link action;
  - [x] whether each step is read-only or mutating;
  - [x] fixture setup and cleanup requirements;
  - [x] evidence to retain; and
  - [x] guide-facing titles, bounded instructions, and explicit checkpoint
        evidence associations when applicable.
- [x] Keep every v1 step deterministic; add agent judgment only through a
      later validated contract.
- [x] Define run outcomes. At minimum: `passed`, `failed`, `blocked`, and
      `error`. Never encode missing credentials as a pass or an invisible skip.
- [x] Define a stable report schema before polishing terminal output.
- [x] Define provenance so every screenshot and executed step can be
      traced to a particular run and journey version.
- [x] Supply bounded exact-origin browser storage through an external private file,
      without retaining its path or contents in journey files, reports, command
      provenance, screenshots, traces, or generated guides. See
      [`docs/architecture/authentication-v1.md`](docs/architecture/authentication-v1.md).
      Cookie import remains fail-closed until the driver boundary can enforce
      exact scheme, host, and port for every request.

## Phase 2: build the safe runner

- [x] Add a CLI that validates a journey before launching a browser.
- [x] Add exact-origin authorization and stop after an observed unauthorized
      redirect, while documenting the hostname-only driver limitation.
- [x] Make every v1 run read-only with no generic click, input, script, or
      mutation capability.
- [x] Add a v3 `follow_link` capability with an exact per-step runtime grant,
      pre-action focused evidence, one non-retried click, and exact same-origin
      postcondition verification.
- [x] Require an explicit mutation capability for journeys that change data.
- [x] Refuse mutating production runs unless the exact target and operation were
      explicitly authorized.
- [x] Add the first replaceable authentication provider for strict external
      `agent-browser` state files and visible role verification.
- [x] Implement the first runner with `agent-browser` behind a documented,
      replaceable execution boundary.
- [ ] Capture step timing, navigation, console errors, failed requests, and
      browser evidence without leaking secrets.
  - [x] Capture step timing, redacted navigation, console/page-error summaries,
        and browser evidence.
  - [ ] Add failed-request capture with an explicit secret-redaction contract.
- [ ] Implement bounded retries that preserve the original failure evidence.
  - [x] Never retry a mutation after dispatch; an uncertain effect is an error
        followed by cleanup and recovery handling.
- [x] Implement cleanup as a reported phase; cleanup failure must remain visible.
- [x] Apply per-command and overall run deadlines, bounded evidence/cleanup
      grace, and a daemon idle reaper.
- [ ] Handle operating-system cancellation signals and process crashes without
      falsely reporting success. V5 handles normal interrupt signals by entering
      cleanup and leaves a durable exact-origin recovery barrier after an
      interrupted or unknown mutation. The exact journey can visibly recover a
      forced-termination barrier, but the terminated process still cannot emit
      its own final report.

## Phase 3: turn runs into findings

- [x] Separate deterministic assertion failures from infrastructure
  errors.
- [x] Give every deterministic finding an explicitly untriaged severity, concise description, evidence references,
  and reproducible steps.
- [ ] Deduplicate repeated symptoms within a run without hiding independent
      failures.
- [x] Preserve partial evidence for blocked and failed journeys.
- [x] Preserve an `untriaged/not_assessed` review state; subjective agent
      observations remain outside v1.
- [x] Produce deterministic JSON output suitable for CI and integrations.
- [x] Produce readable local findings that link directly to verified artifacts.

## Phase 4: generate honest guides

- [x] Render guide steps from the same executed journey representation used for
      verification; do not maintain a second hard-coded click path.
- [x] Include only steps that were actually completed and verified.
- [x] Represent blocked, text-only, incomplete, and conflicting guide output explicitly.
- [x] Make image naming and renderer output deterministic and idempotent.
- [x] Render each requested action screenshot with a reproducible red target outline
      and translucent near-black surrounding mask while preserving the raw
      screenshot as authoritative evidence.
- [x] Detect orphaned images, dead Markdown links, missing index entries, and
      stale generated output through a read-only collection audit.
- [x] Provide a versioned, neutral guide-collection JSON boundary and Markdown
      wiki adapter without coupling application presentation to the runner.

## Phase 5: regression and CI workflow

- [ ] Replay a previously successful journey and explain meaningful differences.
- [ ] Distinguish expected UI evolution from a broken user outcome.
- [x] Define stable exit-code behavior for local use and CI.
- [x] Publish a CI example that cannot turn missing target, action, or
      authentication-state authorization into green; authenticated journeys
      must visibly verify the declared role before evidence capture.
- [x] Upload reports and evidence even when the required real-browser job fails.
- [ ] Add a pull-request summary adapter only after the local workflow is sound.
- [ ] Keep PR execution, scheduled staging runs, and interactive local runs as
      adapters around the same core.

## Phase 6: open-source readiness

- [ ] Choose the public license before publishing artifacts.
- [ ] Reserve appropriate package and repository namespaces.
- [ ] Add contribution, security, and responsible-testing guidance.
- [ ] Document supported targets and the authorization model prominently.
- [x] Provide a self-contained demo application and journeys that require no
      third-party credentials.
- [x] Add unit tests for schema and safety policy plus end-to-end tests for pass,
      fail, blocked, error, mutation denial, and cleanup failure.
  - [x] Cover the read-only pass/fail/blocked/error and cleanup contracts with a
        fake process and a required-in-CI real agent-browser loopback fixture.
  - [x] Cover authorized link pass, preflight mismatch, missing grant,
        off-origin block, and uncertain post-dispatch action state.
  - [x] Cover authenticated pass, missing/unsupported/invalid state, driver-load
        failure, visible verification failure, temporary cleanup, and retained
        output privacy with fake and real browser drivers.
  - [x] Cover missing, malformed, duplicate, extra, and production mutation
        grants before browser launch, plus unexpected mutation flags on legacy
        journeys.
  - [x] Prove with real Chromium that the v5 guard permits the exact-origin
        mutation while blocking cross-port `fetch`, `sendBeacon`, and WebSocket
        traffic to a sensitivity-checked loopback trap.
- [x] Prove that a clean source build can run the demo and reproduce the
      documented artifacts.
- [ ] Prove that a clean managed installation from each non-publishing dry-run
      bundle can run the packaged demo and reproduce the documented artifacts.
- [ ] Publish signed bundles only after that installed dry-run proof passes and
      the owner-gated license, namespace, and production-key work is complete.

## Decisions to make deliberately

- [x] Rust for the MVP core and CLI, with documented Python-then-TypeScript
      fallback triggers in
      [`ADR 0001`](docs/architecture/decisions/0001-rust-runtime-and-agent-browser-boundary.md)
- [x] A process-level, typed-capability `agent-browser` adapter for the MVP;
      consider a broader driver protocol only when a second driver requires it
- [x] Strict declarative TOML v1 with a published JSON Schema
- [x] Require validated declarative actions plus exact runtime grants before
      exposing the minimum driver capability; agent-proposed actions remain a
      later contract.
- [ ] Local model, hosted model, and model-provider abstraction boundaries
- [ ] Baseline/diff strategy for UI changes and nondeterministic content
- [ ] Artifact storage and redaction policy
- [ ] Accessibility, console, network, and visual checks in the default run
- [x] Fixture lifecycle and same-origin mutation isolation through explicit
      setup/main/cleanup phases, public per-run fixture values, and a durable
      exact-origin recovery barrier
- [x] First application-neutral extension point for external authentication
      state and visible role verification; login and hosted-secret providers
      remain future contracts. V5 fixture setup and cleanup use the visible UI
      and stay independent of application-specific fixture adapters.

## MVP definition of done

The first public MVP is done when a new contributor can:

1. install Crawlson from a clean environment;
2. start the included demo application;
3. run one passing and one intentionally failing user journey;
4. inspect structured results, screenshots, and reproducible failure steps;
5. generate a guide from the passing journey;
6. observe a missing credential or unsafe target fail closed; and
7. run the same checks in CI with documented, predictable exit codes.

No step in that demonstration may depend on the Reference Project or private
credentials.

A non-publishing dry run can prove this workflow from an extracted bundle and a
clean managed prefix. The public-MVP definition of done nevertheless remains
open until a new contributor can obtain and authenticate an owner-approved,
signed public release; CI artifacts signed by test-only keys do not satisfy the
installation requirement.
