# Render evidence-backed findings and drift-free guides

- Kanbanlan: `KBL-IPJI2QFTJZA6LD4ZU2YLCSSLFM`
- Canonical home: `github`
- Canonical request: [#12](https://github.com/jmitchel3/crawlson/issues/12)

## Request

## Outcome

Deliver Crawlson 0.3.0 rendering over completed 0.2 run evidence. A verified passing run can produce a deterministic Markdown guide; a deterministic failed checkpoint can produce structured, evidence-linked findings. Both must be derived from the executed journey and report, never a second hard-coded path.

## Acceptance criteria

- Add a crawlson/clson render command that accepts a run directory and the journey source, verifies the journey digest/revision against the report, and validates all inputs before writing.
- Generate a guide only from a final passed run and only from executed passed steps that declare guide instructions and have verified focused-image evidence.
- Generate deterministic JSON and Markdown findings for failed checkpoints, with severity/kind, concise symptom, executed reproduction steps, and verified evidence references.
- Treat blocked, error, incomplete, drifted, tampered, missing-artifact, and cleanup-failed runs as explicit non-publishable outcomes; never emit a verified guide or user-facing bug finding from infrastructure failure.
- Rehash every referenced artifact, reject paths escaping the run directory, preserve existing run evidence, and write renderer outputs atomically beneath the run directory.
- Keep report/journey/renderer models versioned and application-independent; do not add authentication, mutation, autonomous exploration, hosted publication, or application-specific fixtures.
- Add pass/fail/blocked/error, source-drift, artifact-tamper, path-escape, deterministic-output, crawlson/clson parity, and Markdown-link tests.
- Update README, TODO, changelog, schemas/examples, and the durable delivery record without private identifiers or fixture details.

## Scope boundaries

Offline local rendering only. No browser execution changes beyond the minimum provenance needed by the renderer; no model judgment, authentication, mutation, auto-repair, hosted service, wiki publication, or production target execution.

## Decisions

- Rendering is an offline, deterministic consumer. The CLI-supplied run root is
  the sole filesystem authority; saved absolute paths are informational so a
  complete run can be moved before rendering.
- Journey v1 remains immutable. Journey v2 adds an explicit, capture-only
  `evidence_for` relation to unique earlier checkpoints. Image evidence is never
  inferred from proximity, titles, or selectors.
- The renderer accepts only strict, versioned inputs and revalidates the full
  executed sequence, browser provenance, outcome matrix, artifact graph,
  hashes, focus geometry, and overlay metadata before publication.
- A deterministic checkpoint failure becomes an `untriaged/not_assessed`
  finding. Model judgment, impact claims, autonomous exploration, and repair
  claims are outside this version.
- A guide may contain only passed capture steps with authored instructions and
  a verified raw/focused/metadata evidence chain. Its focused images are copied
  into the atomic render snapshot so Markdown never depends on mutable external
  paths.
- Render output is staged and atomically renamed. A byte-identical output is an
  idempotent success; any conflicting existing output is preserved and rejected.
- Local hashes establish consistency, not authenticity. Concurrent malicious
  path replacement and externally anchored signatures remain outside the v1
  threat model and are documented explicitly.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --all-targets --all-features --locked`: 33 unit tests
  and 30 CLI integration tests passed; the explicitly opt-in real
  `agent-browser` loopback test remained ignored in the portable suite.
- `cargo build --release --bins --locked`
- `git diff --check`
- The release-blocking privacy scrub preserved the current `main` tree while
  removing private project identifiers from mutable repository history, commit
  messages, and paths before this branch was published. Provider-owned refs for
  closed reviews require separate host-side garbage collection.
- Renderer integration coverage includes pass, deterministic failure, blocked,
  error, cleanup failure, aliases, moved archives, idempotency, conflicting
  output, source drift, missing/tampered/escaping/symlinked artifacts,
  contradictory reports, explicit image association, Markdown escaping, schema
  conformance, and focus geometry anchored to decoded, digest-verified PNG
  dimensions.

## Delivered result

Crawlson 0.3.0 adds equivalent `crawlson render` and `clson render` commands.
Clean passing evidence produces a self-contained Markdown guide with verified
focused images. Deterministic failed checkpoints produce structured JSON and
Markdown findings with typed reproduction actions and verified report, trace,
and explicitly associated screenshot evidence. Blocked and invalid runs remain
honestly non-publishable. Published journey-v2, render-report-v1, and
findings-v1 schemas and architecture documents define the contracts.

The remaining first-MVP work is a self-contained local demo, clean-install and
real-browser end-to-end CI, failure-artifact upload examples, installers, and a
signed first public release. Authentication, mutation, model judgment, and
hosted publication remain intentionally out of scope.
