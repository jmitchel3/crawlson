# Build verifiable release bundles and managed installers

- Kanbanlan: `KBL-SFGWX4DKQBH7DNFFAFQVMEHK24`
- Canonical home: `github`
- Canonical request: [#16](https://github.com/jmitchel3/crawlson/issues/16)

## Request

Outcome: make Crawlson release-ready without creating or storing a production
signing secret. Build reproducible target-specific bundles containing crawlson,
clson, and the demo; define and generate the signed update manifest from those
exact artifacts; add a first-party managed installation path that writes the
receipt already required by the updater; verify install, alias parity, upgrade
ownership, rollback/failure behavior, and clean demo startup from packaged
artifacts; add a release dry-run workflow that uploads packages but cannot
publish. Production license selection, namespace reservation, production
Minisign key generation/custody, public GitHub release creation, and external
publication remain owner-gated follow-up work.

## Decisions

- Version 0.5.0 defines exactly four release targets:
  `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-pc-windows-msvc`, and `x86_64-unknown-linux-gnu`.
- Each deterministic archive contains `crawlson`, `clson`, `crawlson-demo`, the
  demo script and journey fixtures, and a target-specific payload manifest.
- Distribution and self-update use separate signed documents. The release
  inventory binds each complete bundle; updater manifest v1 lists only raw
  `crawlson` payloads. Each raw payload's size and digest must equal the
  corresponding bundle's `bin/crawlson` member.
- `crawlson install --from-bundle ROOT --prefix ABSOLUTE_BIN_DIR` is the public
  first-party installation boundary. It validates the extracted bundle,
  installs only `crawlson` and `clson`, records exact updater ownership, and
  rolls back the binaries and receipt together on failure. The demo remains in
  the extracted bundle.
- Managed Unix upgrades retain verified same-directory atomic replacement.
  Direct Windows replacement remains fail-closed; Windows users install a new
  version by rerunning the new bundle's installer.
- Release dry runs upload short-lived CI artifacts only, have no publishing
  permission or production secret, and use explicitly test-only signing keys.
  Dry-run output cannot be promoted as a production release.
- The protected default branch requires both aggregate `CI` and
  `Release dry run` checks, so auto-merge cannot bypass the four-target package,
  installer, and signing proof.
- License selection, namespace reservation, production Minisign key generation
  and custody, public GitHub release creation, and external publication require
  an explicit owner decision after the dry-run proof passes.

## Verification

- `cargo fmt --all -- --check` and locked workspace Clippy with warnings denied
  passed on Rust 1.92.0.
- `cargo test --locked --workspace --all-targets --all-features` passed: 37
  core unit tests, 2 demo tests, 30 CLI tests, 10 installer tests, 8 release-tool
  tests, and 2 release-tool CLI tests. The explicitly selected real-browser test
  remains ignored in the portable suite by contract.
- The argument-level packaged-demo regression test, Bash syntax checks, JSON and
  workflow YAML parsing, `git diff --check`, and the repository privacy scan
  passed.
- Two native Apple Silicon packages generated from unchanged final release
  binaries were byte-identical. The final archive SHA-256 was
  `18366833678b9b7524aed78fa38f550ceaa0ec473c861dca625e8e5c5b65a55a`;
  its raw updater payload was
  `850b0e34c46f54655abd48219fff6b572d382a9f3114bc39f968884f5dc3bd18`
  and was byte-identical to bundled `bin/crawlson`.
- The final extracted bundle installed `crawlson` and `clson` into a clean
  managed prefix, wrote the exact ownership receipt, and reported matching
  0.5.0 version/target data from the installed alias.
- The final extracted bundle's documented demo passed through real
  `agent-browser 0.26.0` using default executable discovery. A separate
  package-backed integration run decoded raw and focused PNGs and verified the
  exact vivid-red outline, unchanged action interior, deterministic dim-mask
  pixels, guide image identity, passing guide, intentional finding, and
  pre-browser authorization block.
- PR #17 is merge-gated by hosted `CI` and `Release dry run` aggregates. The
  delivered head must pass ordinary Linux, macOS, and Windows CI; the packaged
  real-browser journey and exact focus-pixel verifier; native package, extract,
  managed-install, alias, receipt, demo-startup, and owned-shutdown checks on
  all four targets; exact-matrix reassembly; ephemeral test signing; and
  signature verification. Failed intermediate Windows runs exposed and then
  regression-tested verbatim/extended path traversal, staged-file flushing,
  and ZIP extraction behavior rather than being rerun without diagnosis.
- GitHub API readback confirmed repository auto-merge and branch deletion are
  enabled, squash is the only merge method, workflow tokens default to read
  only and cannot approve pull requests, and live ruleset `19799701` requires
  the exact `CI` and `Release dry run` contexts with no bypass actors.

## Delivered result

- Shipped the versioned 0.5.0 release contract, deterministic four-target
  bundles, raw updater assets, signed inventory/update schemas, private release
  assembly tool, and non-publishing test-key workflow.
- Added `crawlson install` and equivalent `clson install` behavior with exact
  bundle validation, managed updater ownership, package-manager guardrails,
  concurrent update locking, observed-error rollback, and explicit crash-gap
  documentation.
- Proved the documented guide/finding/blocked loop from packaged Rust binaries
  through real `agent-browser`, including exact vivid-red action outlines and
  deterministic dimmed context.
- Public license selection, namespace reservation, durable crash recovery,
  production key generation/custody, public-key embedding, and public release
  authorization remain explicit owner-gated follow-up work.
