# Build Rust CLI, upgrade policy, and CI foundation

- Kanbanlan: `KBL-FGBANK2BUJAEDGW75P2C6TBIHY`
- Canonical home: `github`
- Canonical request: [#6](https://github.com/jmitchel3/crawlson/issues/6)

## Request

## Outcome

Ship the independently useful Crawlson 0.1.0 command foundation in Rust, callable as crawlson or clson, with stable version/doctor behavior, a safe manual upgrade path, semi-regular automatic update policy, and CI suitable for required auto-merge checks.

## Acceptance criteria

- [x] Create the Rust package with Cargo.toml as the only product-version source, starting at 0.1.0.
- [x] Provide crawlson as the canonical binary and clson as an equivalent launcher; both accept the same arguments and report the same version.
- [x] Add human and JSON-safe doctor output that probes agent-browser availability and the accepted >=0.26.0,<0.27.0 range without installing or upgrading it.
- [x] Add crawlson upgrade and clson upgrade with an injectable update backend, explicit check mode, downgrade/prerelease refusal, and managed-install guardrails.
- [x] Add a semi-regular update policy with persisted cadence and jitter, opt-out/offline/CI detection, no foreground network wait, no JSON stdout contamination, and no journey or host telemetry.
- [x] Add tests for aliases, version parity, updater decisions, cadence, opt-outs, failures, and exit-code preservation.
- [x] Add a stable CI check and update README, TODO, changelog, and the durable request record.

## Scope boundaries

In scope: executable foundation, version policy, update behavior and test seams, CI. Out of scope: browser journey schema/execution, guide generation, release publication, package-manager publication, and main ruleset activation until CI has passed on main.

## Decisions

- `Cargo.toml` is the only product version source. The first development version
  is 0.1.0; fixes increment patch, while features and pre-1.0 breaking behavior
  increment minor. Documentation and CI-only changes do not bump the version.
- `crawlson` is the only self-replacing executable. `clson` is a small sibling
  launcher, avoiding two independently updatable copies.
- The updater trusts only an exact first-party managed-install receipt, an
  immutable stable release from the canonical repository, GitHub asset digests,
  and a Minisign-signed manifest. It fails closed until a public key is embedded.
- Direct binary replacement is enabled only on Unix, where the selected helper
  uses same-directory rename semantics. Windows fails closed and delegates to
  the installer until a tested rollback path exists.
- Package-manager detection precedes receipt acceptance. Invalid explicit
  update policy or config disables periodic updates instead of falling back to
  automatic mutation.
- First-party managed installs default to automatic compatible upgrades on a
  weekly cadence with up to 48 hours of deterministic per-install jitter.
  Package-managed and unknown installs are never overwritten.
- Background work is a detached hidden command with null standard streams.
  Foreground output and exit status are computed first and never await network
  or worker completion.
- The stable required-check candidate is an aggregate job named `CI`; it depends
  on Linux, macOS, and Windows format, lint, test, build, and alias smoke jobs.

## Verification

- `cargo test --workspace --all-targets --all-features` (25 tests)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo fmt --all --check`
- Manual human and JSON smoke tests for `crawlson`, `clson`, `doctor`, and
  offline `upgrade`.
- Privacy search for the private project name, path variants, and identifiers.

## Delivered result

- Added the Rust 0.1.0 package, canonical and alias commands, strict doctor,
  injectable signed updater, periodic policy, tests, and cross-platform CI.
- Documented current commands, output/exit contracts, update trust and privacy
  boundaries, version policy, and opt-outs.
- Follow-up remains intentionally separate: embed the public update key, publish
  signed immutable release assets/installers, activate the main ruleset after
  CI passes on main, prove Windows replacement rollback before enabling direct
  updates there, and implement browser journeys and guide generation.
