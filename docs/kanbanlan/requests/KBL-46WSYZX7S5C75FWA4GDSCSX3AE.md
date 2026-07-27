# Run safe read-only journeys with focused evidence

- Kanbanlan: `KBL-46WSYZX7S5C75FWA4GDSCSX3AE`
- Canonical home: `github`
- Canonical request: [#10](https://github.com/jmitchel3/crawlson/issues/10)

## Request

## Outcome

Deliver the first independently useful browser vertical slice as Crawlson 0.2.0: validate and run one read-only journey through agent-browser, preserve raw evidence, render focused action screenshots, and report passed, failed, blocked, or error honestly.

## Acceptance criteria

- Define a versioned application-independent journey format with identity, purpose, exact authorized origin, ordered read-only steps, checkpoints, and evidence requests.
- Add crawlson run JOURNEY with human and JSON output plus stable exit behavior.
- Validate every commanded and observed URL by exact scheme, hostname, and effective port before continuing; fail closed before browser launch when policy is invalid.
- Implement agent-browser >=0.26.0,<0.27.0 through the documented process boundary using one isolated session, shell-free argv, bounded JSON/stderr, deadlines, trace finalization, and owned-session cleanup.
- Preserve execution order, timing, raw screenshots, target bounding boxes, trace, partial evidence, cleanup status, and artifact digests beneath one run directory.
- Classify deterministic checkpoint false as failed, missing authorization/preconditions as blocked, protocol/infrastructure faults as error, and never let cleanup failure become passed.
- Render each requested action image offline from the raw screenshot with a red target outline and translucent near-black surrounding mask; preserve reproducible overlay metadata.
- Add fake-driver contract tests and local fixture tests for pass, fail, blocked, error, redirect/origin denial, malformed/oversized JSON, timeout, cleanup failure, and screenshot rendering.
- Update README, TODO, changelog, schemas/examples, and the durable delivery record without private project identifiers.

## Scope boundaries

Read-only deterministic journeys only. No authentication, mutation, autonomous model exploration, Markdown guide generation, hosted service, or production target execution.

## Decisions

- Ship the independently useful slice as Crawlson 0.2.0 in Rust. Keep
  `agent-browser >=0.26.0,<0.27.0` behind a typed process boundary instead of
  adopting its package/runtime architecture.
- Make journey v1 strict, deterministic, and read-only. A valid document needs
  an exact root origin, at least one observable checkpoint, and at least one
  focused capture; it exposes no generic click, input, script, authentication,
  or mutation capability.
- Treat the separately supplied `--allow-origin` value as the authorization
  source of truth. Check commanded and observed scheme/host/effective-port
  origins in Crawlson; agent-browser's hostname filter is defense in depth.
- Use an owned session, scrubbed environment, revalidated default-deny policy,
  per-command/overall deadlines, bounded cleanup grace, and daemon idle reaping.
  Graceful operating-system signal reporting remains a visible follow-up.
- Preserve raw PNGs as authoritative evidence. Bind visibility, bounding box,
  screenshot, viewport, and command order into capture provenance, then render
  the red outline and near-black mask offline with pinned PNG settings.
- Publish separate journey-v1 and run-report-v1 schemas. Preserve the primary
  execution outcome/reason when trace, diagnostics, or cleanup later fails.
  Store only the journey filename and redacted URLs in report provenance.

## Verification

- `cargo test --all-targets`: 31 library tests and 17 CLI contract tests passed;
  the opt-in real-browser test was discovered and intentionally skipped in the
  default portable suite.
- `cargo test --test real_agent_browser -- --ignored --nocapture`: passed with
  installed `agent-browser 0.26.0` against an exact, ephemeral loopback origin,
  including visible text, focused capture, trace, diagnostics, and owned close.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo build --release --bins`: passed for both `crawlson` and `clson`.
- The CLI suite validates pass, failed, blocked, error, and preflight reports
  against the published report schema; both JSON schemas also parse cleanly.
- `git diff --check` passed. The source-system inventory was generalized to
  remove exact private fixture identities/details, and a case-insensitive
  repository/history scan found no private project name, domain, or checkout
  path.

## Delivered result

Crawlson 0.2.0 can now validate and run an explicitly authorized read-only UI
journey through the supported agent-browser boundary, produce honest stable
exits and versioned reports, and retain ordered trace, diagnostic, raw-image,
focused-image, metadata, timing, digest, and cleanup evidence. The focused
derivative uses the requested red action outline and translucent near-black
surrounding mask without changing the raw PNG.

Authentication execution, mutations, autonomous agent judgment, findings,
Markdown guide generation, graceful signal handling, request-failure capture,
hosted execution, and production targets remain outside this request and stay
visible in `TODO.md`.
