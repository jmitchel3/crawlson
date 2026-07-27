# Crawlson implementation guide

This is a starting backlog, not a frozen specification. Update it as the design
becomes concrete. Preserve the product contract in `README.md`: Crawlson runs
real user journeys, reports evidence-backed bugs, and may render successful
journeys as guides.

## First milestone: one honest vertical slice

Build the smallest end-to-end run before designing a broad platform.

A user should be able to define one read-only journey, run it against an
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
- [ ] Preserve a few sanitized examples as design fixtures; do not copy secrets,
      production identifiers, or customer data.
- [ ] Write a short architecture decision describing the chosen MVP language and
      runtime. Evaluate the existing TypeScript/Playwright implementation, but
      do not choose it by inertia.

## Phase 1: define the contracts

- [ ] Define a versioned journey schema that can express:
  - [ ] journey identity, purpose, and expected user outcome;
  - [ ] target and allowed hostnames;
  - [ ] user role and authentication requirements;
  - [ ] ordered actions and observable checkpoints;
  - [ ] whether each step is read-only or mutating;
  - [ ] fixture setup and cleanup requirements;
  - [ ] evidence to retain; and
  - [ ] guide-facing titles and instructions when applicable.
- [ ] Decide which steps are deterministic and which permit agent judgment.
- [ ] Define run outcomes. At minimum: `passed`, `failed`, `blocked`, and
      `error`. Never encode missing credentials as a pass or an invisible skip.
- [ ] Define a stable report schema before polishing terminal output.
- [ ] Define provenance so every screenshot, finding, and guide step can be
      traced to a particular run and journey version.
- [ ] Decide how secrets and authenticated session state are supplied without
      entering journey files, logs, screenshots, or generated guides.

## Phase 2: build the safe runner

- [ ] Add a CLI that validates a journey before launching a browser.
- [ ] Add exact target allowlisting and reject redirects to unauthorized hosts.
- [ ] Default every run to read-only mode.
- [ ] Require an explicit mutation capability for journeys that change data.
- [ ] Refuse mutating production runs unless the exact target and operation were
      explicitly authorized.
- [ ] Add pluggable authentication/session providers.
- [ ] Implement the first runner with `agent-browser` behind a documented,
      replaceable execution boundary.
- [ ] Capture step timing, navigation, console errors, failed requests, and
      browser evidence without leaking secrets.
- [ ] Implement bounded retries that preserve the original failure evidence.
- [ ] Implement cleanup as a reported phase; cleanup failure must remain visible.
- [ ] Handle cancellation and crashes without falsely reporting success.

## Phase 3: turn runs into findings

- [ ] Separate assertion failures from agent observations and infrastructure
      errors.
- [ ] Give every finding a severity, concise description, evidence references,
      and reproducible steps.
- [ ] Deduplicate repeated symptoms within a run without hiding independent
      failures.
- [ ] Preserve partial evidence for blocked and failed journeys.
- [ ] Add a human review state for subjective usability findings.
- [ ] Produce deterministic JSON output suitable for CI and integrations.
- [ ] Produce a readable local report that links directly to its artifacts.

## Phase 4: generate honest guides

- [ ] Render guide steps from the same executed journey representation used for
      verification; do not maintain a second hard-coded click path.
- [ ] Include only steps that were actually completed and verified.
- [ ] Represent blocked, text-only, incomplete, and retired guides explicitly.
- [ ] Make image naming and replacement deterministic.
- [ ] Detect orphaned images, dead Markdown links, missing index entries, and
      stale generated output.
- [ ] Allow application-specific rendering without coupling it to the runner.

## Phase 5: regression and CI workflow

- [ ] Replay a previously successful journey and explain meaningful differences.
- [ ] Distinguish expected UI evolution from a broken user outcome.
- [ ] Define stable exit-code behavior for local use and CI.
- [ ] Publish a CI example that cannot turn missing credentials into green.
- [ ] Upload reports and evidence even when a run fails.
- [ ] Add a pull-request summary adapter only after the local workflow is sound.
- [ ] Keep PR execution, scheduled staging runs, and interactive local runs as
      adapters around the same core.

## Phase 6: open-source readiness

- [ ] Choose the license after confirming what can be extracted from the Reference Project.
- [ ] Reserve appropriate package and repository namespaces.
- [ ] Add contribution, security, and responsible-testing guidance.
- [ ] Document supported targets and the authorization model prominently.
- [ ] Provide a self-contained demo application and journeys that require no
      third-party credentials.
- [ ] Add unit tests for schema and safety policy plus end-to-end tests for pass,
      fail, blocked, error, mutation denial, and cleanup failure.
- [ ] Publish only after a clean install can run the demo and reproduce the
      documented artifacts.

## Decisions to make deliberately

- [ ] Python, TypeScript, or a split architecture
- [ ] The smallest useful `agent-browser` adapter contract and whether a broader
      driver protocol is warranted later
- [ ] Declarative data format versus code-first journey API
- [ ] How agents propose actions without bypassing deterministic safety checks
- [ ] Local model, hosted model, and model-provider abstraction boundaries
- [ ] Baseline/diff strategy for UI changes and nondeterministic content
- [ ] Artifact storage and redaction policy
- [ ] Accessibility, console, network, and visual checks in the default run
- [ ] Fixture lifecycle and parallel-run isolation
- [ ] Extension points for application authentication and role setup

## MVP definition of done

The first public MVP is done when a new contributor can:

1. install Crawlson from a clean environment;
2. start the included demo application;
3. run one passing and one intentionally failing user journey;
4. inspect structured results, screenshots, and reproducible failure steps;
5. generate a guide from the passing journey;
6. observe a missing credential or unsafe target fail closed; and
7. run the same checks in CI with documented, predictable exit codes.

No step in that demonstration may depend on the Reference Project or private credentials.
