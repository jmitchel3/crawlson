# Make Windows upgrades notify-only before payload download

- Kanbanlan: `KBL-I7A6OQE7F5BBLHWLMYEW6AZDUA`
- Canonical home: `github`
- Canonical request: [#20](https://github.com/jmitchel3/crawlson/issues/20)

## Request

Outcome: ship a patch release where Windows managed installs never download a raw updater payload they cannot safely self-replace. Acceptance: default and explicitly requested automatic mode resolve to notify-only on Windows; periodic checks persist the available version and success cadence without invoking installation; manual upgrade reports a typed blocked result with immutable release URL and validated bundle-installer guidance before backend installation/download; check-only remains successful; Unix auto-upgrade behavior is unchanged; cross-platform injected tests prove the policy; README, TODO, changelog, and version are updated when required.

## Decisions

- The patch version is 0.5.1 because this corrects shipped 0.5.0 update policy
  without adding a new journey or release contract.
- Raw self-replacement is a compiled platform capability: supported Unix builds
  retain `auto`; Windows and other unsupported builds reduce `auto` to
  `notify`. Explicit `notify` and `off` remain unchanged.
- Metadata checking remains available on Windows so Crawlson can authenticate
  and persist the candidate and immutable release URL. Manual non-check
  upgrades block after metadata and ownership validation but before the backend
  install call, which owns every raw payload download.
- Periodic ownership is still sampled only after acquiring the update lock, and
  configured mode is still evaluated after the metadata request immediately
  before installation. This preserves concurrent-install exclusion and a Unix
  user's ability to switch from `auto` to `off` while a check is in flight.
- Windows replacement continues through an authenticated extracted bundle and
  the existing managed installer; Crawlson does not weaken rollback or execute
  an in-place self-replacement.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --locked -p crawlson update::tests -- --nocapture`: 22 focused
  update tests passed on Unix, including policy re-evaluation after metadata,
  notify-only state persistence, and retained automatic installation.
- `CRAWLSON_OFFLINE=1 cargo test --workspace --all-targets --all-features
  --locked`: 94 tests passed; the explicitly opt-in real-browser test remained
  ignored in this portable suite.
- `bash tests/test_demo_script.sh`
- Independent post-fix review found no implementation or documentation blocker
  after ownership-under-lock and post-metadata policy ordering were restored.
- Hosted Windows CI remains required to execute the compiled `cfg(windows)`
  manual and periodic no-download tests before merge.

## Delivered result

Crawlson 0.5.1 makes managed Windows update checks durable and notify-only,
including when configuration requests `auto`. Manual Windows upgrade reports a
typed block with release and bundle-installer guidance before the raw installer
can be invoked. Supported Unix automatic upgrades retain their compatible
replacement path. Public signing and release publication remain separately
owner-gated by #18.
