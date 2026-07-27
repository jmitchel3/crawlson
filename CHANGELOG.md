# Changelog

All notable Crawlson changes will be recorded here. The project uses semantic
versioning; before 1.0, compatible fixes increment the patch version and new or
breaking product behavior increments the minor version.

## [Unreleased]

## [0.7.0] - 2026-07-27

### Added

- Strict guide-collection manifest, application document, and collection-report
  v1 contracts for composing verified runs into deterministic Markdown wikis.
- A neutral ordered guide-step model with audience/topic context and explicit
  page, image, journey, run, report, manifest, and snapshot digest bindings.
- Equivalent `crawlson guides build/check` and `clson guides build/check`
  workflows with root/topic navigation, byte-identical focused images, separate
  findings review output, stable status/exit semantics, and no browser launch.
- A read-only collection audit for stale or changed files, dead links, orphaned
  images, missing index reachability, unexpected files, and symlinks.
- Complete demo coverage that builds and checks a two-guide public collection
  and a separate two-finding review collection from real browser runs.

### Security

- Collection generation ignores prior run render output, snapshots bounded raw
  inputs, and reuses the strict offline renderer for journey, authorization,
  cleanup, artifact, focus-sidecar, and provenance validation.
- Collection-wide resource budgets are enforced before byte retention or
  staging, portable paths reject cross-platform aliases, and supported systems
  commit new trees with a kernel-enforced no-replace rename.
- Public guide output is all-or-nothing. A failed, blocked, unavailable,
  incomplete, or tampered current entry cannot leave a partial collection that
  claims to be publishable, and conflicting output is never overwritten.

## [0.6.0] - 2026-07-27

### Added

- Journey schema v3 and run-report schema v2 for explicitly authorized,
  deterministic same-origin link actions with pre-action focused evidence and
  exact post-action URL verification.
- Per-step `--allow-action JOURNEY@REVISION:STEP` grants, a minimal dynamic
  `agent-browser` action policy, and honest unattempted, acknowledged, verified,
  and unknown action states.
- A two-page loopback demo proving that a generated guide can describe a link
  action Crawlson actually executed and verified.
- Findings that distinguish invisible, disabled, invalid, mismatched-href, and
  acknowledged wrong-destination link failures.

### Security

- Generic click, form input, authentication, script execution, uploads, and
  arbitrary mutation remain unavailable. Link actions are preflighted for a
  visible, enabled target and an exact credential-free same-origin destination,
  dispatched once through an anchor-and-exact-href-constrained selector, and
  never retried after an uncertain result. Malformed grants are rejected without
  retaining or echoing their contents.

## [0.5.1] - 2026-07-27

### Fixed

- Windows managed installations now treat automatic update policy as
  notify-only, preserving the signed release notice on the normal success
  cadence without downloading a raw executable that cannot safely replace the
  running process.
- Manual Windows upgrades now return a typed blocked result with the immutable
  release URL and authenticated bundle-installer guidance before any updater
  payload download.

## [0.5.0] - 2026-07-27

### Added

- A versioned release contract for deterministic target-specific bundles on
  Apple Silicon macOS, Intel macOS, x86-64 Windows, and x86-64 GNU/Linux.
- Bundles containing `crawlson`, `clson`, `crawlson-demo`, and the complete
  credential-free demo fixtures, with a manifest that binds every payload by
  path, size, and SHA-256 digest.
- Separate signed release inventory and updater manifests: the inventory binds
  complete bundles, while updater v1 contains only raw `crawlson` payloads that
  must be byte-identical to each bundle's `bin/crawlson` member.
- `crawlson install --from-bundle ROOT --prefix ABSOLUTE_BIN_DIR` for validated
  first-party installation of the canonical CLI and alias, including managed
  ownership receipt and rollback while leaving the demo bundle-local.
- Packaged-demo binary overrides, required CI coverage for the complete
  red-box/dimmed-screenshot journey, and a non-publishing release dry-run that
  tests every bundle's installation, update ownership, failure rollback, and
  demo-application HTTP startup.

### Security

- Release dry runs are read-only with respect to repository publication, use
  test-only signing material, and cannot produce promotable release assets.
- Unix self-upgrade remains a verified same-directory atomic replacement;
  Windows self-upgrade remains fail-closed and directs users back to the bundle
  installer.
- Public release creation remains blocked on owner-selected licensing,
  namespace reservation, and production Minisign key generation and custody.

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
