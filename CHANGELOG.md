# Changelog

All notable Crawlson changes will be recorded here. The project uses semantic
versioning; before 1.0, compatible fixes increment the patch version and new or
breaking product behavior increments the minor version.

## [Unreleased]

### Added

- Rust CLI foundation at version 0.1.0.
- Canonical `crawlson` executable and `clson` forwarding launcher.
- Human and JSON `version`, `doctor`, and `upgrade` workflows.
- Strict `agent-browser >=0.26.0,<0.27.0` availability diagnostics.
- Signed-manifest, managed-install upgrade boundary with atomic Unix
  replacement and fail-closed Windows installer fallback.
- Weekly, jittered automatic update policy for first-party managed installs.
- Cross-platform Rust CI with one stable `CI` aggregate check.
