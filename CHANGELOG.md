# Changelog

All notable Crawlson changes will be recorded here. The project uses semantic
versioning; before 1.0, compatible fixes increment the patch version and new or
breaking product behavior increments the minor version.

## [Unreleased]

## [0.4.0] - 2026-07-27

### Added

- A loopback-only, credential-free `crawlson-demo` application with stable,
  accessible visible UI and read-only HTTP routes.
- Passing and intentionally failing journey v2 examples that drive the same
  real browser, evidence, finding, and guide pipeline used for external targets.
- `scripts/demo.sh`, a one-command complete demonstration that verifies passed,
  failed, and preflight-blocked outcomes without overwriting prior evidence.
- A required Linux CI gate for `agent-browser 0.26.0`, including raw and focused
  screenshot pixel checks, trace and digest validation, guide links, findings,
  graceful demo shutdown, and always-uploaded diagnostic artifacts.
- Documentation for the demo's loopback safety boundary, inspectable artifacts,
  portable-test opt-out, and explicit real-browser test invocation.

## [0.3.0] - 2026-07-27

### Added

- Offline `crawlson render` and equivalent `clson render` over completed run
  evidence, with strict journey provenance and full artifact re-verification.
- Deterministic Markdown guides built only from passed capture steps with
  verified focused images and authored guide instructions.
- Versioned deterministic JSON and Markdown findings for failed URL and visible
  text checkpoints, including untriaged severity, executed reproduction steps,
  report/trace evidence, and explicitly associated focused screenshots.
- Backward-compatible journey v2, adding optional `evidence_for` links from
  capture steps to unique earlier checkpoints, preventing screenshot provenance
  from being inferred by timing.
- Atomic, idempotent renderer-owned output and explicit blocked, error,
  incomplete, drift, tamper, missing-artifact, and path-escape results.
- Published render-report and findings JSON Schemas plus pass/fail/blocked,
  alias-parity, determinism, move, drift, tamper, and symlink safety coverage.

## [0.2.0] - 2026-07-27

### Added

- Strict, versioned TOML journey and JSON report contracts for deterministic
  read-only browser sessions.
- `crawlson run JOURNEY` and equivalent `clson run` with explicit exact-origin
  authorization and stable passed/failed/blocked/error exits.
- Replaceable `agent-browser 0.26.x` process adapter with owned sessions,
  default-deny action policy, bounded structured output, action deadlines,
  an overall run deadline, daemon reaping, trace finalization, diagnostics
  summaries, and visible cleanup.
- Run directories containing ordered step evidence, provenance, artifact
  digests, raw viewport screenshots, and browser traces.
- Deterministic offline focused screenshots with a vivid red target outline,
  translucent near-black surrounding mask, preserved raw PNG, and reproducible
  sidecar metadata.
- Fake-process, schema, renderer, and real agent-browser loopback coverage for
  pass, deterministic failure, visibility, missing authorization/authentication,
  cross-origin responses, malformed/contradictory/oversized protocol output,
  evidence faults, timeouts, cleanup failure, and focused-image rendering.

## [0.1.0] - 2026-07-27

### Added

- Rust CLI foundation at version 0.1.0.
- Canonical `crawlson` executable and `clson` forwarding launcher.
- Human and JSON `version`, `doctor`, and `upgrade` workflows.
- Strict `agent-browser >=0.26.0,<0.27.0` availability diagnostics.
- Signed-manifest, managed-install upgrade boundary with atomic Unix
  replacement and fail-closed Windows installer fallback.
- Weekly, jittered automatic update policy for first-party managed installs.
- Cross-platform Rust CI with one stable `CI` aggregate check.
